//! Embedded single-page web client.
//!
//! The client is compiled into the binary so a deployment stays one file with no
//! Node runtime beside it. This module is the router fallback: anything that no
//! API route claimed is either a built asset or a client-side route, and a
//! client-side route resolves by returning the shell.
//!
//! Every response here carries the web client's security headers, a
//! Content-Security-Policy among them ([`crate::http::security_headers`]).

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use rust_embed::Embed;
use std::sync::OnceLock;

use crate::api::ErrorResponse;

#[derive(Embed)]
// Staged by build.rs: the real client build when present, a placeholder
// otherwise. Never the source tree, which a build script must not write to.
#[folder = "$OUT_DIR/webapp-dist/"]
struct Assets;

const INDEX: &str = "index.html";

/// Namespaces that belong to the server rather than the client. An unknown path
/// under one of them is a genuine 404: returning the SPA shell there would
/// answer a mistyped API call with `200 text/html`, which turns an obvious
/// client bug into a confusing parse error.
const RESERVED_PREFIXES: [&str; 3] = ["api/", "rest/", "share/"];

/// Single endpoints, matched whole. Comparing these by prefix would swallow
/// client routes that merely start the same way — `/reference-guide` is a
/// legitimate page, not a mistyped `/reference`.
const RESERVED_EXACT: [&str; 4] = ["health", "ready", "openapi.json", "reference"];

/// Files the installed client is made of (RFC-022 decision 4), answered as
/// themselves or not at all.
///
/// Unlike [`RESERVED_EXACT`], these are looked up among the assets first: they
/// are the client's, and a real build ships them. What they must never get is
/// the fallback. A browser reading the manifest expects JSON and an icon an
/// image; handed the shell, it fails to parse it and the client is not
/// installable, with nothing in the response to say why. A build without them
/// — the placeholder `build.rs` embeds — answers a JSON 404 instead.
///
/// `sw.js`, the service worker (RFC-022, offline listening), for the same
/// reason: a browser registering a worker and handed the shell refuses it as
/// a script of the wrong type, and the client silently loses the offline copy.
///
/// `favicon.ico`, the logo kit's icon for browsers that read no SVG, is the
/// case that taught the rule. Before a build shipped one, the fallback handed
/// the browser the shell for it; a browser that asks for an icon and receives
/// `200 text/html` cannot decode it, learns nothing, and asks again — sixty-six
/// times in fifty-one seconds on a real session, forty kilobytes of HTML spent
/// on a tab icon. Served from the build when it is there, a JSON 404 when it
/// is not: either is a definitive answer, asked once.
///
/// `favicon.svg` and the logo kit's files under `brand/` for the same reason
/// as the `.ico`: an image, which the shell can never stand in for.
const ASSET_ONLY_EXACT: [&str; 5] = [
    "manifest.webmanifest",
    "apple-touch-icon.png",
    "favicon.ico",
    "favicon.svg",
    "sw.js",
];
const ASSET_ONLY_PREFIXES: [&str; 2] = ["icons/", "brand/"];

pub async fn handler(uri: Uri) -> Response {
    let mut response = respond(uri.path().trim_start_matches('/'));
    crate::http::security_headers::apply_web_client(response.headers_mut(), policy());
    response
}

/// The Content-Security-Policy every response here carries, built once from
/// the shell this binary embeds, so its inline blocks are admitted by the
/// digests of exactly the bytes that are served.
fn policy() -> &'static HeaderValue {
    static POLICY: OnceLock<HeaderValue> = OnceLock::new();
    POLICY.get_or_init(|| {
        HeaderValue::from_str(&crate::http::security_headers::web_client_policy(
            &embedded_shell(),
        ))
        // A digest is base64 and every directive is ASCII, so this cannot
        // fail; if it ever did, the strictest policy is the safe answer.
        .unwrap_or(HeaderValue::from_static("default-src 'none'"))
    })
}

fn respond(path: &str) -> Response {
    let reserved = RESERVED_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix))
        || RESERVED_EXACT.contains(&path);
    if reserved {
        return not_found();
    }
    if let Some(response) = asset(path) {
        return response;
    }
    let asset_only = ASSET_ONLY_EXACT.contains(&path)
        || ASSET_ONLY_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix));
    if asset_only {
        return not_found();
    }
    shell()
}

/// The client's shell, for a route the client renders.
fn shell() -> Response {
    asset(INDEX).unwrap_or_else(missing_client)
}

/// The shell as `/share/{token}` hands it to a browser (RFC-020 decision 3).
///
/// `share/` stays reserved above, so nothing under it falls through to the
/// client by accident, and the one address that does is chosen by its own
/// route. It carries the client's policy, which the shell's inline blocks need
/// to run, with framing refused as on every share response
/// ([`crate::http::security_headers::share_page_policy`]); the share route's own
/// layer adds the rest of the share headers.
pub fn share_page() -> Response {
    static POLICY: OnceLock<HeaderValue> = OnceLock::new();
    let policy = POLICY.get_or_init(|| {
        HeaderValue::from_str(&crate::http::security_headers::share_page_policy(
            &embedded_shell(),
        ))
        .unwrap_or(HeaderValue::from_static("default-src 'none'"))
    });
    let mut response = shell();
    response
        .headers_mut()
        .insert(header::CONTENT_SECURITY_POLICY, policy.clone());
    response
}

/// The shell this binary embeds, as text, for the digests of its inline blocks.
fn embedded_shell() -> String {
    Assets::get(INDEX)
        .map(|file| String::from_utf8_lossy(&file.data).into_owned())
        .unwrap_or_default()
}

fn asset(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = file.metadata.mimetype();
    let content_type =
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream"));
    // Only files under the bundler's output directory carry a content hash, so
    // only they are safe to freeze. Everything else — the shell, favicon.svg,
    // robots.txt, a service worker — keeps a stable name across deploys, and
    // pinning those for a year would strand clients on a stale build.
    let cache_control = if path.starts_with("assets/") {
        HeaderValue::from_static("public, max-age=31536000, immutable")
    } else {
        HeaderValue::from_static("no-cache")
    };
    Some(
        (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, cache_control),
            ],
            Body::from(file.data.into_owned()),
        )
            .into_response(),
    )
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            code: "not_found",
            message: "Resource not found",
        }),
    )
        .into_response()
}

/// Only reachable if the build placeholder was deleted without a real build.
fn missing_client() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            code: "web_client_unavailable",
            message: "The web client was not built into this binary",
        }),
    )
        .into_response()
}
