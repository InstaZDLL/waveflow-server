//! Playback bookmarks.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

/// A playback position to store on a track.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BookmarkRequest {
    /// Milliseconds from the start of the file. Negative positions are refused.
    pub position_ms: i64,
    /// Free text. Omitting it clears whatever comment the bookmark carried,
    /// because a bookmark is replaced rather than patched.
    #[serde(default)]
    pub comment: Option<String>,
}

#[utoipa::path(get, path = "/api/v2/bookmarks", tag = "user-data", responses((status = 200, body = [crate::services::BookmarkItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_bookmarks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::BookmarkItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .bookmarks(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// One bookmark per account and track, so this replaces rather than adds.
///
/// `PUT` and not `POST` for that reason: the track names the resource, and
/// sending the same position twice leaves the same single bookmark. Backed by
/// the same `DomainServices` method as the Subsonic `createBookmark`, so the
/// two surfaces cannot disagree about what a second call does.
#[utoipa::path(put, path = "/api/v2/bookmarks/{track_id}", tag = "user-data", params(("track_id" = Uuid, Path)), request_body = BookmarkRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_bookmark(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<BookmarkRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .set_bookmark_with_context(
            user.id,
            track_id,
            request.position_ms,
            request.comment.as_deref(),
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deleting a bookmark that is not there succeeds: the caller asked for the
/// track to carry none, and it does not. It also avoids answering a question
/// about a track the account cannot reach.
#[utoipa::path(delete, path = "/api/v2/bookmarks/{track_id}", tag = "user-data", params(("track_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_bookmark(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .delete_bookmark_with_context(user.id, track_id, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
