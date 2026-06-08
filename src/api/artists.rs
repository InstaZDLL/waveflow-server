//! `/api/v1/profiles/{profile_id}/libraries/{library_id}/artists*` —
//! tenant-scoped read surface over the `artist` table (Phase 4.d.0.4).
//!
//! Same shape as [`super::albums`], one table over. Writes are not
//! exposed — artist rows materialise from the sync apply pipeline
//! (`apply::track`), not from a user-facing CRUD form.
//!
//! The drill-down "tracks contributed by this artist" surfaces every
//! row from `track_artist` (multi-artist tracks appear under each
//! contributor), `ORDER BY (disc_number, track_number, id)` to match
//! the album drill-down's sleeve-order feel.

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

/// Wire-format artist row. `picture_hash` rides through the same
/// shared metadata cache as `album.cover_hash` / `playlist.cover_hash`
/// — `None` until the server-side artist-picture pipeline ships.
#[derive(Debug, Serialize, ToSchema)]
pub struct ArtistResponse {
    pub id: i64,
    pub name: String,
    pub picture_hash: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl From<db::artist::ArtistRow> for ArtistResponse {
    fn from(row: db::artist::ArtistRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            picture_hash: row.picture_hash,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_artists))
        .routes(routes!(list_artist_tracks))
        .with_state(state)
}

/// List every artist under `(profile_id, library_id)` owned by the
/// calling user, most-recently-updated first. 404 covers every
/// non-owned case; an owned-but-empty library returns `200 []`.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/artists",
    tag = "artists",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
    ),
    responses(
        (status = 200, description = "Artists under the library, most-recently-updated first", body = Vec<ArtistResponse>),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Library / profile not owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn list_artists(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    match db::artist::list_for_library(&state.db, library_id, profile_id, user_id).await {
        Ok(Some(rows)) => {
            let body: Vec<ArtistResponse> = rows.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "library not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                user_id,
                profile_id,
                library_id,
                "list artists failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response()
        }
    }
}

/// Drill-down: every track contributed by `artist_id` under
/// `(profile_id, library_id)`. Multi-artist tracks appear under every
/// contributor — `track_artist` is the source of truth, scoped by
/// `library_id` so the result never crosses a tenant boundary.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/artists/{id}/tracks",
    tag = "artists",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
        ("id" = i64, Path, description = "Artist id"),
    ),
    responses(
        (status = 200, description = "Tracks contributed by the artist (may be empty)", body = Vec<TrackResponse>),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "No artist with that id under the (library, profile) owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn list_artist_tracks(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id, id)): Path<(i64, i64, i64)>,
) -> impl IntoResponse {
    match db::artist::list_tracks_for_artist(&state.db, id, library_id, profile_id, user_id).await {
        Ok(Some(rows)) => {
            let body: Vec<TrackResponse> = rows.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "artist not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                id,
                user_id,
                profile_id,
                library_id,
                "list artist tracks failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "list tracks failed").into_response()
        }
    }
}
