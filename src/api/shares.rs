//! Public shares.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateShareRequest {
    pub track_ids: Vec<Uuid>,
    pub description: Option<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateShareRequest {
    pub description: Option<String>,
    pub expires_at: Option<i64>,
    /// Optional fields to blank out, by name: `description`, `expires_at`.
    ///
    /// Without this, an expiry set by mistake is permanent — `COALESCE` reads an
    /// absent field and an explicit null identically, so the owner's only
    /// recourse would be deleting the share and publishing a different URL.
    #[serde(default)]
    pub clear: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShareResponse {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub description: Option<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub visit_count: i64,
    pub track_ids: Vec<Uuid>,
}

/// See [`playlist_clear`].
pub(super) fn share_clear(names: &[String]) -> Result<crate::services::ShareClear, ApiError> {
    let mut clear = crate::services::ShareClear::default();
    for name in names {
        match name.as_str() {
            "description" => clear.description = true,
            "expires_at" => clear.expires_at = true,
            _ => return Err(ApiError::Validation),
        }
    }
    Ok(clear)
}

#[utoipa::path(get, path = "/api/v2/shares", tag = "user-data", responses((status = 200, body = [ShareResponse]), (status = 401, body = ErrorResponse)))]
pub async fn list_shares(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ShareResponse>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let shares = state
        .services
        .shares(user.id)
        .await
        .map_err(service_error)?
        .into_iter()
        .map(|share| share_response(&state, share))
        .collect();
    Ok(Json(shares))
}

#[utoipa::path(post, path = "/api/v2/shares", tag = "user-data", request_body = CreateShareRequest, responses((status = 201, body = ShareResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<ShareResponse>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let share = state
        .services
        .create_share_with_context(
            user.id,
            &request.track_ids,
            request.description.as_deref(),
            request.expires_at,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(share_response(&state, share))))
}

#[utoipa::path(patch, path = "/api/v2/shares/{share_id}", tag = "user-data", params(("share_id" = Uuid, Path)), request_body = UpdateShareRequest, responses((status = 200, body = ShareResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn update_share(
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdateShareRequest>,
) -> Result<Json<ShareResponse>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let share = state
        .services
        .update_share_with_context(
            user.id,
            share_id,
            request.description.as_deref(),
            request.expires_at,
            share_clear(&request.clear)?,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(Json(share_response(&state, share)))
}

#[utoipa::path(delete, path = "/api/v2/shares/{share_id}", tag = "user-data", params(("share_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_share(
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .delete_share_with_context(user.id, share_id, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn share_response(state: &AppState, share: crate::services::ShareItem) -> ShareResponse {
    let url = share.url_token.map(|token| {
        let path = format!("/share/{token}");
        state
            .public_url
            .as_ref()
            .map_or_else(|| path.clone(), |base| format!("{base}{path}"))
    });
    ShareResponse {
        id: share.id,
        url,
        description: share.description,
        expires_at: share.expires_at,
        created_at: share.created_at,
        visit_count: share.visit_count,
        track_ids: share.songs.into_iter().map(|song| song.id).collect(),
    }
}
