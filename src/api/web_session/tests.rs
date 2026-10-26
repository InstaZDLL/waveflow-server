use super::public_url_is_https;

#[test]
fn secure_cookie_detection_uses_the_parsed_url_scheme() {
    assert!(public_url_is_https(Some("HTTPS://waveflow.test/")));
    assert!(!public_url_is_https(Some("http://waveflow.test")));
    assert!(!public_url_is_https(Some("not a URL")));
    assert!(!public_url_is_https(None));
}
