//! GET /health — liveness probe.
//!
//! Deliberately minimal: this endpoint must succeed even if every
//! downstream dependency (Postgres, plugin runtime, …) is broken.
//! Health-aware probes (readiness) get their own dedicated endpoint
//! `GET /ready` once we have downstream state to verify; for now the
//! server has no state, so the two would be identical and we ship
//! just the cheaper one.
//!
//! Returns the binary's compile-time `CARGO_PKG_VERSION` so a load
//! balancer's healthcheck log can correlate "node restarted" with
//! "node upgraded".

use axum::{routing::get, Json, Router};
use serde::Serialize;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

pub fn router() -> Router {
    Router::new().route("/health", get(health))
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: VERSION,
    })
}
