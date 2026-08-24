//! First-run setup.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatusResponse {
    pub required: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
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

#[utoipa::path(post, path = "/api/v2/setup", tag = "authentication", params(("Origin" = String, Header, description = "Required browser origin")), request_body = SetupRequest, responses((status = 201, body = SetupResponse), (status = 403, description = "Origin header missing or rejected", body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetupRequest>,
) -> Result<(StatusCode, Json<SetupResponse>), ApiError> {
    validate_web_origin(&state, &headers)?;
    let user_id = state
        .services
        .bootstrap_admin(&request.username, &request.password)
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(SetupResponse { user_id })))
}
