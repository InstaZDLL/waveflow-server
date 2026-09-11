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

/// What the file says and what a correction says instead.
///
/// Separate from `GET /api/v2/tracks/{track_id}`, which answers the effective
/// value and should keep doing so: the catalogue has no use for provenance,
/// and putting it on `SongItem` would weigh down the type every listing
/// returns — a type the frozen Subsonic façade also builds from.
///
/// It is the read half of correcting a tag. The write half, `PATCH
/// /api/v2/tracks/{track_id}`, is a partial patch, so a client no longer needs
/// this route to avoid dropping corrections it did not mean to touch. What it
/// still needs it for is showing them: which fields are corrected, what the
/// file says beneath, and what removing a correction would give back.
#[utoipa::path(get, path = "/api/v2/tracks/{track_id}/overrides", tag = "catalog", params(("track_id" = Uuid, Path)), responses((status = 200, body = crate::services::TrackOverrides), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_track_overrides(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::TrackOverrides>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .track_overrides(user.id, track_id)
        .await
        .map(Json)
        .map_err(service_error)
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

/// Corrects some of a track's tags, leaving the rest as they are.
///
/// A partial patch in three states. A field **absent** from the body leaves
/// that correction alone, **`null`** removes it and hands the field back to the
/// file, and a **value** sets it. `{}` therefore changes nothing. A blank string
/// reads as `null`; `[]` is a value for the two lists, meaning the track credits
/// nobody.
///
/// It replaced the whole set until #177, which dropped every correction a
/// client did not mention — another client's included. The one client that sent
/// it spelled every field out, `null` included, so its requests mean what they
/// meant.
///
/// The file on disk is never written. `full_hash` therefore does not move, so a
/// client holding a content-based link to this track still holds it afterwards.
#[utoipa::path(
    patch,
    path = "/api/v2/tracks/{track_id}",
    tag = "catalog",
    params(("track_id" = Uuid, Path)),
    request_body = crate::services::TrackMetadataPatch,
    responses(
        (status = 200, body = crate::services::SongItem),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn update_track(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
    Json(patch): Json<crate::services::TrackMetadataPatch>,
) -> Result<Json<crate::services::SongItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    // Named on the library feed, so the client that made the correction can
    // tell its own change from somebody else's.
    let device = origin_device(&state, &headers, user.id).await?;
    state
        .services
        .set_track_metadata(user.id, track_id, patch, device)
        .await
        .map(Json)
        .map_err(service_error)
}
