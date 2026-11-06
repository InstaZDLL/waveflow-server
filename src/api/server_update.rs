//! The update check (RFC-028): what an administrator is told, and their answer.
//!
//! Administrative routes, refused to anyone else as the other ones are.
//! Handlers orchestrate HTTP only; the rules live in `DomainServices`.

use super::*;

/// An administrator's answer to "may this server check for updates?".
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateCheckRequest {
    pub enabled: bool,
}

/// The policy, the stored answer, the running version and the newest one
/// known, while the check is on.
#[utoipa::path(get, path = "/api/v2/server/update", tag = "server", responses((status = 200, body = crate::services::UpdateStatus), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse)))]
pub async fn get_server_update(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<crate::services::UpdateStatus>, ApiError> {
    let user = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .update_status(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Records whether the server may check, under `WAVEFLOW_UPDATE_CHECK=ask`;
/// `409` when the environment has decided. A yes checks at once, in the
/// background: the answer is the state before that check ends.
#[utoipa::path(put, path = "/api/v2/server/update", tag = "server", request_body = UpdateCheckRequest, responses((status = 200, body = crate::services::UpdateStatus), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 409, body = ErrorResponse)))]
pub async fn put_server_update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateCheckRequest>,
) -> Result<Json<crate::services::UpdateStatus>, ApiError> {
    let user = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .set_update_check(user.id, request.enabled)
        .await
        .map(Json)
        .map_err(service_error)
}
