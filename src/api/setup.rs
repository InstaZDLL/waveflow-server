//! First-run setup.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;
use crate::setup_token::Admission;

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatusResponse {
    pub required: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    /// The one-time setup token (RFC-019): the content of
    /// `<data dir>/setup-token`, or the value of `WAVEFLOW_SETUP_TOKEN` when
    /// the operator set one. Optional in the schema so that a request without
    /// it is answered `403 setup_token_rejected` rather than a parse error.
    #[serde(default)]
    pub setup_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupResponse {
    pub user_id: Uuid,
}

#[utoipa::path(get, path = "/api/v2/setup", tag = "authentication", responses((status = 200, body = SetupStatusResponse)))]
pub async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    let required = state.db.setup_required().await.map_err(db_error)?;
    Ok(Json(SetupStatusResponse { required }))
}

#[utoipa::path(post, path = "/api/v2/setup", tag = "authentication", params(("Origin" = String, Header, description = "Required browser origin")), request_body = SetupRequest, responses((status = 201, body = SetupResponse), (status = 403, description = "Origin header missing or rejected (`forbidden`), or the setup token missing or wrong (`setup_token_rejected`)", body = ErrorResponse), (status = 409, description = "An account already exists", body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetupRequest>,
) -> Result<(StatusCode, Json<SetupResponse>), ApiError> {
    validate_web_origin(&state, &headers)?;
    // Before the username and the password are looked at: a caller without the
    // token learns nothing from this route, not even the password rules.
    match state.setup.admit(request.setup_token.as_deref()) {
        Admission::Admitted => {}
        Admission::Rejected => return Err(ApiError::SetupTokenRejected),
        Admission::Closed => return Err(ApiError::Conflict),
    }
    let outcome = state
        .services
        .bootstrap_admin(&request.username, &request.password)
        .await;
    // Closed on success, and on the conflict that says an account was made
    // some other way meanwhile — the command line, beside a running server.
    // A refused username or password leaves it open, for the next attempt.
    if matches!(
        outcome,
        Ok(_) | Err(crate::services::ServiceError::Conflict)
    ) {
        state.setup.close();
    }
    let user_id = outcome.map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(SetupResponse { user_id })))
}
