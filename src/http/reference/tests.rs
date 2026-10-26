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

/// A client that takes no gzip costs one decompression per process, not
/// one per request: the route is open to whoever `/reference` is open to.
/// Served twice, and three times more, the count stays at one, and both
/// answers are the published bundle.
#[tokio::test]
async fn the_bundle_is_decompressed_once_whatever_the_requests() {
    use http_body_util::BodyExt as _;
    let mut bodies = Vec::new();
    for _ in 0..5 {
        let response = serve_script(HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
        bodies.push(response.into_body().collect().await.unwrap().to_bytes());
    }
    assert_eq!(
        DECOMPRESSIONS.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the bundle must be decompressed once and kept"
    );
    assert_eq!(hex::encode(Sha256::digest(&bodies[0])), SCALAR_SHA256);
    assert!(bodies.iter().all(|body| body == &bodies[0]));
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
    // A coding the field names is decided by its own entry, and `*` only
    // stands for the ones it does not name (RFC 9110 §12.5.3).
    assert!(!with("gzip;q=0, *;q=1"));
    assert!(!with("*, gzip;q=0"));
    assert!(with("*;q=0, gzip"));
    assert!(!with("br, *;q=0"));
    assert!(with("x-gzip"));
    // Names are case-insensitive, the weight's as much as the coding's,
    // and whitespace around `;` and `=` changes nothing.
    assert!(!with("gzip;Q=0"));
    assert!(!with("GZIP ; q = 0"));
    assert!(!with("gzip;q=0.000, *"));
    assert!(with("Gzip;Q=0.001"));
    assert!(with("gzip;q=1.000"));
    // A weight outside the grammar makes its entry say nothing: the next
    // entry, or the wildcard, decides, and nothing panics.
    for malformed in ["2", "1.5", "-1", "0.0001", "abc", "", "NaN", "inf", "1e0"] {
        assert!(
            !with(&format!("gzip;q={malformed}")),
            "q={malformed} is no acceptance"
        );
        assert!(
            with(&format!("gzip;q={malformed}, *")),
            "q={malformed} leaves the wildcard deciding"
        );
        assert!(
            !with(&format!("gzip;q={malformed}, gzip;q=0")),
            "q={malformed} leaves the next gzip entry deciding"
        );
    }
    // An empty field is identity only.
    assert!(!with(""));
    assert!(!with(" , ;q=1"));
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
