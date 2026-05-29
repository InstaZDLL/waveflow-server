//! waveflow-server library entrypoint.
//!
//! The public API is intentionally tight: callers (the binary in
//! `main.rs` + integration tests) construct a [`Config`] and obtain a
//! ready-to-serve axum router via [`app`]. Internal modules stay
//! private until something outside this crate needs them.
//!
//! The full architectural intent lives in [RFC-001][rfc] — read that
//! before opening a PR that adds a new module.
//!
//! [rfc]: https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md

use axum::Router;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

pub mod api;
pub mod config;

pub use config::Config;

/// Header used for inbound + propagated request IDs. UUIDv4 by default
/// (via `MakeRequestUuid`), but a client / upstream proxy can supply
/// its own — useful for stitching traces across multiple services.
const REQUEST_ID_HEADER: &str = "x-request-id";

/// Build the axum router. Wired with:
/// - per-request UUID via `x-request-id` (generated if absent, echoed back).
/// - structured access logging keyed on the request id.
/// - configurable timeout (default 30 s, set via `WAVEFLOW_REQUEST_TIMEOUT_SECS`).
///
/// The router doesn't carry runtime state in 1.b.1 — `Config` is
/// consumed at build time. When sqlx pools land in 1.b.2 they'll be
/// wired here as `Router::with_state`.
pub fn app(config: Config) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(SetRequestIdLayer::new(
            axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(PropagateRequestIdLayer::new(
            axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
        ))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(config.request_timeout_secs),
        ));

    Router::new().merge(api::router()).layer(middleware)
}
