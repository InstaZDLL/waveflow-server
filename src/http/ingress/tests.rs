use super::*;

use axum::{body::Body, routing::get, Router};
use tower::ServiceExt;

#[test]
fn a_prefix_is_home_assistants_path_and_one_url_safe_token() {
    let prefix = IngressPath::parse("/api/hassio_ingress/aB3-_x").expect("valid");
    assert_eq!(prefix.as_str(), "/api/hassio_ingress/aB3-_x");

    for refused in [
        "",
        "/api/hassio_ingress/",
        "/api/hassio_ingress",
        "/api/hassio_ingress/abc/",
        "/api/hassio_ingress/abc/def",
        "/api/hassio_ingress/a\"b",
        "/api/hassio_ingress/a b",
        "/api/hassio_ingress/a<b",
        "/api/hassio_ingress/a;b",
        "/other/abc",
        "api/hassio_ingress/abc",
    ] {
        assert!(IngressPath::parse(refused).is_none(), "{refused:?}");
    }
    assert!(IngressPath::parse(&format!("/api/hassio_ingress/{}", "a".repeat(128))).is_some());
    assert!(IngressPath::parse(&format!("/api/hassio_ingress/{}", "a".repeat(129))).is_none());
}

#[test]
fn only_the_configured_peers_may_connect() {
    let peers = vec!["172.30.32.2/32".parse::<IpNet>().unwrap()];
    assert!(peer_allowed(&peers, "172.30.32.2".parse().unwrap()));
    // The same address as a dual-stack socket reports it.
    assert!(peer_allowed(&peers, "::ffff:172.30.32.2".parse().unwrap()));
    assert!(!peer_allowed(&peers, "172.30.32.1".parse().unwrap()));
    assert!(!peer_allowed(&peers, "192.168.1.10".parse().unwrap()));
    assert!(!peer_allowed(&[], "172.30.32.2".parse().unwrap()));
}

fn ingress_router() -> Router {
    Router::new()
        .route(
            "/probe",
            get(|ingress: Ingress| async move { ingress.root().to_owned() }),
        )
        .layer(axum::middleware::from_fn(require_prefix))
}

async fn probe(router: Router, prefixes: &[&str]) -> (StatusCode, String) {
    let mut request = Request::builder().uri("/probe");
    for prefix in prefixes {
        request = request.header(INGRESS_PATH_HEADER, *prefix);
    }
    let response = router
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1 << 16)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn the_ingress_router_hands_the_checked_prefix_on() {
    let (status, root) = probe(ingress_router(), &["/api/hassio_ingress/tok"]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(root, "/api/hassio_ingress/tok");
}

#[tokio::test]
async fn the_ingress_router_refuses_a_request_without_a_valid_prefix() {
    for prefixes in [
        &[][..],
        &["/api/hassio_ingress/a\"b"][..],
        &["/api/hassio_ingress/one", "/api/hassio_ingress/two"][..],
    ] {
        let (status, body) = probe(ingress_router(), prefixes).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{prefixes:?}");
        assert!(body.contains("ingress_path_invalid"), "{body}");
    }
}

#[tokio::test]
async fn without_the_layer_the_header_is_never_read() {
    // The main listener's router: the same handler, no layer. A client that
    // sends the header there is served at the root all the same.
    let router = Router::new().route(
        "/probe",
        get(|ingress: Ingress| async move { ingress.root().to_owned() }),
    );
    let (status, root) = probe(router, &["/api/hassio_ingress/tok"]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(root, "");
}
