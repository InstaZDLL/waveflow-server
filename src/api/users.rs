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
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{db, AppState};

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

/// Mint a new user row and return its id. The endpoint takes no body
/// because the dev shim has no metadata to track — Better Auth (1.d)
/// will plug profile + email + verified flags onto the same row.
#[utoipa::path(
    post,
    path = "/api/v1/users",
    tag = "users",
    responses(
        (status = 201, description = "User created", body = CreateUserResponse),
        (status = 500, description = "Database or internal failure (body is a plain-text reason)"),
    ),
)]
async fn create_user(State(state): State<AppState>) -> impl IntoResponse {
    let now = Utc::now().timestamp_millis();
    match db::users::create(&state.db, now).await {
        Ok(id) => (StatusCode::CREATED, Json(CreateUserResponse { id })).into_response(),
        Err(err) => {
            tracing::error!(error = %err, "user insert failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "failed to create user").into_response()
        }
    }
}
