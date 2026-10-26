use super::*;

fn accept(value: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::ACCEPT, HeaderValue::from_str(value).unwrap());
    headers
}

#[test]
fn only_an_explicit_html_request_is_handed_the_page() {
    for page in [
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        "TEXT/HTML",
        "application/json;q=0.5, text/html",
    ] {
        assert!(wants_page(&accept(page)), "{page}");
    }
    for document in [
        "*/*",
        "application/json",
        "text/*",
        "text/html;q=0",
        "text/htmlx",
    ] {
        assert!(!wants_page(&accept(document)), "{document}");
    }
    assert!(!wants_page(&HeaderMap::new()));
}
