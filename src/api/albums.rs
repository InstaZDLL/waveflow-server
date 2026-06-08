//! `/api/v1/profiles/{profile_id}/libraries/{library_id}/albums*` —
//! tenant-scoped read surface over the `album` table (Phase 4.d.0.4).
//!
//! Same nested pattern as `tracks.rs`: every handler reads [`UserId`]
//! from the request extension, threads the path's `profile_id` +
//! `library_id` through, and calls into [`crate::db::album`]. The
//! repository SQL walks `album → library → profile → user` inline so
//! requests targeting a foreign profile / foreign library / foreign
//! album short-circuit at the storage layer.
//!
//! Writes are NOT exposed here. Album rows materialise from the sync
//! apply pipeline (`apply::track`, phase 4.d.0.2) — they're derived
//! from the desktop's tag metadata, not user-created on the server.
//! Same rationale as the artist endpoints in [`super::artists`].
//!
//! 404 (vs 403) on missing / foreign-owned rows is deliberate — same
//! no-existence-leak rationale as every other tenant-scoped endpoint.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{db, middleware::UserId, AppState};

use super::tracks::TrackResponse;

/// Wire-format album row. Surfaces the `album_artist_name` joined
/// from `artist` so the web client's album-grid doesn't have to
/// resolve N artist names one-by-one. `album_artist_id` rides
/// alongside so the UI can deep-link straight into the artist
/// drill-down without a name → id lookup.
///
/// `album_artist_*` are `None` for compilations (the schema's
/// `NULLS NOT DISTINCT` natural key collapses NULL `album_artist_id`
/// to a single row per `(library, title)`); the UI renders the
/// "Various Artists" label client-side based on `is_compilation`.
#[derive(Debug, Serialize, ToSchema)]
pub struct AlbumResponse {
    pub id: i64,
    pub canonical_title: String,
    pub album_artist_id: Option<i64>,
    pub album_artist_name: Option<String>,
    pub year: Option<i64>,
    /// BLAKE3 hex of the album cover in the shared metadata cache.
    /// `None` until the server-side cover-extraction pipeline ships.
    pub cover_hash: Option<String>,
    pub is_compilation: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<db::album::AlbumRow> for AlbumResponse {
    fn from(row: db::album::AlbumRow) -> Self {
        Self {
            id: row.id,
            canonical_title: row.canonical_title,
            album_artist_id: row.album_artist_id,
            album_artist_name: row.album_artist_name,
            year: row.year,
            cover_hash: row.cover_hash,
            is_compilation: row.is_compilation,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_albums))
        .routes(routes!(list_album_tracks))
        .with_state(state)
}

/// List every album under `(profile_id, library_id)` owned by the
/// calling user, most-recently-updated first. 404 covers "no such
/// library" / "library belongs to a foreign profile" / "foreign user"
/// — same no-leak blur as `get_library`. An owned-but-empty library
/// returns `200 []`.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/albums",
    tag = "albums",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
    ),
    responses(
        (status = 200, description = "Albums under the library, most-recently-updated first", body = Vec<AlbumResponse>),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Library / profile not owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn list_albums(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    match db::album::list_for_library(&state.db, library_id, profile_id, user_id).await {
        Ok(Some(rows)) => {
            let body: Vec<AlbumResponse> = rows.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "library not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                user_id,
                profile_id,
                library_id,
                "list albums failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response()
        }
    }
}

/// Drill-down: list every track linked to `album_id` under
/// `(profile_id, library_id)`, ordered `(disc_number, track_number,
/// id)` so the standard "Side A → Side B" sleeve order falls out
/// naturally. 404 blurs every non-owned case. An owned album with
/// no remaining tracks (every linked track was deleted; the album
/// row outlives its tracks via `ON DELETE SET NULL`) returns
/// `200 []`.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/albums/{id}/tracks",
    tag = "albums",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
        ("id" = i64, Path, description = "Album id"),
    ),
    responses(
        (status = 200, description = "Tracks under the album, in sleeve order (may be empty)", body = Vec<TrackResponse>),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "No album with that id under the (library, profile) owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn list_album_tracks(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id, id)): Path<(i64, i64, i64)>,
) -> impl IntoResponse {
    match db::album::list_tracks_for_album(&state.db, id, library_id, profile_id, user_id).await {
        Ok(Some(rows)) => {
            let body: Vec<TrackResponse> = rows.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "album not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                id,
                user_id,
                profile_id,
                library_id,
                "list album tracks failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "list tracks failed").into_response()
        }
    }
}
