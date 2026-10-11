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
        .create_api_token(
            actor.id,
            &actor.scopes,
            &username,
            &request.name,
            &request.scopes,
        )
        .await
        // 403 and not the blanket 404, the same way the self-scoped route does
        // it and for the same reason: nothing is being looked up when a
        // credential is refused the right to hand on what it does not hold,
        // and a 404 would tell an administrator the account they just named
        // does not exist.
        .map_err(|error| match error {
            crate::services::ServiceError::Forbidden => ApiError::Forbidden,
            other => service_error(other),
        })?;
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

/// The clients an account has registered for itself.
///
/// **Self-scoped, where the three routes above are administrative**, and the
/// reason is what makes them different rather than redundant: a token carries
/// the authority of the account it belongs to and never more —
/// `authenticated` checks the role separately from the scopes, so a scope
/// narrows a credential and cannot promote one. So which devices are reading
/// an account's library, and stopping one, is a question about that account.
///
/// Until now it needed the instance's administrator. A listener could not find
/// out which clients held a credential on their own library, let alone revoke
/// one — the material was there and visible nowhere.
#[utoipa::path(get, path = "/api/v2/api-tokens", tag = "account", responses((status = 200, body = [crate::database::ApiTokenRecord]), (status = 401, body = ErrorResponse)))]
pub async fn list_own_api_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::database::ApiTokenRecord>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .own_api_tokens(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Issues a client credential to the account asking.
///
/// A credential cannot mint one wider than itself: an empty scope list means
/// "everything this account has", so a narrowed caller asking for one is
/// refused. See `DomainServices::create_own_api_token`.
#[utoipa::path(post, path = "/api/v2/api-tokens", tag = "account", request_body = CreateApiTokenRequest, responses((status = 201, body = CreateApiTokenResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_own_api_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateApiTokenRequest>,
) -> Result<(StatusCode, Json<CreateApiTokenResponse>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let (token, secret) = state
        .services
        .create_own_api_token(
            user.id,
            &user.scopes,
            user.credential_expires_at,
            &request.name,
            &request.scopes,
        )
        .await
        // **403 here, against the blanket mapping that sends `Forbidden` to
        // 404.** That blur exists so a surface never confirms the existence of
        // another tenant's resource; nothing is being looked up here. A 404
        // would tell a caller its own account does not exist, when what
        // happened is that its credential may not hand on what it asked for —
        // and the difference is the whole of what the caller has to fix.
        .map_err(|error| match error {
            crate::services::ServiceError::Forbidden => ApiError::Forbidden,
            other => service_error(other),
        })?;
    Ok((
        StatusCode::CREATED,
        Json(CreateApiTokenResponse { token, secret }),
    ))
}

/// Revokes one of the asking account's own credentials.
///
/// A token belonging to somebody else answers as a missing one — here the
/// blur is exactly right, because something *is* being looked up and naming
/// another tenant's token must not confirm that it exists.
#[utoipa::path(delete, path = "/api/v2/api-tokens/{token_id}", tag = "account", params(("token_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn revoke_own_api_token(
    State(state): State<AppState>,
    Path(token_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .revoke_own_api_token(user.id, token_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
