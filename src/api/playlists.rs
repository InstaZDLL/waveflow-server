//! Playlists.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlaylistRequest {
    pub name: String,
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePlaylistRequest {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
    /// Track ids appended to the end, applied after `remove_indexes`.
    #[serde(default)]
    pub add: Vec<Uuid>,
    /// Zero-based positions removed before `add` is applied.
    #[serde(default)]
    pub remove_indexes: Vec<usize>,
    /// Optional fields to blank out, by name. Currently `comment`.
    ///
    /// Omitting a field leaves it untouched, so clearing needs its own verb:
    /// naming it here is the only way to distinguish "unchanged" from "empty",
    /// and it cannot fire by accident on a client that simply omits the field.
    #[serde(default)]
    pub clear: Vec<String>,
}

#[utoipa::path(get, path = "/api/v2/playlists", tag = "user-data", responses((status = 200, body = [crate::services::PlaylistItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_playlists(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::PlaylistItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .playlists(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/playlists", tag = "user-data", request_body = CreatePlaylistRequest, responses((status = 201, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreatePlaylistRequest>,
) -> Result<(StatusCode, Json<crate::services::PlaylistItem>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let playlist = state
        .services
        .create_playlist_with_context(user.id, &request.name, &request.track_ids, context)
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(playlist)))
}

#[utoipa::path(get, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .playlist(user.id, playlist_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// An unknown name is refused rather than ignored: a client asking to clear
/// `expiresAt` instead of `expires_at` would otherwise be told it succeeded
/// while the field stayed put.
pub(super) fn playlist_clear(names: &[String]) -> Result<crate::services::PlaylistClear, ApiError> {
    let mut clear = crate::services::PlaylistClear::default();
    for name in names {
        match name.as_str() {
            "comment" => clear.comment = true,
            _ => return Err(ApiError::Validation),
        }
    }
    Ok(clear)
}

#[utoipa::path(patch, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), request_body = UpdatePlaylistRequest, responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdatePlaylistRequest>,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .update_playlist_with_context(
            user.id,
            playlist_id,
            request.name.as_deref(),
            request.comment.as_deref(),
            request.public,
            &request.add,
            &request.remove_indexes,
            playlist_clear(&request.clear)?,
            context,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(delete, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .delete_playlist_with_context(user.id, playlist_id, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
