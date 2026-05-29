//! End-to-end smoke test for the /health endpoint.
//!
//! Spawns the actual axum app on a kernel-assigned port, fires a
//! reqwest call at it, and validates the response shape. This is the
//! template for every future endpoint test — it confirms that the
//! router wiring + middleware stack actually reach the handler, not
//! just that the handler compiles.

use std::net::SocketAddr;

use serde_json::Value;
use waveflow_server::{app, Config};

#[tokio::test]
async fn health_returns_ok_with_version() {
    // `127.0.0.1:0` lets the kernel hand us a free port — concurrent
    // test runs (cargo's default behaviour) don't collide.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = Config {
        bind_addr: addr,
        request_timeout_secs: 5,
    };

    // Serve in the background. The test holds no handle on the task —
    // it drops at process exit, which is fine for a one-shot test.
    let app = app(config);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let client = reqwest::Client::new();
    let body: Value = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("request failed")
        .error_for_status()
        .expect("non-2xx response")
        .json()
        .await
        .expect("invalid JSON");

    assert_eq!(body["status"], "ok");
    let version = body["version"].as_str().expect("version missing");
    assert!(
        !version.is_empty(),
        "version should mirror CARGO_PKG_VERSION"
    );
}

#[tokio::test]
async fn health_propagates_inbound_request_id() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let config = Config {
        bind_addr: addr,
        request_timeout_secs: 5,
    };

    let app = app(config);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    let provided = "test-request-id-1234";
    let resp = reqwest::Client::new()
        .get(format!("http://{addr}/health"))
        .header("x-request-id", provided)
        .send()
        .await
        .expect("request failed");

    let echoed = resp
        .headers()
        .get("x-request-id")
        .expect("server dropped x-request-id")
        .to_str()
        .unwrap();

    assert_eq!(
        echoed, provided,
        "server must echo a client-supplied request id verbatim"
    );
}
