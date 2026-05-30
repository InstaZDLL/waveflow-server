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

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::Serialize;
use sqlx::PgPool;

use crate::AppState;

#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    db: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new().route("/ready", get(ready)).with_state(state)
}

async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    match probe_db(&state.db).await {
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

async fn probe_db(pool: &PgPool) -> Result<(), sqlx::Error> {
    // `SELECT 1` is the canonical cheap connectivity check. We could
    // call into `waveflow-core`'s repository traits for a richer probe
    // (e.g. count profiles), but that would couple readiness to a
    // table that doesn't exist on the very first boot before migrations
    // run — `SELECT 1` is schema-agnostic and stays correct.
    sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(pool)
        .await
        .map(|_| ())
}
