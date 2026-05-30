//! `/api/v1/profiles/{profile_id}/libraries/*` — nested tenant-scoped
//! CRUD over the `library` table.
//!
//! Same design as [`super::profiles`]: every handler reads
//! [`UserId`] from the request extension that
//! `middleware::require_user_id` attached, threads the path's
//! `profile_id` straight through, and calls a `*_for_profile` method on
//! [`PostgresLibraryRepository`]. The repository SQL validates the
//! `library → profile → user` chain inline, so a request that targets
//! a foreign profile (or a foreign library under an owned profile)
//! short-circuits at the storage layer with a `None` / `false` instead
//! of returning data.
//!
//! 404 (vs 403) on missing or non-owned rows is deliberate: the
//! production goal is to hide whether a row exists at all from a
//! caller who doesn't own it. The same rationale documented in
//! `profiles.rs`.

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
    domain::library::Library,
    repository::{
        library::{LibraryDraft, LibraryUpdate},
        postgres::PostgresLibraryRepository,
    },
};

use crate::{middleware::UserId, AppState};

/// Brand defaults for `color_id` / `icon_id` mirror the desktop's
/// SQLite column defaults (`DEFAULT 'emerald'` / `DEFAULT 'library'`).
/// The `INSERT` still names the columns explicitly so the server stays
/// authoritative on what gets written when the client omits them.
const DEFAULT_COLOR_ID: &str = "emerald";
const DEFAULT_ICON_ID: &str = "library";

/// Wire-format library. Same shape as
/// [`waveflow_core::domain::library::Library`] minus `profile_id` —
/// the field exists on the row (and powers the FK to `profile`) but
/// the client always knows it from the URL path, so echoing it back
/// would be redundant noise.
#[derive(Debug, Serialize, ToSchema)]
pub struct LibraryResponse {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub color_id: String,
    pub icon_id: String,
    pub created_at: i64,
    pub updated_at: i64,
    /// Denormalised counts. Stubbed at `0` for now; populated by the
    /// real aggregates once track / album / folder tables ship in
    /// 1.b.5b. The wire field stays so the web client doesn't need to
    /// adapt when the values become real.
    pub track_count: i64,
    pub album_count: i64,
    pub artist_count: i64,
    pub genre_count: i64,
    pub folder_count: i64,
}

impl From<Library> for LibraryResponse {
    fn from(l: Library) -> Self {
        Self {
            id: l.id,
            name: l.name,
            description: l.description,
            color_id: l.color_id,
            icon_id: l.icon_id,
            created_at: l.created_at,
            updated_at: l.updated_at,
            track_count: l.track_count,
            album_count: l.album_count,
            artist_count: l.artist_count,
            genre_count: l.genre_count,
            folder_count: l.folder_count,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLibraryRequest {
    /// Display name shown in the sidebar shelf. Trimmed and validated
    /// server-side — a payload whose `name` is empty or whitespace-only
    /// after trim is rejected with 400 before the storage round-trip.
    /// The trimmed form is what gets persisted.
    pub name: String,
    /// Optional free-form description ("Live recordings 2024-2026", …).
    pub description: Option<String>,
    /// Brand-defined design-system colour token. Falls back to
    /// `"emerald"` (the desktop default) when omitted.
    pub color_id: Option<String>,
    /// Brand-defined design-system icon token. Falls back to
    /// `"library"` (the desktop default) when omitted.
    pub icon_id: Option<String>,
}

/// Partial update payload. Every field is optional; the repository's
/// `COALESCE` keeps the existing value when a field is omitted, so
/// PATCH semantics fall out naturally. `name`, when present, is
/// trimmed and validated server-side — a `Some("")` / `Some("   ")`
/// payload is rejected with 400 before the storage round-trip, same
/// rule as `CreateLibraryRequest`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateLibraryRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub color_id: Option<String>,
    pub icon_id: Option<String>,
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_libraries, create_library))
        .routes(routes!(get_library, update_library, delete_library))
        .with_state(state)
}

/// List every library the calling user owns under `profile_id`,
/// most-recently-updated first. A foreign `profile_id` (one the user
/// doesn't own) returns `[]` — no existence leak, matches
/// `list_profiles`'s rationale.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/libraries",
    tag = "libraries",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
    ),
    responses(
        (status = 200, description = "Libraries under the profile, most-recently-updated first", body = Vec<LibraryResponse>),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn list_libraries(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path(profile_id): Path<i64>,
) -> impl IntoResponse {
    let repo = PostgresLibraryRepository::new(state.db.clone());
    match repo.list_for_profile(profile_id, user_id).await {
        Ok(libs) => {
            let body: Vec<LibraryResponse> = libs.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, user_id, profile_id, "list libraries failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response()
        }
    }
}

/// Create a library under `profile_id`. The `INSERT … SELECT …
/// WHERE … AND p.user_id = $` clause in the repo guarantees the
/// profile is owned by the calling user atomically — no
/// check-then-insert race. A non-owned (or non-existent) `profile_id`
/// surfaces as 404 here, same no-existence-leak rationale as
/// `get_profile`.
#[utoipa::path(
    post,
    path = "/api/v1/profiles/{profile_id}/libraries",
    tag = "libraries",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
    ),
    request_body = CreateLibraryRequest,
    responses(
        (status = 201, description = "Library created", body = LibraryResponse),
        (status = 400, description = "Empty or whitespace-only `name` after trim"),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 404, description = "Profile not owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn create_library(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path(profile_id): Path<i64>,
    Json(req): Json<CreateLibraryRequest>,
) -> impl IntoResponse {
    // Validate at the boundary — don't trust client-side trimming. An
    // empty-name row would render as a blank shelf in the sidebar and
    // is almost certainly the result of a client bug rather than user
    // intent, so reject before the DB round-trip.
    let name = req.name.trim();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, "name is required").into_response();
    }
    let now = Utc::now().timestamp_millis();
    let draft = LibraryDraft {
        name: name.to_string(),
        description: req.description,
        color_id: req.color_id.unwrap_or_else(|| DEFAULT_COLOR_ID.to_string()),
        icon_id: req.icon_id.unwrap_or_else(|| DEFAULT_ICON_ID.to_string()),
        now_ms: now,
    };
    let repo = PostgresLibraryRepository::new(state.db.clone());
    match repo.insert_for_profile(&draft, profile_id, user_id).await {
        Ok(Some(library)) => {
            (StatusCode::CREATED, Json(LibraryResponse::from(library))).into_response()
        }
        Ok(None) => {
            // Profile doesn't exist OR isn't owned by the caller —
            // blur the two so the response doesn't leak existence of
            // a foreign profile.
            (StatusCode::NOT_FOUND, "profile not found").into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, user_id, profile_id, "create library failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "create failed").into_response()
        }
    }
}

/// Fetch one library by id, scoped to both the path's profile and the
/// calling user. 404 covers "no such library", "library belongs to a
/// different profile", AND "profile belongs to a different user" — the
/// repository deliberately blurs the three so the API never leaks the
/// existence of foreign rows.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{profile_id}/libraries/{id}",
    tag = "libraries",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("id" = i64, Path, description = "Library id"),
    ),
    responses(
        (status = 200, description = "Library found", body = LibraryResponse),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 404, description = "No library with that id under the profile owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn get_library(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let repo = PostgresLibraryRepository::new(state.db.clone());
    match repo.get_for_profile(id, profile_id, user_id).await {
        Ok(Some(library)) => (StatusCode::OK, Json(LibraryResponse::from(library))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "library not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, profile_id, user_id, "get library failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "get failed").into_response()
        }
    }
}

/// Partial update via `UPDATE … RETURNING …` — every `None` field
/// is left untouched by SQL `COALESCE`. Same race-free pattern as
/// `update_profile`: a concurrent DELETE can't flip a successful
/// update into a misleading 404 because the update + read happen in
/// one round-trip. 404 when the library isn't owned by the
/// (profile_id, user_id) pair.
#[utoipa::path(
    patch,
    path = "/api/v1/profiles/{profile_id}/libraries/{id}",
    tag = "libraries",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("id" = i64, Path, description = "Library id"),
    ),
    request_body = UpdateLibraryRequest,
    responses(
        (status = 200, description = "Library updated", body = LibraryResponse),
        (status = 400, description = "`name` was supplied but is empty / whitespace-only after trim"),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 404, description = "No library with that id under the profile owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn update_library(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, id)): Path<(i64, i64)>,
    Json(req): Json<UpdateLibraryRequest>,
) -> impl IntoResponse {
    // Same boundary check as create_library: if the client did supply
    // a name, it must trim to non-empty. An omitted name (None) is
    // legitimate — the COALESCE in the repo preserves the existing
    // value — but `Some("")` / `Some("   ")` are almost always a
    // client bug and would silently blank the sidebar shelf.
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
    let patch = LibraryUpdate {
        name,
        description: req.description,
        color_id: req.color_id,
        icon_id: req.icon_id,
    };
    let now = Utc::now().timestamp_millis();
    let repo = PostgresLibraryRepository::new(state.db.clone());
    match repo
        .update_for_profile(id, &patch, now, profile_id, user_id)
        .await
    {
        Ok(Some(library)) => (StatusCode::OK, Json(LibraryResponse::from(library))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "library not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, profile_id, user_id, "update library failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "update failed").into_response()
        }
    }
}

/// Delete a library. 204 on success, 404 when the row isn't owned by
/// the (profile_id, user_id) pair (same no-leak rationale as
/// `get_library`). The schema's ON DELETE CASCADE on
/// `library.profile_id` plus the future track / folder FKs to
/// `library.id` cleans the dependents in one statement.
#[utoipa::path(
    delete,
    path = "/api/v1/profiles/{profile_id}/libraries/{id}",
    tag = "libraries",
    params(
        ("x-user-id" = i64, Header, description = "Dev shim — owning user id (replaced by JWT in 1.d)"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("id" = i64, Path, description = "Library id"),
    ),
    responses(
        (status = 204, description = "Library deleted"),
        (status = 401, description = "Missing or invalid X-User-Id"),
        (status = 404, description = "No library with that id under the profile owned by the calling user"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn delete_library(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    let repo = PostgresLibraryRepository::new(state.db.clone());
    match repo.delete_for_profile(id, profile_id, user_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "library not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, profile_id, user_id, "delete library failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}
