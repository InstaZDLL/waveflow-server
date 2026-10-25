//! `/reference`: the API reference, served from the binary (RFC-023
//! decision 3).
//!
//! The page is [Scalar](https://github.com/scalar/scalar)'s API reference over
//! this server's own OpenAPI document. Until RFC-023 it was `utoipa-scalar`'s
//! template, which loads `https://cdn.jsdelivr.net/npm/@scalar/api-reference`
//! with no version: every visitor's browser fetched whatever that CDN called
//! the latest release, from a host the operator never chose, and ran it on this
//! server's origin. The bundle is now part of the binary, pinned and checked,
//! and the page needs no host but this one.
//!
//! **What is embedded.** `dist/browser/standalone.js` of the npm package
//! `@scalar/api-reference` at [`SCALAR_VERSION`] — the file jsDelivr resolves
//! that package to — compressed with `gzip -9n`, in
//! `assets/scalar/api-reference-<version>.js.gz`, with Scalar's MIT licence
//! beside it. The package tarball's npm integrity is
//! `sha512-YatOueurjdSAbE3K51L0NkyzdQEpksnjvzCpJTSMPaYd6VWpXMrycDfTBY0myuhszgxk+PcOehExyIXc20yXhw==`;
//! the bundle's SHA-256 once decompressed is [`SCALAR_SHA256`], which a test
//! holds the embedded bytes to.
//!
//! **To move to another version:** fetch the package, compress its
//! `dist/browser/standalone.js` the same way, replace the file, and change
//! the version, the digest and the integrity above together. The script's
//! address carries the version, so a browser that cached the old one under
//! `immutable` never mixes the two.
//!
//! **Stored compressed, served as stored.** Four megabytes of script would
//! otherwise be compressed again on every visit, or weigh four times as much
//! in the repository and the binary. Every browser that can run the page
//! accepts gzip; a client that does not is handed the bytes decompressed.
//!
//! **Under a policy of its own** ([`crate::security_headers::reference_policy`]):
//! scripts from this origin only, and no connection anywhere else.

use std::io::Read as _;

use axum::{
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

/// The `@scalar/api-reference` release embedded.
pub const SCALAR_VERSION: &str = "1.73.1";

/// SHA-256 of the embedded bundle, decompressed: the bytes npm publishes as
/// `dist/browser/standalone.js` in that release.
pub const SCALAR_SHA256: &str = "62c34a0dda38945864011ebaf6e7fa7812a332429120d18922b34d01c4f7ccc8";

/// The bundle, as `gzip -9n` left it.
static SCALAR_GZIP: &[u8] = include_bytes!("../assets/scalar/api-reference-1.73.1.js.gz");

/// Where the page loads the bundle from. Versioned, so it can be cached for
/// good.
pub const SCALAR_SCRIPT_PATH: &str = "/reference/scalar-1.73.1.js";

/// The two routes: the page, and the bundle it loads.
///
/// Merged outside the compression layer: the bundle is already compressed,
/// and the page is a few hundred kilobytes of JSON the browser reads once.
pub fn router<S: Clone + Send + Sync + 'static>(openapi: &utoipa::openapi::OpenApi) -> Router<S> {
    let page = page_template(openapi);
    Router::new()
        .route(
            crate::SCALAR_PATH,
            get(move || {
                let page = page.clone();
                async move { serve_page(page) }
            }),
        )
        .route(SCALAR_SCRIPT_PATH, get(serve_script))
}

/// The page, built once.
///
/// The document goes into a `<script type="application/json">` block, which
/// no browser executes and which Scalar reads as the specification. Every `<`
/// in it is written `\u003c`, a JSON escape that decodes to the same
/// character, so no string in the document can close the block early.
fn page_template(openapi: &utoipa::openapi::OpenApi) -> String {
    let spec = serde_json::to_string(openapi)
        .expect("the OpenAPI document serializes")
        .replace('<', "\\u003c");
    // Everything in Scalar that would reach a host of its own, switched off:
    // its typefaces (fonts.scalar.com), its telemetry, which is on by default,
    // its AI agent and MCP integration, and the developer toolbar that offers
    // to publish the document to scalar.com. The policy refuses those
    // connections anyway; this keeps the page from offering or attempting them.
    let configuration = serde_json::json!({
        "withDefaultFonts": false,
        "telemetry": false,
        "showDeveloperTools": "never",
        "agent": { "disabled": true },
        "mcp": { "disabled": true },
    })
    .to_string()
    .replace('"', "&quot;");
    format!(
        "<!doctype html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>WaveFlow Server API</title>\n\
         </head>\n\
         <body>\n\
         <script id=\"api-reference\" type=\"application/json\" data-configuration=\"{configuration}\">{spec}</script>\n\
         <script src=\"{SCALAR_SCRIPT_PATH}\"></script>\n\
         </body>\n\
         </html>\n"
    )
}

fn serve_page(page: String) -> Response {
    let mut response = ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], page).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    crate::security_headers::apply_reference(headers, &crate::security_headers::reference_policy());
    response
}

async fn serve_script(request_headers: HeaderMap) -> Response {
    let gzip = accepts_gzip(&request_headers);
    let body = if gzip {
        SCALAR_GZIP.to_vec()
    } else {
        match decompressed() {
            Ok(bytes) => bytes,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    };
    let mut response = body.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("accept-encoding"));
    if gzip {
        headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    }
    crate::security_headers::apply_reference(
        headers,
        &HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    response
}

/// Whether the request names gzip with a non-zero weight, or `*`.
fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|coding| {
            let mut parts = coding.split(';').map(str::trim);
            let name = parts.next().unwrap_or_default();
            let refused = parts.any(|parameter| {
                parameter
                    .strip_prefix("q=")
                    .and_then(|q| q.trim().parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            });
            (name.eq_ignore_ascii_case("gzip") || name == "*") && !refused
        })
}

/// The bundle as published.
pub fn decompressed() -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(SCALAR_GZIP).read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    /// The embedded bytes are the release this module names, and nothing
    /// else: a file replaced without its digest, or a digest changed without
    /// its file, fails here.
    #[test]
    fn the_embedded_bundle_is_the_pinned_release() {
        let bundle = decompressed().expect("the bundle decompresses");
        assert_eq!(hex::encode(Sha256::digest(&bundle)), SCALAR_SHA256);
        assert!(SCALAR_SCRIPT_PATH.contains(SCALAR_VERSION));
    }

    #[test]
    fn gzip_is_sent_only_to_whoever_takes_it() {
        let with = |value: &str| {
            let mut headers = HeaderMap::new();
            headers.insert(
                header::ACCEPT_ENCODING,
                HeaderValue::from_str(value).unwrap(),
            );
            accepts_gzip(&headers)
        };
        assert!(with("gzip, deflate, br, zstd"));
        assert!(with("br;q=1.0, GZIP;q=0.5"));
        assert!(with("*"));
        assert!(!with("br"));
        assert!(!with("gzip;q=0"));
        assert!(!accepts_gzip(&HeaderMap::new()));
    }

    /// No string in the document can end the block it is carried in.
    #[test]
    fn the_document_cannot_close_its_own_block() {
        let mut openapi = utoipa::openapi::OpenApi::default();
        openapi.info.description = Some("</script><script>alert(1)</script>".to_owned());
        let page = page_template(&openapi);
        assert_eq!(page.matches("</script>").count(), 2, "{page}");
        assert!(page.contains("\\u003c/script>"));
    }
}
