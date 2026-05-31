//! `/api/v1/profiles/{profile_id}/playlists/*` — tenant-scoped CRUD
//! over the `playlist` table.
//!
//! A playlist belongs directly to a profile (not nested under
//! library), so the ownership chain is the shorter
//! `playlist → profile → user` — same depth as libraries, different
//! parent. Every handler reads [`UserId`] from the request extension,
//! threads the path's `profile_id` straight through, and calls a
//! `*_for_profile` method on [`PostgresPlaylistRepository`]. The
//! repository SQL validates the chain inline, so requests targeting
//! a foreign profile / non-owned playlist short-circuit at the
//! storage layer.
//!
//! 1.b.5c ships custom playlists only. Smart playlists
//! (`is_smart = 1`, `smart_rules` JSON) and the playlist_track join
//! table are scheduled for later phases; the wire shape keeps the
//! fields stubbed so the web client doesn't need to adapt when they
//! materialise.
//!
//! 404 (vs 403) on missing or non-owned rows is deliberate — same
//! no-existence-leak rationale as `libraries.rs` and `profiles.rs`.

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
    domain::playlist::Playlist,
    repository::{
        playlist::{PlaylistDraft, PlaylistUpdate},
        postgres::PostgresPlaylistRepository,
    },
};

use crate::{middleware::UserId, AppState};

/// Brand defaults for `color_id` / `icon_id` mirror the desktop's
/// SQLite column defaults (`DEFAULT 'violet'` / `DEFAULT 'music'`).
const DEFAULT_COLOR_ID: &str = "violet";
const DEFAULT_ICON_ID: &str = "music";

/// Wire-format playlist. Mirrors the desktop's `Playlist` DTO minus
/// the path-derived `profile_id` and the desktop-only `cover_path`
/// (resolved app-side from `cover_hash` against the per-profile
/// artwork dir, which the server doesn't own — same NULL projection
/// as the repo's SELECT).
#[derive(Debug, Serialize, ToSchema)]
pub struct PlaylistResponse {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub color_id: String,
    pub icon_id: String,
    /// `0` for user-curated playlists, `1` for smart-generated ones.
    /// Always `0` today — server-side smart playlists land in a
    /// later phase.
    pub is_smart: i64,
    /// BLAKE3 hash of the cover image in the shared metadata cache.
    /// `None` until the artwork pipeline ships on the server.
    pub cover_hash: Option<String>,
    /// `1` when the cover is managed by the auto-regen pipeline,
    /// `0` when the user uploaded their own image and the pipeline
    /// should leave it alone. Always `1` on freshly-created rows
    /// here — matches the desktop convention.
    pub cover_is_auto: i64,
    pub position: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// Denormalised count, stubbed at `0` until `playlist_track`
    /// ships on the server.
    pub track_count: i64,
    /// Denormalised sum, stubbed at `0` until `playlist_track`
    /// ships on the server.
    pub total_duration_ms: i64,
    /// Raw JSON payload from `playlist.smart_rules`. Always `None`
    /// today (every server-side playlist is custom).
    pub smart_rules: Option<String>,
}

impl From<Playlist> for PlaylistResponse {
    fn from(p: Playlist) -> Self {
        Self {
            id: p.id,
            name: p.name,
            description: p.description,
            color_id: p.color_id,
            icon_id: p.icon_id,
            is_smart: p.is_smart,
            cover_hash: p.cover_hash,
            cover_is_auto: p.cover_is_auto,
            position: p.position,
            created_at: p.created_at,
            updated_at: p.updated_at,
            track_count: p.track_count,
            total_duration_ms: p.total_duration_ms,
            smart_rules: p.smart_rules,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlaylistRequest {
    /// Display name shown in the sidebar. Trimmed and validated
    /// server-side — empty / whitespace-only after trim is rejected
    /// with 400. The trimmed form is what gets persisted.
    pub name: String,
    /// Optional free-form description.
    pub description: Option<String>,
    /// Brand-defined design-system colour token. Falls back to
    /// `"violet"` (the desktop default) when omitted.
    pub color_id: Option<String>,
    /// Brand-defined design-system icon token. Falls back to
    /// `"music"` (the desktop default) when omitted.
    pub icon_id: Option<String>,
}

/// Partial update payload. Every field is optional; the repository's
/// `COALESCE` keeps the existing value when a field is omitted. `name`,
/// when present, is trimmed and validated server-side — `Some("")` /
/// `Some("   ")` is rejected with 400 before the storage round-trip,
/// same rule as `CreatePlaylistRequest`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePlaylistRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color_id: Option<String>,
    pub icon_id: Option<String>,
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_playlists, create_playlist))
        .routes(routes!(get_playlist, update_playlist, delete_playlist))
        .with_state(state)
}

/// List every playlist the calling user owns under `profile_id`,
/// ordered `(position ASC, updated_at DESC)` to match the desktop
/// sidebar. A foreign `profile_id` returns `[]` — no existence leak.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/playlists",
    tag = "playlists",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
    ),
    responses(
        (status = 200, description = "Playlists under the profile, in sidebar order", body = Vec<PlaylistResponse>),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn list_playlists(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path(profile_id): Path<i64>,
) -> impl IntoResponse {
    let repo = PostgresPlaylistRepository::new(state.db.clone());
    match repo.list_for_profile(profile_id, user_id).await {
        Ok(playlists) => {
            let body: Vec<PlaylistResponse> = playlists.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, user_id, profile_id, "list playlists failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response()
        }
    }
}

/// Create a custom playlist under `profile_id`. Smart playlists
/// aren't writable through this route — the repo hardcodes
/// `is_smart = 0`, `smart_rules = NULL`, `position = 0`,
/// `cover_hash = NULL`, `cover_is_auto = 1` (auto-managed slot,
/// matches the desktop default). The `INSERT … SELECT … WHERE …
/// AND p.user_id = $` clause guarantees atomicity — a non-owned
/// profile fails the same round-trip as the write.
#[utoipa::path(
    post,
    path = "/api/v1/profiles/{profile_id}/playlists",
    tag = "playlists",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
    ),
    request_body = CreatePlaylistRequest,
    responses(
        (status = 201, description = "Playlist created", body = PlaylistResponse),
        (status = 400, description = "Empty / whitespace-only `name` after trim"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Profile not owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn create_playlist(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path(profile_id): Path<i64>,
    Json(req): Json<CreatePlaylistRequest>,
) -> impl IntoResponse {
    let name = req.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "name is required").into_response();
    }
    let now = Utc::now().timestamp_millis();
    let draft = PlaylistDraft {
        name: name.to_string(),
        description: req.description,
        color_id: req.color_id.unwrap_or_else(|| DEFAULT_COLOR_ID.to_string()),
        icon_id: req.icon_id.unwrap_or_else(|| DEFAULT_ICON_ID.to_string()),
        now_ms: now,
    };
    let repo = PostgresPlaylistRepository::new(state.db.clone());
    match repo.insert_for_profile(&draft, profile_id, user_id).await {
        Ok(Some(playlist)) => {
            (StatusCode::CREATED, Json(PlaylistResponse::from(playlist))).into_response()
        }
        Ok(None) => {
            // Profile doesn't exist OR isn't owned by the caller —
            // blur the two so the response doesn't leak existence
            // of a foreign profile.
            (StatusCode::NOT_FOUND, "profile not found").into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, user_id, profile_id, "create playlist failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "create failed").into_response()
        }
    }
}

/// Fetch one playlist by id, scoped to both the profile and the
/// calling user. 404 covers "no such playlist", "playlist belongs to
/// a different profile", AND "profile belongs to a different user".
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/playlists/{id}",
    tag = "playlists",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("id" = i64, Path, description = "Playlist id"),
    ),
    responses(
        (status = 200, description = "Playlist found", body = PlaylistResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "No playlist with that id under the profile owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn get_playlist(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let repo = PostgresPlaylistRepository::new(state.db.clone());
    match repo.get_for_profile(id, profile_id, user_id).await {
        Ok(Some(playlist)) => {
            (StatusCode::OK, Json(PlaylistResponse::from(playlist))).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "playlist not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, profile_id, user_id, "get playlist failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "get failed").into_response()
        }
    }
}

/// Partial update via `UPDATE … RETURNING …`. Race-free against
/// concurrent delete; `name`, when supplied, must trim to non-empty.
#[utoipa::path(
    patch,
    path = "/api/v1/profiles/{profile_id}/playlists/{id}",
    tag = "playlists",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("id" = i64, Path, description = "Playlist id"),
    ),
    request_body = UpdatePlaylistRequest,
    responses(
        (status = 200, description = "Playlist updated", body = PlaylistResponse),
        (status = 400, description = "`name` was supplied but is empty / whitespace-only after trim"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "No playlist with that id under the profile owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn update_playlist(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, id)): Path<(i64, i64)>,
    Json(req): Json<UpdatePlaylistRequest>,
) -> impl IntoResponse {
    let name = match req.name {
        Some(n) => {
            let trimmed = n.trim();
            if trimmed.is_empty() {
                return (StatusCode::BAD_REQUEST, "name must not be empty").into_response();
            }
            Some(trimmed.to_string())
        }
        None => None,
    };
    let patch = PlaylistUpdate {
        name,
        description: req.description,
        color_id: req.color_id,
        icon_id: req.icon_id,
    };
    let now = Utc::now().timestamp_millis();
    let repo = PostgresPlaylistRepository::new(state.db.clone());
    match repo
        .update_for_profile(id, &patch, now, profile_id, user_id)
        .await
    {
        Ok(Some(playlist)) => {
            (StatusCode::OK, Json(PlaylistResponse::from(playlist))).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "playlist not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, profile_id, user_id, "update playlist failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "update failed").into_response()
        }
    }
}

/// Delete a playlist. 204 on success, 404 when the row isn't owned
/// by the (profile_id, user_id) pair. The future `playlist_track`
/// table will carry `ON DELETE CASCADE` on `playlist_id` so the
/// dependent rows go away in one statement once that schema lands.
#[utoipa::path(
    delete,
    path = "/api/v1/profiles/{profile_id}/playlists/{id}",
    tag = "playlists",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("id" = i64, Path, description = "Playlist id"),
    ),
    responses(
        (status = 204, description = "Playlist deleted"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "No playlist with that id under the profile owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn delete_playlist(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let repo = PostgresPlaylistRepository::new(state.db.clone());
    match repo.delete_for_profile(id, profile_id, user_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "playlist not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, profile_id, user_id, "delete playlist failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}
