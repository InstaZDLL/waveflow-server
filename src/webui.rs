//! Embedded single-page web client.
//!
//! The client is compiled into the binary so a deployment stays one file with no
//! Node runtime beside it. This module is the router fallback: anything that no
//! API route claimed is either a built asset or a client-side route, and a
//! client-side route resolves by returning the shell.

use axum::{
    body::Body,
    http::{header, HeaderValue, StatusCode, Uri},
    response::{IntoResponse, Response},
    Json,
};
use rust_embed::Embed;

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

pub async fn handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
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
    asset(INDEX).unwrap_or_else(missing_client)
}

fn asset(path: &str) -> Option<Response> {
    let file = Assets::get(path)?;
    let mime = file.metadata.mimetype();
    let content_type =
        HeaderValue::from_str(mime).unwrap_or(HeaderValue::from_static("application/octet-stream"));
    // Only files under the bundler's output directory carry a content hash, so
    // only they are safe to freeze. Everything else — the shell, favicon.ico,
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
