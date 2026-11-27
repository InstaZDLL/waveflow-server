//! The shell as each root serves it (RFC-030 decision 3).
//!
//! The client is built with relative references (`./assets/…`), which is what
//! lets its stylesheet and its chunks find each other under any prefix. The
//! shell is the one file where that is wrong: its references resolve against
//! the page's own address, so on `/albums/<id>` `./assets/x.js` would be asked
//! for at `/albums/assets/x.js`. So every reference the shell makes is anchored
//! here to the root of the request — `/` on the main listener, the ingress
//! prefix inside Home Assistant — and the prefix is written into the shell for
//! the client to read.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
};

use axum::http::HeaderValue;

/// The shell for one root, and the policy computed on exactly those bytes.
pub(super) struct Shell {
    pub html: Arc<str>,
    pub policy: HeaderValue,
}

/// How many prefixes are kept. A Home Assistant installation has one, which
/// changes only when it is reinstalled; the bound is for a peer that sends a
/// new one every time, which costs it a rewrite each instead of memory.
const CACHED_PREFIXES: usize = 8;

/// The shell for the main listener (`root` empty) or for an ingress prefix.
pub(super) fn shell_for(embedded: &str, root: &str) -> Arc<Shell> {
    if root.is_empty() {
        static MAIN: OnceLock<Arc<Shell>> = OnceLock::new();
        return MAIN.get_or_init(|| build(embedded, root)).clone();
    }
    static PREFIXED: OnceLock<Mutex<HashMap<String, Arc<Shell>>>> = OnceLock::new();
    let cache = PREFIXED.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(shell) = cache.get(root) {
        return shell.clone();
    }
    let shell = build(embedded, root);
    if cache.len() >= CACHED_PREFIXES {
        cache.clear();
    }
    cache.insert(root.to_owned(), shell.clone());
    shell
}

fn build(embedded: &str, root: &str) -> Arc<Shell> {
    let html = rewrite(embedded, root);
    let policy = HeaderValue::from_str(&crate::http::security_headers::web_client_policy(&html))
        // A digest is base64 and every directive is ASCII, so this cannot
        // fail; if it ever did, the strictest policy is the safe answer.
        .unwrap_or(HeaderValue::from_static("default-src 'none'"));
    Arc::new(Shell {
        html: Arc::from(html),
        policy,
    })
}

/// The shell with its references anchored at `root` and the prefix written
/// into it.
///
/// `root` is empty or an [`IngressPath`](crate::http::ingress::IngressPath),
/// whose characters are checked to the byte, so it needs no escaping in an
/// attribute.
pub(super) fn rewrite(html: &str, root: &str) -> String {
    let base = format!("{root}/");
    let mut out = String::with_capacity(html.len() + 16 * root.len());
    let mut rest = html;
    while let Some(at) = next_reference(rest) {
        let (before, after) = rest.split_at(at.value);
        out.push_str(before);
        let path = if let Some(relative) = after.strip_prefix("./") {
            relative
        } else {
            // Rooted: `/x`, but never `//host`, which leaves the origin.
            &after[1..]
        };
        out.push_str(&base);
        rest = path;
    }
    out.push_str(rest);
    let out = out.replacen(
        r#"name="waveflow-base" content="""#,
        &format!(r#"name="waveflow-base" content="{root}""#),
        1,
    );
    if root.is_empty() {
        out
    } else {
        without_manifest(&out)
    }
}

/// Where the next `src="…"` or `href="…"` value to anchor begins.
struct Reference {
    value: usize,
}

fn next_reference(html: &str) -> Option<Reference> {
    let mut from = 0;
    loop {
        let found = ["src=\"", "href=\""]
            .iter()
            .filter_map(|attribute| {
                html[from..]
                    .find(attribute)
                    .map(|index| from + index + attribute.len())
            })
            .min()?;
        let value = &html[found..];
        let anchored =
            value.starts_with("./") || (value.starts_with('/') && !value.starts_with("//"));
        if anchored {
            return Some(Reference { value: found });
        }
        from = found;
    }
}

/// Under a prefix the client is not installable (RFC-030 decision 8): the
/// manifest's own paths start at `/`, which inside Home Assistant is Home
/// Assistant's, and its app is what a phone installs.
fn without_manifest(html: &str) -> String {
    let Some(start) = html.find(r#"<link rel="manifest""#) else {
        return html.to_owned();
    };
    let Some(length) = html[start..].find('>') else {
        return html.to_owned();
    };
    format!("{}{}", &html[..start], &html[start + length + 1..])
}
