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

/// The share page runs the client under the client's own policy, and is
/// framed by nobody (RFC-020 decision 3).
#[test]
fn the_share_page_is_the_client_policy_framed_by_nobody() {
    let html = "<html><head><style>p{}</style></head></html>";
    let page = share_page_policy(html);
    assert_eq!(
        page,
        web_client_policy(html).replace("frame-ancestors 'self'", "frame-ancestors 'none'")
    );
}

/// The share layer keeps the page's policy and gives every other share
/// response the framing-only one.
#[tokio::test]
async fn the_share_layer_keeps_the_page_policy_and_sets_the_rest() {
    let page_policy = HeaderValue::from_static("default-src 'self'; frame-ancestors 'none'");
    let mut page = Response::new(axum::body::Body::empty());
    page.headers_mut()
        .insert(header::CONTENT_SECURITY_POLICY, page_policy.clone());
    let page = share(page).await;
    assert_eq!(page.headers()[header::CONTENT_SECURITY_POLICY], page_policy);
    assert_eq!(page.headers()[header::REFERRER_POLICY], "no-referrer");

    let document = share(Response::new(axum::body::Body::empty())).await;
    assert_eq!(
        document.headers()[header::CONTENT_SECURITY_POLICY],
        "frame-ancestors 'none'"
    );
    assert_eq!(document.headers()[header::X_FRAME_OPTIONS], "DENY");
}
