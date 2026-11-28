use super::shell::rewrite;

/// The head of a build made with `base: "./"`, as Vite writes it.
const BUILT: &str = r#"<head>
<meta name="waveflow-base" content="" />
<link rel="icon" href="./favicon.svg" type="image/svg+xml" />
<link rel="manifest" href="./manifest.webmanifest" />
<script type="module" crossorigin src="./assets/index-abc.js"></script>
<link rel="stylesheet" crossorigin href="./assets/index-def.css">
</head><body><img src="/brand/logo.svg"><a href="//elsewhere.example/x">x</a>
<a href="https://example.org/">y</a></body>"#;

#[test]
fn the_main_listener_anchors_every_reference_at_the_root() {
    let html = rewrite(BUILT, "");
    assert!(html.contains(r#"href="/favicon.svg""#), "{html}");
    assert!(html.contains(r#"src="/assets/index-abc.js""#), "{html}");
    assert!(html.contains(r#"href="/assets/index-def.css""#), "{html}");
    assert!(html.contains(r#"src="/brand/logo.svg""#), "{html}");
    // Installable here, and the prefix is empty.
    assert!(html.contains(r#"href="/manifest.webmanifest""#), "{html}");
    assert!(
        html.contains(r#"name="waveflow-base" content="""#),
        "{html}"
    );
    assert!(!html.contains("\"./"), "{html}");
}

#[test]
fn an_ingress_prefix_anchors_every_reference_under_it() {
    let prefix = "/api/hassio_ingress/tok";
    let html = rewrite(BUILT, prefix);
    assert!(
        html.contains(r#"src="/api/hassio_ingress/tok/assets/index-abc.js""#),
        "{html}"
    );
    assert!(
        html.contains(r#"href="/api/hassio_ingress/tok/favicon.svg""#),
        "{html}"
    );
    assert!(
        html.contains(r#"src="/api/hassio_ingress/tok/brand/logo.svg""#),
        "{html}"
    );
    assert!(
        html.contains(r#"name="waveflow-base" content="/api/hassio_ingress/tok""#),
        "{html}"
    );
    // Not installable inside Home Assistant (RFC-030 decision 8).
    assert!(!html.contains("manifest"), "{html}");
}

#[test]
fn a_reference_that_leaves_the_origin_is_left_alone() {
    for root in ["", "/api/hassio_ingress/tok"] {
        let html = rewrite(BUILT, root);
        assert!(html.contains(r#"href="//elsewhere.example/x""#), "{html}");
        assert!(html.contains(r#"href="https://example.org/""#), "{html}");
    }
}
