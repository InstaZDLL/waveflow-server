//! GET /ready — DB-aware readiness probe.
//!
//! `/health` only proves the binary is alive and serving requests.
//! `/ready` is the orchestrator's signal that downstream dependencies
//! are reachable — for now just Postgres; the future plugin host and
//! background-job runner (RFC-001 1.f) will plug in here once they
//! exist.
//!
//! Returns:
//! - `200 {status: "ready", db: "ok"}` when the pool can acquire a
//!   connection and `SELECT 1` round-trips.
//! - `503 {status: "not_ready", db: "unavailable"}` otherwise. The
//!   sqlx error detail is emitted to `tracing::warn!` only — the body
//!   keeps a fixed sentinel so an unauthenticated probe (load
//!   balancer, healthchecker) doesn't see the connection-URL host or
//!   credentials. A 503 here tells a Kubernetes / systemd-style probe
//!   to keep waiting / stop routing traffic; the process keeps
//!   running so a transient Postgres blip self-heals.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{db, AppState};

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadyResponse {
    /// `"ready"` when every probed dependency is reachable, `"not_ready"`
    /// otherwise.
    #[schema(example = "ready")]
    pub status: &'static str,
    /// `"ok"` when the Postgres pool responded to the connectivity
    /// probe, `"unavailable"` otherwise. The sqlx error detail is
    /// emitted to `tracing::warn!` only — never returned in the body,
    /// since unauthenticated probes (load balancers, healthcheckers)
    /// shouldn't see the connection URL host / credentials.
    #[schema(example = "ok")]
    pub db: &'static str,
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(ready))
        .with_state(state)
}

/// Readiness probe — confirms every downstream dependency (Postgres
/// today, plugin host + background-job runner later) is reachable.
/// Returns 503 when degraded so a Kubernetes / systemd-style probe
/// stops routing traffic without crashing the process — a transient
/// Postgres blip self-heals.
#[utoipa::path(
    get,
    path = "/ready",
    tag = "probes",
    responses(
        (status = 200, description = "Every probed dependency is healthy", body = ReadyResponse),
        (status = 503, description = "At least one dependency is degraded", body = ReadyResponse),
    ),
)]
async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    // The actual `SELECT 1` lives in `db::ping` so this handler stays
    // pure HTTP orchestration — same boundary the project enforces
    // between Tauri commands and `waveflow-core` on the desktop side.
    match db::ping(&state.db).await {
        Ok(()) => (
            StatusCode::OK,
            Json(ReadyResponse {
                status: "ready",
                db: "ok",
            }),
        ),
        Err(err) => {
            // Keep the sqlx error detail in the warn log only — a
            // sqlx error string can reveal the connection-URL host /
            // user and is not the kind of thing /ready should leak to
            // an unauthenticated caller (the typical /ready consumer
            // is a load balancer with no auth header). The body keeps
            // a non-sensitive sentinel so the response shape stays a
            // tight enum.
            tracing::warn!(error = %err, "readiness probe failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadyResponse {
                    status: "not_ready",
                    db: "unavailable",
                }),
            )
        }
    }
}
