//! Tracks and their lyrics.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize)]
pub struct TrackQuery {
    pub q: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/tracks", tag = "catalog", params(("library_id" = Uuid, Path), ("q" = Option<String>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::catalog::TrackRecord]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_tracks(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    Query(query): Query<TrackQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::catalog::TrackRecord>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    if state
        .db
        .library_for_user(user.id, library_id)
        .await
        .map_err(db_error)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(500);
    if offset < 0 || !(1..=500).contains(&limit) {
        return Err(ApiError::Validation);
    }
    let query = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let tracks = state
        .db
        .browse_tracks_for_user(user.id, library_id, query, offset, limit)
        .await
        .map_err(db_error)?;
    Ok(Json(tracks))
}

#[utoipa::path(get, path = "/api/v2/tracks/{track_id}", tag = "catalog", params(("track_id" = Uuid, Path)), responses((status = 200, body = crate::services::SongItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_track(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::SongItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .songs_by_ids(user.id, &[track_id])
        .await
        .map_err(service_error)?
        .into_iter()
        .next()
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(get, path = "/api/v2/tracks/{track_id}/lyrics", tag = "catalog", params(("track_id" = Uuid, Path)), responses((status = 200, body = crate::lyrics::LyricsList), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_track_lyrics(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::lyrics::LyricsList>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .lyrics(user.id, track_id)
        .await
        .map(Json)
        .map_err(service_error)
}
