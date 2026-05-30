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
//! - `503 {status: "not_ready", db: <error>}` otherwise. A 503 here
//!   tells a Kubernetes / systemd-style probe to keep waiting / stop
//!   routing traffic; it doesn't crash the process so a transient
//!   Postgres blip self-heals.

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::Serialize;
use sqlx::PgPool;

use crate::AppState;

#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    db: String,
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
                db: "ok".to_string(),
            }),
        ),
        Err(err) => {
            // Log at warn so a flapping DB shows up in dashboards
            // without burying the rest of the access log. Error
            // detail goes in the response body for the orchestrator's
            // probe logs; status stays a tight enum.
            tracing::warn!(error = %err, "readiness probe failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadyResponse {
                    status: "not_ready",
                    db: err.to_string(),
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
