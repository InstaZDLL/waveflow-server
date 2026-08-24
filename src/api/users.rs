//! User administration and Subsonic credentials.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub username: String,
    pub web_password: String,
    pub role: crate::database::AccountRole,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub role: Option<crate::database::AccountRole>,
    pub disabled: Option<bool>,
    pub library_ids: Option<Vec<Uuid>>,
    pub subsonic_password: Option<String>,
    pub web_password: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetSubsonicCredentialRequest {
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubsonicCredentialResponse {
    /// Shown once. Only its SHA-256 hash is stored by the server.
    pub api_key: String,
}

#[utoipa::path(get, path = "/api/v2/admin/users", tag = "administration", responses((status = 200, body = [crate::services::UserItem]), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse)))]
pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::UserItem>>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .users(actor.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/admin/users", tag = "administration", request_body = CreateUserRequest, responses((status = 201, body = crate::services::UserItem), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<crate::services::UserItem>), ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    let user = state
        .services
        .create_web_user(
            actor.id,
            &request.username,
            &request.web_password,
            request.role,
        )
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(user)))
}

#[utoipa::path(patch, path = "/api/v2/admin/users/{username}", tag = "administration", params(("username" = String, Path)), request_body = UpdateUserRequest, responses((status = 200, body = crate::services::UserItem), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<crate::services::UserItem>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .update_user(
            actor.id,
            &username,
            crate::services::UserUpdate {
                admin: request
                    .role
                    .map(|role| role == crate::database::AccountRole::Admin),
                disabled: request.disabled,
                folder_ids: request.library_ids.as_deref(),
                subsonic_password: request.subsonic_password.as_deref(),
                web_password: request.web_password.as_deref(),
            },
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(delete, path = "/api/v2/admin/users/{username}", tag = "administration", params(("username" = String, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .delete_user(actor.id, &username)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(put, path = "/api/v2/admin/users/{username}/subsonic-credential", tag = "administration", params(("username" = String, Path)), request_body = SetSubsonicCredentialRequest, responses((status = 200, body = SubsonicCredentialResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_subsonic_credential(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SetSubsonicCredentialRequest>,
) -> Result<Json<SubsonicCredentialResponse>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    let api_key = state
        .services
        .set_subsonic_credential(actor.id, &username, &request.password)
        .await
        .map_err(service_error)?;
    Ok(Json(SubsonicCredentialResponse { api_key }))
}

#[utoipa::path(delete, path = "/api/v2/admin/users/{username}/subsonic-credential", tag = "administration", params(("username" = String, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn revoke_subsonic_credential(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .revoke_subsonic_credential(actor.id, &username)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
