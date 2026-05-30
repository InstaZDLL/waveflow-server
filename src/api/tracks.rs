//! `/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/*` —
//! deeply-nested tenant-scoped CRUD over the `track` table.
//!
//! Same pattern as [`super::libraries`], one level deeper. Every
//! handler reads [`UserId`] from the request extension, threads the
//! path's `profile_id` and `library_id` through, and calls a
//! `*_for_library` method on [`PostgresTrackRepository`]. The
//! repository SQL walks the full `track → library → profile → user`
//! ownership chain inline, so requests that target a foreign profile,
//! a foreign library, or a foreign track under an owned library all
//! short-circuit at the storage layer.
//!
//! 404 (vs 403) on missing or non-owned rows is deliberate — same
//! no-existence-leak rationale as `libraries.rs` and `profiles.rs`.
//! Adding a third tier doesn't change the contract, just the chain
//! depth.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use waveflow_core::{
    domain::track::TrackRow,
    repository::{
        postgres::PostgresTrackRepository,
        track::{TrackDraft, TrackUpdate},
    },
};

use crate::{middleware::UserId, AppState};

/// Wire-format track. Mirrors the desktop's `TrackRow` shape so a
/// future shared response-type extraction stays cheap, minus the
/// joined columns that are server-side stubs anyway (album / artist /
/// artwork tables haven't shipped). The omitted fields would always
/// be `null` until those tables land, so dropping them keeps the
/// payload tight; the client populates them via separate joins as
/// they become available.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrackResponse {
    pub id: i64,
    pub library_id: i64,
    pub title: String,
    pub duration_ms: i64,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub year: Option<i64>,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub bit_depth: Option<i64>,
    pub codec: Option<String>,
    pub musical_key: Option<String>,
    pub file_path: String,
    pub file_size: i64,
    pub added_at: i64,
    /// Raw POPM byte (0-255). 0-5 stars with half-step UI on the
    /// client side.
    pub rating: Option<i64>,
}

impl From<TrackRow> for TrackResponse {
    fn from(t: TrackRow) -> Self {
        Self {
            id: t.id,
            library_id: t.library_id,
            title: t.title,
            duration_ms: t.duration_ms,
            track_number: t.track_number,
            disc_number: t.disc_number,
            year: t.year,
            bitrate: t.bitrate,
            sample_rate: t.sample_rate,
            channels: t.channels,
            bit_depth: t.bit_depth,
            codec: t.codec,
            musical_key: t.musical_key,
            file_path: t.file_path,
            file_size: t.file_size,
            added_at: t.added_at,
            rating: t.rating,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTrackRequest {
    /// Track title shown in the UI. Trimmed and validated server-side
    /// — empty / whitespace-only after trim is rejected with 400.
    pub title: String,
    /// Absolute path on the host filesystem. The desktop reads + plays
    /// this; the server stores it as an identifier (the streaming
    /// endpoint that consumes it lands in a later phase).
    pub file_path: String,
    /// File size in bytes. Same units as the desktop's `track.file_size`.
    pub file_size: i64,
    /// Duration in milliseconds.
    pub duration_ms: i64,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub year: Option<i64>,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub bit_depth: Option<i64>,
    pub codec: Option<String>,
    pub musical_key: Option<String>,
}

/// Partial update payload. Every field is optional; the repository's
/// `COALESCE` keeps the existing value when a field is omitted, so
/// PATCH semantics fall out naturally. `title`, when present, is
/// trimmed and validated server-side — `Some("")` / `Some("   ")` is
/// rejected with 400 before the storage round-trip, same rule as
/// `CreateTrackRequest`.
///
/// `rating` is `u8` (0-255). Serde rejects 256+ at the JSON
/// deserialization boundary — the value can't reach the repository
/// out-of-range. The schema's `CHECK (rating BETWEEN 0 AND 255)` is
/// defense in depth on top of the type-level guarantee.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTrackRequest {
    pub title: Option<String>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub year: Option<i64>,
    pub rating: Option<u8>,
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_tracks, create_track))
        .routes(routes!(get_track, update_track, delete_track))
        .with_state(state)
}

/// List every track the calling user owns under
/// `(profile_id, library_id)`, most-recently-added first. A foreign
/// `profile_id` / `library_id` returns `[]` — no existence leak.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks",
    tag = "tracks",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
    ),
    responses(
        (status = 200, description = "Tracks under the library, most-recently-added first", body = Vec<TrackResponse>),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn list_tracks(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let repo = PostgresTrackRepository::new(state.db.clone());
    match repo.list_for_library(library_id, profile_id, user_id).await {
        Ok(tracks) => {
            let body: Vec<TrackResponse> = tracks.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(err) => {
            tracing::error!(
                error = %err,
                user_id,
                profile_id,
                library_id,
                "list tracks failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response()
        }
    }
}

/// Create a track under `(profile_id, library_id)`. The repo's
/// `INSERT … SELECT FROM library JOIN profile WHERE …` validates the
/// full chain atomically — a non-owned library / profile fails the
/// same round-trip as the write, no check-then-insert race.
#[utoipa::path(
    post,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks",
    tag = "tracks",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
    ),
    request_body = CreateTrackRequest,
    responses(
        (status = 201, description = "Track created", body = TrackResponse),
        (status = 400, description = "Empty / whitespace-only `title` or `file_path` after trim"),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 404, description = "Library / profile not owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn create_track(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id)): Path<(i64, i64)>,
    Json(req): Json<CreateTrackRequest>,
) -> impl IntoResponse {
    let title = req.title.trim();
    if title.is_empty() {
        return (StatusCode::BAD_REQUEST, "title is required").into_response();
    }
    // file_path is also identity for the (library_id, file_path)
    // unique index — a blank string would either let a duplicate
    // create succeed or surface as a confusing FK violation later.
    // Reject up front.
    let file_path = req.file_path.trim();
    if file_path.is_empty() {
        return (StatusCode::BAD_REQUEST, "file_path is required").into_response();
    }
    let now = Utc::now().timestamp_millis();
    let draft = TrackDraft {
        title: title.to_string(),
        file_path: file_path.to_string(),
        file_size: req.file_size,
        duration_ms: req.duration_ms,
        track_number: req.track_number,
        disc_number: req.disc_number,
        year: req.year,
        bitrate: req.bitrate,
        sample_rate: req.sample_rate,
        channels: req.channels,
        bit_depth: req.bit_depth,
        codec: req.codec,
        musical_key: req.musical_key,
        now_ms: now,
    };
    let repo = PostgresTrackRepository::new(state.db.clone());
    match repo
        .insert_for_library(&draft, library_id, profile_id, user_id)
        .await
    {
        Ok(Some(track)) => (StatusCode::CREATED, Json(TrackResponse::from(track))).into_response(),
        Ok(None) => {
            // Library doesn't exist OR isn't owned by the
            // (profile, user) pair — blurred so the response
            // doesn't leak existence of foreign rows.
            (StatusCode::NOT_FOUND, "library not found").into_response()
        }
        Err(err) => {
            tracing::error!(
                error = %err,
                user_id,
                profile_id,
                library_id,
                "create track failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "create failed").into_response()
        }
    }
}

/// Fetch one track by id, scoped to the entire `(library, profile,
/// user)` chain. 404 covers every non-owned case — missing id, foreign
/// library, foreign profile, foreign user — same no-leak rationale as
/// `get_library`.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}",
    tag = "tracks",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
        ("id" = i64, Path, description = "Track id"),
    ),
    responses(
        (status = 200, description = "Track found", body = TrackResponse),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 404, description = "No track with that id under the (library, profile) owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn get_track(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id, id)): Path<(i64, i64, i64)>,
) -> impl IntoResponse {
    let repo = PostgresTrackRepository::new(state.db.clone());
    match repo
        .get_for_library(id, library_id, profile_id, user_id)
        .await
    {
        Ok(Some(track)) => (StatusCode::OK, Json(TrackResponse::from(track))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "track not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                id,
                profile_id,
                library_id,
                user_id,
                "get track failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "get failed").into_response()
        }
    }
}

/// Partial update via `UPDATE … RETURNING …` — every `None` field
/// is left untouched by SQL `COALESCE`. Race-free against concurrent
/// delete because the update + read happen in one round-trip. `title`,
/// when supplied, must trim to non-empty (matches `create_track`).
#[utoipa::path(
    patch,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}",
    tag = "tracks",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
        ("id" = i64, Path, description = "Track id"),
    ),
    request_body = UpdateTrackRequest,
    responses(
        (status = 200, description = "Track updated", body = TrackResponse),
        (status = 400, description = "`title` was supplied but is empty / whitespace-only after trim"),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 404, description = "No track with that id under the (library, profile) owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn update_track(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id, id)): Path<(i64, i64, i64)>,
    Json(req): Json<UpdateTrackRequest>,
) -> impl IntoResponse {
    // Same boundary check as create — Some("") / Some("   ") for the
    // optional title is almost always a client bug, not user intent.
    // None stays legitimate (the COALESCE preserves the existing
    // value).
    let title = match req.title {
        Some(t) => {
            let trimmed = t.trim();
            if trimmed.is_empty() {
                return (StatusCode::BAD_REQUEST, "title must not be empty").into_response();
            }
            Some(trimmed.to_string())
        }
        None => None,
    };
    let patch = TrackUpdate {
        title,
        track_number: req.track_number,
        disc_number: req.disc_number,
        year: req.year,
        rating: req.rating,
    };
    let repo = PostgresTrackRepository::new(state.db.clone());
    match repo
        .update_for_library(id, &patch, library_id, profile_id, user_id)
        .await
    {
        Ok(Some(track)) => (StatusCode::OK, Json(TrackResponse::from(track))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "track not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                id,
                profile_id,
                library_id,
                user_id,
                "update track failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "update failed").into_response()
        }
    }
}

/// Delete a track. 204 on success, 404 when the row isn't owned by
/// the `(library, profile, user)` chain. The schema doesn't yet have
/// dependents on `track.id` (track_artist / track_genre / play_event
/// land in a later phase), so the delete is a single row removal for
/// now.
#[utoipa::path(
    delete,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}",
    tag = "tracks",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
        ("id" = i64, Path, description = "Track id"),
    ),
    responses(
        (status = 204, description = "Track deleted"),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 404, description = "No track with that id under the (library, profile) owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn delete_track(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id, id)): Path<(i64, i64, i64)>,
) -> impl IntoResponse {
    let repo = PostgresTrackRepository::new(state.db.clone());
    match repo
        .delete_for_library(id, library_id, profile_id, user_id)
        .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "track not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                id,
                profile_id,
                library_id,
                user_id,
                "delete track failed"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}
