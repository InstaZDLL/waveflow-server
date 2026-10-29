//! Liveness and readiness.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Serialize, ToSchema)]
pub struct ProbeResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub schema: u8,
    /// The source of this version: the repository at the tag `v<version>`,
    /// offered to every network user as AGPL-3.0 §13 requires. Public like
    /// the probe, so the sign-in page can link to it before anyone signs in.
    pub source: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadyResponse {
    pub status: &'static str,
    pub database: &'static str,
}

#[utoipa::path(
    get,
    path = "/health",
    tag = "probes",
    responses((status = 200, body = ProbeResponse))
)]
pub async fn health() -> Json<ProbeResponse> {
    Json(ProbeResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        schema: 2,
        source: crate::app::SOURCE_URL,
    })
}

#[utoipa::path(
    get,
    path = "/ready",
    tag = "probes",
    responses(
        (status = 200, body = ReadyResponse),
        (status = 503, body = ReadyResponse)
    )
)]
pub async fn ready(State(state): State<AppState>) -> Response {
    match state.db.ping().await {
        Ok(()) => (
            StatusCode::OK,
            Json(ReadyResponse {
                status: "ready",
                database: "ok",
            }),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "readiness database probe failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadyResponse {
                    status: "unavailable",
                    database: "unavailable",
                }),
            )
                .into_response()
        }
    }
}
