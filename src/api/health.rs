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

use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    /// Always `"ok"` while the process is serving requests.
    #[schema(example = "ok")]
    pub status: &'static str,
    /// Mirrors `CARGO_PKG_VERSION` of the running binary. Useful for
    /// correlating a healthcheck restart with a deploy.
    #[schema(example = "0.0.0")]
    pub version: &'static str,
}

pub fn router() -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(health))
}

/// Liveness probe — always succeeds while the process is serving
/// requests. Doesn't touch the database; use `/ready` for a probe
/// that confirms downstream dependencies are reachable.
#[utoipa::path(
    get,
    path = "/health",
    tag = "probes",
    responses(
        (status = 200, description = "Process is alive", body = HealthResponse),
    ),
)]
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: VERSION,
    })
}
