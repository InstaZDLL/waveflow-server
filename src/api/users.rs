//! POST /api/v1/users — dev-only user creation.
//!
//! Phase 1.b ships an `X-User-Id`-header auth shim, which is only
//! useful once the caller has *a* user id to send. This endpoint is
//! the boot-strap: anyone can hit it (no auth) to mint a fresh row
//! in the `users` table and get the assigned id back.
//!
//! Phase 1.d retires this entirely — Better Auth owns user creation
//! once JWT verification lands. The endpoint stays usable in dev
//! against a local Postgres without standing up the full auth stack.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{db, AppState};

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    /// Stable identifier from the upstream auth provider — `sub`
    /// claim of a Better Auth-issued JWT in 1.d. Trimmed and
    /// validated server-side (empty / whitespace-only is rejected
    /// 400). Optional in 1.d.1 so the dev `X-User-Id` shim path
    /// can still mint users without an upstream account; the JWT
    /// middleware (1.d.1-PR2) refuses to authenticate a row whose
    /// `external_id` is NULL.
    pub external_id: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateUserResponse {
    /// `BIGSERIAL` row id of the new user. Stash this client-side and
    /// send it in the `X-User-Id` header on subsequent calls — Better
    /// Auth-issued JWTs replace this in Phase 1.d.
    #[schema(example = 1)]
    pub id: i64,
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(create_user))
        .with_state(state)
}

/// Mint a new user row and return its id. Phase 1.d.1 widens the
/// payload to accept an optional `external_id` — the seed for the
/// upcoming JWT auth path. The dev `X-User-Id` shim can still POST
/// with an empty body and get a usable id; once Better Auth lands
/// (1.d.2) the `external_id` becomes mandatory and this endpoint
/// retires alongside the shim.
#[utoipa::path(
    post,
    path = "/api/v1/users",
    tag = "users",
    request_body = CreateUserRequest,
    responses(
        (status = 201, description = "User created", body = CreateUserResponse),
        (status = 400, description = "`external_id` supplied but empty / whitespace-only after trim"),
        (status = 409, description = "`external_id` collides with an existing users row"),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn create_user(
    State(state): State<AppState>,
    body: Option<Json<CreateUserRequest>>,
) -> impl IntoResponse {
    // axum's `Option<Json<T>>` covers both "no body" (legacy callers
    // hitting this endpoint without Content-Type) and "empty JSON"
    // — either way the resulting `CreateUserRequest` is fine since
    // every field is optional.
    let req = body.map(|Json(r)| r).unwrap_or_default();

    // Trim + validate the optional external_id at the boundary so a
    // whitespace-only payload can't slip past the UNIQUE index and
    // sit in the DB as a non-NULL-but-blank string.
    let external_id = match req.external_id {
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return (StatusCode::BAD_REQUEST, "external_id must not be blank").into_response();
            }
            Some(trimmed.to_string())
        }
        None => None,
    };

    let now = Utc::now().timestamp_millis();
    match db::users::create(&state.db, now, external_id.as_deref()).await {
        Ok(id) => (StatusCode::CREATED, Json(CreateUserResponse { id })).into_response(),
        Err(err) => {
            // Postgres unique-violation (SQLSTATE 23505) on
            // `external_id` is the "you already minted this" case
            // — distinct from a transient 500. Surface as 409 so
            // the caller can re-bootstrap rather than retry.
            if matches!(
                &err,
                sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("23505"),
            ) {
                return (
                    StatusCode::CONFLICT,
                    "external_id already taken by another user",
                )
                    .into_response();
            }
            tracing::error!(error = %err, "user insert failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to create user").into_response()
        }
    }
}
