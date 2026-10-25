//! The response headers that tell a browser how far to trust what this server
//! sends (RFC-019 decision 2).
//!
//! Two surfaces carry them, and only two:
//!
//! - **the embedded web client** ([`crate::webui`]), every response of the
//!   router fallback: the shell, its assets, and its refusals;
//! - **the public shares** under `/share/`, whose token *is* the path, so a
//!   referrer would hand it to whatever host a page linked to.
//!
//! The Subsonic façade is deliberately left alone: its observable wire
//! behaviour is frozen (RFC-002), and a header a validated client has never
//! seen is a change to it. The JSON API is left alone too — no browser renders
//! it as a document, so none of these would change what a browser does with it.

use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::Response;
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};

/// The web client's policy, for a shell whose inline blocks are in `html`.
///
/// Every directive is the narrowest the built client works under:
///
/// - **scripts and styles** from this origin, plus each inline block of the
///   shell by its SHA-256 digest. The shell carries two on purpose — the
///   splash screen's style and its ten-second fallback — and a digest admits
///   those two and nothing an attacker could inject beside them. They are
///   computed from the shell actually embedded, so editing either block can
///   never leave a stale digest behind. React sets `style` through the CSSOM,
///   which no `style-src` governs, so no `'unsafe-inline'` is needed;
/// - **images** from this origin, `data:` (the stylesheet's inline SVGs) and
///   `blob:` (artwork is fetched with the session and shown as a blob);
/// - **media** from this origin and `blob:`, and from any `http:` or `https:`
///   host: an internet radio station is played by the browser straight from
///   the station's own server, whose address the user typed;
/// - **fonts** from this origin and `data:`, since the bundler may inline a
///   small one;
/// - **connections** to this origin only, sockets included;
/// - **no plugins, no `<base>`, forms posted only here, and framed only by
///   this origin**.
pub fn web_client_policy(html: &str) -> String {
    let scripts = inline_digests(html, "script");
    let styles = inline_digests(html, "style");
    format!(
        "default-src 'self'; \
         script-src 'self'{scripts}; \
         style-src 'self'{styles}; \
         img-src 'self' data: blob:; \
         media-src 'self' blob: http: https:; \
         font-src 'self' data:; \
         connect-src 'self'; \
         worker-src 'self'; \
         object-src 'none'; \
         base-uri 'none'; \
         form-action 'self'; \
         frame-ancestors 'self'"
    )
}

/// The headers every web client response carries.
///
/// **`same-origin`, not `no-referrer`.** Both send nothing to another host,
/// which is what the policy is for here: a radio station's server, or last.fm
/// when an account is linked. They differ only on requests to this server, and there
/// `no-referrer` buys nothing and puts the origin check at risk: the Fetch
/// standard serialises `Origin` as `null` for a non-CORS `POST` made under it,
/// and the web sign-in and setup refuse every origin but their own. Chromium
/// was measured sending the real origin on the client's `fetch` calls even so;
/// `same-origin` does not depend on that.
pub fn apply_web_client(headers: &mut HeaderMap, policy: &HeaderValue) {
    headers.insert(header::CONTENT_SECURITY_POLICY, policy.clone());
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
}

/// The headers a public share response carries.
///
/// A share is JSON and audio, never a page, so it is framed by nobody and its
/// policy names framing only: a `default-src` here would stop a browser from
/// playing the stream it was opened on.
pub async fn share(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("frame-ancestors 'none'"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    response
}

/// `'sha256-…'` for each inline `<tag>` block of `html`, each preceded by a
/// space. A block with a `src` loads from a URL, which `'self'` governs, and
/// has no content to digest.
fn inline_digests(html: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut digests = String::new();
    let mut rest = html;
    while let Some(start) = rest.find(&open) {
        rest = &rest[start + open.len()..];
        // `<styles>` or `<scripts>` is not the tag.
        if !rest.starts_with(['>', ' ', '\t', '\n', '\r']) {
            continue;
        }
        let Some(end_of_tag) = rest.find('>') else {
            break;
        };
        let attributes = &rest[..end_of_tag];
        rest = &rest[end_of_tag + 1..];
        let Some(end) = rest.find(&close) else {
            break;
        };
        let content = &rest[..end];
        rest = &rest[end + close.len()..];
        if attributes.contains("src=") {
            continue;
        }
        digests.push_str(" 'sha256-");
        digests.push_str(&STANDARD.encode(Sha256::digest(content.as_bytes())));
        digests.push('\'');
    }
    digests
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(content: &str) -> String {
        format!(
            "'sha256-{}'",
            STANDARD.encode(Sha256::digest(content.as_bytes()))
        )
    }

    #[test]
    fn each_inline_block_is_admitted_by_its_digest_and_nothing_else() {
        let style = "\n  #splash { opacity: 1; }\n";
        let script = "\n  setTimeout(function () {}, 10000);\n";
        let html = format!(
            "<html><head><style>{style}</style>\
             <script type=\"module\" crossorigin src=\"/assets/index.js\"></script>\
             <link rel=\"stylesheet\" href=\"/assets/index.css\"></head>\
             <body><scripts>not a tag</scripts><script>{script}</script></body></html>"
        );
        let policy = web_client_policy(&html);
        assert!(
            policy.contains(&format!("script-src 'self' {};", digest(script))),
            "{policy}"
        );
        assert!(
            policy.contains(&format!("style-src 'self' {};", digest(style))),
            "{policy}"
        );
        // The bundle's own tag has a `src` and is governed by `'self'`.
        assert_eq!(policy.matches("'sha256-").count(), 2, "{policy}");
        assert!(!policy.contains("unsafe-inline"), "{policy}");
        assert!(!policy.contains("unsafe-eval"), "{policy}");
    }

    #[test]
    fn a_shell_without_inline_blocks_admits_this_origin_only() {
        let policy = web_client_policy("<html><body><p>placeholder</p></body></html>");
        assert!(policy.contains("script-src 'self';"), "{policy}");
        assert!(policy.contains("style-src 'self';"), "{policy}");
        assert!(policy.ends_with("frame-ancestors 'self'"), "{policy}");
    }
}
