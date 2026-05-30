//! HTTP API surface.
//!
//! The router is split per-resource so new endpoints land in their own
//! file: `health.rs` covers liveness, `ready.rs` covers DB-aware
//! readiness, `users.rs` mints dev-shim users, `profiles.rs` covers
//! tenant-scoped profile CRUD. Future modules will cover `libraries`,
//! `tracks`, `playlists`, `auth`, `sync`, `stream` (per RFC-001 §6 / §7).
//!
//! Versioning policy: every resource module mounts under `/api/v1/`
//! (except `/health` and `/ready`, which are unversioned by convention
//! — they're infrastructure probes, not part of the public API contract).
//!
//! Each module returns a `utoipa_axum::OpenApiRouter` so endpoints
//! tagged with `#[utoipa::path]` show up in the generated OpenAPI spec
//! automatically — no parallel `paths(...)` list to keep in sync.
//!
//! Auth: profiles ride behind `middleware::require_user_id`, which in
//! Phase 1.b is a dev-only `X-User-Id` header shim. Phase 1.d replaces
//! it with JWT verification against Better Auth's JWKS.

use axum::middleware;
use utoipa_axum::router::OpenApiRouter;

use crate::{middleware as auth_middleware, AppState};

mod health;
mod profiles;
mod ready;
mod users;

/// Combined router for every API module. Mounted at the root by
/// [`crate::app`]; sub-routers prefix their own paths and contribute
/// their `#[utoipa::path]` declarations to the merged OpenAPI spec.
pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        // Probes — no auth.
        .merge(health::router())
        .merge(ready::router(state.clone()))
        // Dev-shim user creation — no auth (you need an id before
        // you can send the `X-User-Id` header anywhere).
        .merge(users::router(state.clone()))
        // Tenant-scoped data routes — `require_user_id` rejects 401
        // when the header is absent or malformed, otherwise injects
        // `UserId` for the handlers.
        .merge(profiles::router(state).layer(middleware::from_fn(auth_middleware::require_user_id)))
}
