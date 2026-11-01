//! `waveflow-server health`, as a container's `HEALTHCHECK` runs it: the real
//! executable, pointed by `WAVEFLOW_BIND` at whatever holds the port, judged by
//! its exit status and its two streams.

use super::*;

use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;

use axum::{http::StatusCode, routing::get, Router};

/// Longer than the command's own timeout, so a test that waits this long is
/// looking at a probe that never gave up.
const COMMAND_DEADLINE: Duration = Duration::from_secs(60);

/// What `health` did when asked about `bind`.
struct Outcome {
    success: bool,
    stdout: String,
    stderr: String,
}

async fn health_against(bind: SocketAddr) -> Outcome {
    let temp = tempfile::tempdir().unwrap();
    let mut command = cli_command();
    // In a directory of its own, so no `.env` of the checkout is read.
    command
        .current_dir(temp.path())
        .env("WAVEFLOW_BIND", bind.to_string())
        .arg("health");
    let output = tokio::time::timeout(
        COMMAND_DEADLINE,
        tokio::process::Command::from(command)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("health gave no verdict")
    .unwrap();
    Outcome {
        success: output.status.success(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

/// `router`, served on a loopback port of its own.
async fn serving(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
    });
    address
}

/// A loopback port nobody listens on: drawn from the system, then let go.
fn free_port() -> SocketAddr {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.local_addr().unwrap()
}

fn assert_one_line(outcome: &Outcome, expected: &str) {
    assert!(!outcome.success, "health passed: {:?}", outcome.stderr);
    assert!(outcome.stdout.is_empty(), "stdout: {:?}", outcome.stdout);
    assert_eq!(
        outcome.stderr.trim_end().lines().count(),
        1,
        "not one line: {:?}",
        outcome.stderr
    );
    assert!(
        outcome.stderr.contains(expected),
        "{expected:?} not in {:?}",
        outcome.stderr
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ready_server_passes_in_silence() {
    let (_temp, config, state) = test_app().await;
    let address = serving(waveflow_server::app(&config, state)).await;

    let outcome = health_against(address).await;
    assert!(outcome.success, "health failed: {:?}", outcome.stderr);
    assert!(outcome.stdout.is_empty(), "stdout: {:?}", outcome.stdout);
    assert!(outcome.stderr.is_empty(), "stderr: {:?}", outcome.stderr);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_server_fails_with_one_line() {
    let address = free_port();

    let outcome = health_against(address).await;
    assert_one_line(&outcome, &format!("http://{address}/ready is unreachable"));
}

/// Something answers, and answers no: a server whose database is gone says
/// 503 on `/ready` while `/health` still says 200, and the check must follow
/// `/ready`.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_is_not_ready_fails_with_its_status() {
    let address =
        serving(Router::new().route("/ready", get(|| async { StatusCode::SERVICE_UNAVAILABLE })))
            .await;

    let outcome = health_against(address).await;
    assert_one_line(&outcome, "answered 503 Service Unavailable");
}

/// A 200 is not enough: whatever else holds the port may answer 200 to
/// anything, JSON included. (WaveFlow's own web client answers 200 too, with
/// its page, on any path it does not route — which a misspelt path in the
/// command would reach, and `a_ready_server_passes_in_silence` catches.)
#[tokio::test(flavor = "multi_thread")]
async fn a_200_that_is_not_the_readiness_probe_fails() {
    let address = serving(Router::new().route(
        "/ready",
        get(|| async { axum::Json(serde_json::json!({ "status": "ok" })) }),
    ))
    .await;

    let outcome = health_against(address).await;
    assert_one_line(&outcome, "not as WaveFlow's readiness probe");
}

/// A port that takes the connection and never answers: the check gives up on
/// its own, well before a container runtime would have to kill it.
#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_never_answers_fails_on_the_timeout() {
    // Bound and never accepted from: the kernel completes the handshake into
    // the backlog, and nobody reads the request.
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();

    let started = std::time::Instant::now();
    let outcome = health_against(address).await;
    assert_one_line(&outcome, "did not answer within 3 s");
    assert!(started.elapsed() < Duration::from_secs(30));
    drop(listener);
}
