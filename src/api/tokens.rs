//! API tokens.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

/// A token to issue.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApiTokenRequest {
    /// What the token is for. Shown in the listing so a stale one can be told
    /// apart from a live one before it is revoked.
    pub name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// An issued token. The secret appears here and nowhere else, ever again.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateApiTokenResponse {
    #[serde(flatten)]
    pub token: crate::database::ApiTokenRecord,
    /// Shown once. Only its SHA-256 hash is stored, so it cannot be recovered.
    pub secret: String,
}

#[utoipa::path(get, path = "/api/v2/admin/users/{username}/tokens", tag = "administration", params(("username" = String, Path)), responses((status = 200, body = [crate::database::ApiTokenRecord]), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn list_api_tokens(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::database::ApiTokenRecord>>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .api_tokens(actor.id, &username)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Issues an API token without a shell on the host.
///
/// The `token create` CLI command remains, for bootstrapping an instance that
/// has no administrator session yet; from here on the two share
/// `DomainServices::create_api_token`, so a token minted either way carries the
/// same scopes and the same audit trail.
#[utoipa::path(post, path = "/api/v2/admin/users/{username}/tokens", tag = "administration", params(("username" = String, Path)), request_body = CreateApiTokenRequest, responses((status = 201, body = CreateApiTokenResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_api_token(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateApiTokenRequest>,
) -> Result<(StatusCode, Json<CreateApiTokenResponse>), ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    let (token, secret) = state
        .services
        .create_api_token(actor.id, &username, &request.name, &request.scopes)
        .await
        .map_err(service_error)?;
    Ok((
        StatusCode::CREATED,
        Json(CreateApiTokenResponse { token, secret }),
    ))
}

#[utoipa::path(delete, path = "/api/v2/admin/users/{username}/tokens/{token_id}", tag = "administration", params(("username" = String, Path), ("token_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn revoke_api_token(
    State(state): State<AppState>,
    Path((username, token_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .revoke_api_token(actor.id, &username, token_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
