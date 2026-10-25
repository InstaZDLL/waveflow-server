//! Moving an entry of a playlist (the verb RFC-020 left open).

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct MovePlaylistEntryRequest {
    /// The zero-based position of the entry to move, in the list this account
    /// is shown.
    pub from: usize,
    /// The zero-based position it should be shown at once moved.
    pub to: usize,
    /// The track the caller believes sits at `from`. When it does not, the
    /// move is refused with 409 rather than applied to another entry: with
    /// repeats allowed, a position read before someone else's edit can name a
    /// different entry by the time it arrives.
    pub track_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RemovePlaylistEntryRequest {
    /// The zero-based position of the entry to remove, in the list this
    /// account is shown.
    pub position: usize,
    /// The track the caller believes sits at `position`; when it does not,
    /// the removal is refused with 409 rather than taking another entry.
    pub track_id: Option<Uuid>,
}

/// Removes one entry by the position shown, guarded like a move. The
/// `remove_indexes` of an update take positions with no such guard.
#[utoipa::path(post, path = "/api/v2/playlists/{playlist_id}/remove", tag = "user-data", params(("playlist_id" = Uuid, Path)), request_body = RemovePlaylistEntryRequest, responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, description = "`track_id` is not the track shown at `position`", body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn remove_playlist_entry(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<RemovePlaylistEntryRequest>,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .remove_playlist_entry_with_context(
            user.id,
            playlist_id,
            request.position,
            request.track_id,
            context,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/playlists/{playlist_id}/move", tag = "user-data", params(("playlist_id" = Uuid, Path)), request_body = MovePlaylistEntryRequest, responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, description = "`track_id` is not the track shown at `from`", body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn move_playlist_entry(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<MovePlaylistEntryRequest>,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .move_playlist_entry_with_context(
            user.id,
            playlist_id,
            request.from,
            request.to,
            request.track_id,
            context,
        )
        .await
        .map(Json)
        .map_err(service_error)
}
