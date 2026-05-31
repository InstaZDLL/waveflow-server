//! `/api/v1/profiles/*` — tenant-scoped CRUD over the `profile` table.
//!
//! Every handler reads the owning user id from the [`UserId`]
//! extension that `middleware::authenticate` attached to the
//! request, and dispatches to a `*_for_user` method on
//! [`PostgresProfileRepository`]. The trait surface from
//! `waveflow-core` is *not* used here — it has no notion of tenancy
//! and would let a careless query leak data across users.

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
    domain::profile::Profile,
    repository::{
        postgres::PostgresProfileRepository,
        profile::{ProfileDeleteOutcome, ProfileDraft},
    },
};

use crate::{middleware::UserId, AppState};

/// Wire-format profile. Drops `data_dir` from the public response —
/// the column exists on the row (it'd be the desktop's on-disk dir)
/// but the server has no per-profile filesystem state, so always-empty
/// noise on the wire would just confuse the web client.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProfileResponse {
    pub id: i64,
    pub name: String,
    pub color_id: String,
    pub avatar_hash: Option<String>,
    pub created_at: i64,
    pub last_used_at: i64,
}

impl From<Profile> for ProfileResponse {
    fn from(p: Profile) -> Self {
        Self {
            id: p.id,
            name: p.name,
            color_id: p.color_id,
            avatar_hash: p.avatar_hash,
            created_at: p.created_at,
            last_used_at: p.last_used_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProfileRequest {
    /// Display name shown in the profile picker.
    pub name: String,
    /// Brand-defined colour token (one of the design system's
    /// avatar-background palette). Validated by the front-end, the
    /// server just stores the string.
    pub color_id: String,
    /// BLAKE3 hash of the avatar PNG, if any. `None` falls back to
    /// the default initial-on-colour avatar.
    pub avatar_hash: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProfileRequest {
    /// New display name. Only field that's mutable in 1.b.4 —
    /// avatar / color updates land in a follow-up.
    pub name: String,
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(list_profiles, create_profile))
        .routes(routes!(get_profile, update_profile, delete_profile))
        .with_state(state)
}

/// List the calling user's profiles, MRU-first.
#[utoipa::path(
    get,
    path = "/api/v1/profiles",
    tag = "profiles",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
    ),
    responses(
        (status = 200, description = "Owned profiles, most-recently-used first", body = Vec<ProfileResponse>),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn list_profiles(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
) -> impl IntoResponse {
    let repo = PostgresProfileRepository::new(state.db.clone());
    match repo.list_for_user(user_id).await {
        Ok(profiles) => {
            let body: Vec<ProfileResponse> = profiles.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, user_id, "list profiles failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "list failed").into_response()
        }
    }
}

/// Create a profile owned by the calling user. Returns 409 when the
/// FK rejects — the request carried a user id that no longer exists.
#[utoipa::path(
    post,
    path = "/api/v1/profiles",
    tag = "profiles",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
    ),
    request_body = CreateProfileRequest,
    responses(
        (status = 201, description = "Profile created", body = ProfileResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
        (status = 409, description = "X-User-Id does not match an existing users row"),
    ),
)]
async fn create_profile(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Json(req): Json<CreateProfileRequest>,
) -> impl IntoResponse {
    let now = Utc::now().timestamp_millis();
    let draft = ProfileDraft {
        name: req.name,
        color_id: req.color_id,
        avatar_hash: req.avatar_hash,
        now_ms: now,
    };
    let repo = PostgresProfileRepository::new(state.db.clone());
    let id = match repo.insert_for_user(&draft, user_id).await {
        Ok(id) => id,
        Err(err) => {
            // sqlx::Error::Database with code 23503 is the FK
            // violation — the X-User-Id header carried a user that
            // doesn't exist (or was deleted between the middleware
            // check and the insert). Surface that distinctly so the
            // client can re-bootstrap a user.
            if matches!(
                &err,
                waveflow_core::error::CoreError::Database(sqlx::Error::Database(db_err))
                    if db_err.code().as_deref() == Some("23503"),
            ) {
                return (StatusCode::CONFLICT, "X-User-Id has no matching user row")
                    .into_response();
            }
            tracing::error!(error = %err, user_id, "create profile failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "create failed").into_response();
        }
    };

    // Read back the freshly inserted row so the response carries
    // server-assigned fields (id, created_at, last_used_at, the
    // canonical `last_used_at` after sqlx round-trip).
    match repo.get_for_user(id, user_id).await {
        Ok(Some(profile)) => {
            (StatusCode::CREATED, Json(ProfileResponse::from(profile))).into_response()
        }
        Ok(None) => {
            // Should never happen — we just inserted with the same
            // user_id. If it does, treat as a server bug.
            tracing::error!(id, user_id, "freshly created profile not found");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "read-after-create failed",
            )
                .into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, id, user_id, "read-after-create failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "read-after-create failed",
            )
                .into_response()
        }
    }
}

/// Fetch one profile by id, scoped to the calling user. 404 covers
/// both "no such id" and "id belongs to another user" — the
/// repository deliberately blurs the two so we don't leak the
/// existence of other tenants' rows.
#[utoipa::path(
    get,
    path = "/api/v1/profiles/{id}",
    tag = "profiles",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("id" = i64, Path, description = "Profile id"),
    ),
    responses(
        (status = 200, description = "Profile found", body = ProfileResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
        (status = 404, description = "No profile with that id owned by the calling user"),
    ),
)]
async fn get_profile(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let repo = PostgresProfileRepository::new(state.db.clone());
    match repo.get_for_user(id, user_id).await {
        Ok(Some(profile)) => (StatusCode::OK, Json(ProfileResponse::from(profile))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "profile not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, user_id, "get profile failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "get failed").into_response()
        }
    }
}

/// Rename a profile. 404 when the id isn't owned by the caller, same
/// no-data-leak rationale as `get_profile`.
#[utoipa::path(
    patch,
    path = "/api/v1/profiles/{id}",
    tag = "profiles",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("id" = i64, Path, description = "Profile id"),
    ),
    request_body = UpdateProfileRequest,
    responses(
        (status = 200, description = "Profile renamed", body = ProfileResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
        (status = 404, description = "No profile with that id owned by the calling user"),
    ),
)]
async fn update_profile(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path(id): Path<i64>,
    Json(req): Json<UpdateProfileRequest>,
) -> impl IntoResponse {
    // `rename_for_user` now hands back the updated row via
    // `UPDATE … RETURNING …` in one round-trip — no separate
    // read-back, so a concurrent DELETE can no longer flip a
    // successful rename into a misleading 404.
    let repo = PostgresProfileRepository::new(state.db.clone());
    match repo.rename_for_user(id, &req.name, user_id).await {
        Ok(Some(profile)) => (StatusCode::OK, Json(ProfileResponse::from(profile))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "profile not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, user_id, "rename profile failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "rename failed").into_response()
        }
    }
}

/// Delete a profile. 409 when the row was the user's last remaining
/// profile (the storage layer refuses to leave the user with zero
/// profiles — same invariant the desktop's `ProfileSelectorModal`
/// enforces client-side).
#[utoipa::path(
    delete,
    path = "/api/v1/profiles/{id}",
    tag = "profiles",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("id" = i64, Path, description = "Profile id"),
    ),
    responses(
        (status = 204, description = "Profile deleted"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
        (status = 404, description = "No profile with that id owned by the calling user"),
        (status = 409, description = "Refused — would leave the user with zero profiles"),
    ),
)]
async fn delete_profile(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let repo = PostgresProfileRepository::new(state.db.clone());
    match repo.delete_guarded_for_user(id, user_id).await {
        Ok(ProfileDeleteOutcome::Deleted) => StatusCode::NO_CONTENT.into_response(),
        Ok(ProfileDeleteOutcome::NotFound) => {
            (StatusCode::NOT_FOUND, "profile not found").into_response()
        }
        Ok(ProfileDeleteOutcome::WasLast) => (
            StatusCode::CONFLICT,
            "cannot delete the user's last remaining profile",
        )
            .into_response(),
        Err(err) => {
            tracing::error!(error = %err, id, user_id, "delete profile failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}
