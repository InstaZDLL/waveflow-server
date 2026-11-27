//! Public shares.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateShareRequest {
    pub track_ids: Vec<Uuid>,
    pub description: Option<String>,
    pub expires_at: Option<i64>,
    /// How many times the share may be opened; absent for no cap (RFC-020
    /// decision 6). A visit is one opening of the share, not one stream
    /// request, and the visit that reaches the cap keeps playing.
    #[schema(minimum = 1)]
    pub max_visits: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateShareRequest {
    pub description: Option<String>,
    pub expires_at: Option<i64>,
    /// A new visit cap. Lowering it to the count already reached closes the
    /// share at once.
    #[schema(minimum = 1)]
    pub max_visits: Option<i64>,
    /// Optional fields to blank out, by name: `description`, `expires_at`,
    /// `max_visits`.
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
    pub max_visits: Option<i64>,
    pub track_ids: Vec<Uuid>,
}

/// See [`playlist_clear`].
pub(super) fn share_clear(names: &[String]) -> Result<crate::services::ShareClear, ApiError> {
    let mut clear = crate::services::ShareClear::default();
    for name in names {
        match name.as_str() {
            "description" => clear.description = true,
            "expires_at" => clear.expires_at = true,
            "max_visits" => clear.max_visits = true,
            _ => return Err(ApiError::Validation),
        }
    }
    Ok(clear)
}

#[utoipa::path(get, path = "/api/v2/shares", tag = "user-data", responses((status = 200, body = [ShareResponse]), (status = 401, body = ErrorResponse)))]
pub async fn list_shares(
    State(state): State<AppState>,
    ingress: crate::http::ingress::Ingress,
    headers: HeaderMap,
) -> Result<Json<Vec<ShareResponse>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let shares = state
        .services
        .shares(user.id)
        .await
        .map_err(service_error)?
        .into_iter()
        .map(|share| share_response(&state, &ingress, share))
        .collect();
    Ok(Json(shares))
}

#[utoipa::path(post, path = "/api/v2/shares", tag = "user-data", request_body = CreateShareRequest, responses((status = 201, body = ShareResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse, description = "Inside Home Assistant, with no `WAVEFLOW_PUBLIC_URL`: the link would have no address (`share_address_missing`, RFC-030)"), (status = 422, body = ErrorResponse)))]
pub async fn create_share(
    State(state): State<AppState>,
    ingress: crate::http::ingress::Ingress,
    headers: HeaderMap,
    Json(request): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<ShareResponse>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    // Before anything is created: a share whose link nobody outside the panel
    // could open is not one to make (RFC-030 decision 8).
    if ingress.0.is_some() && state.public_url.is_none() {
        return Err(ApiError::ShareAddressMissing);
    }
    let context = mutation_context(&state, &headers, user.id).await?;
    let share = state
        .services
        .create_share_with_context(
            user.id,
            &request.track_ids,
            request.description.as_deref(),
            request.expires_at,
            request.max_visits,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok((
        StatusCode::CREATED,
        Json(share_response(&state, &ingress, share)),
    ))
}

#[utoipa::path(patch, path = "/api/v2/shares/{share_id}", tag = "user-data", params(("share_id" = Uuid, Path)), request_body = UpdateShareRequest, responses((status = 200, body = ShareResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn update_share(
    State(state): State<AppState>,
    ingress: crate::http::ingress::Ingress,
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
            request.max_visits,
            share_clear(&request.clear)?,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(Json(share_response(&state, &ingress, share)))
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

/// A share as the API answers it, with its link.
///
/// The link is the public URL followed by `/share/<token>`, or that path alone
/// when no public URL is set, which on the main listener is the server's own
/// address. Inside Home Assistant (RFC-030 decision 8) a bare path would name
/// a page of Home Assistant's, so a share listed there with no public URL
/// carries no link.
pub(super) fn share_response(
    state: &AppState,
    ingress: &crate::http::ingress::Ingress,
    share: crate::services::ShareItem,
) -> ShareResponse {
    let url = share.url_token.and_then(|token| {
        let path = format!("/share/{token}");
        match state.public_url.as_ref() {
            Some(base) => Some(format!("{base}{path}")),
            None if ingress.0.is_some() => None,
            None => Some(path),
        }
    });
    ShareResponse {
        id: share.id,
        url,
        description: share.description,
        expires_at: share.expires_at,
        created_at: share.created_at,
        visit_count: share.visit_count,
        max_visits: share.max_visits,
        track_ids: share.songs.into_iter().map(|song| song.id).collect(),
    }
}
