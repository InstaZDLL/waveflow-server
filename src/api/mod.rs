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

use axum::{extract::Request, http::StatusCode, middleware, middleware::Next, response::Response};
use utoipa_axum::router::OpenApiRouter;

use crate::{middleware as auth_middleware, AppState, Config};

mod health;
mod profiles;
mod ready;
mod users;

/// Combined router for every API module. Mounted at the root by
/// [`crate::app`]; sub-routers prefix their own paths and contribute
/// their `#[utoipa::path]` declarations to the merged OpenAPI spec.
///
/// `/api/v1/users` and `/api/v1/profiles/*` ride behind
/// [`reject_dev_auth_disabled`] when `config.dev_auth_enabled` is
/// false (the production default). Without the gate a forged
/// `X-User-Id` header on a publicly-exposed instance would walk
/// straight into another tenant's data — Phase 1.d retires both the
/// flag and the shim together when Better Auth lands.
pub fn router(state: AppState, config: &Config) -> OpenApiRouter {
    let users_router = if config.dev_auth_enabled {
        users::router(state.clone())
    } else {
        users::router(state.clone()).layer(middleware::from_fn(reject_dev_auth_disabled))
    };

    let profiles_router = if config.dev_auth_enabled {
        profiles::router(state.clone()).layer(middleware::from_fn(auth_middleware::require_user_id))
    } else {
        profiles::router(state.clone()).layer(middleware::from_fn(reject_dev_auth_disabled))
    };

    OpenApiRouter::new()
        // Probes — no auth, no gate.
        .merge(health::router())
        .merge(ready::router(state))
        .merge(users_router)
        .merge(profiles_router)
}

/// Reject every request with **503 Service Unavailable**. Mounted on
/// `/api/v1/*` when `WAVEFLOW_DEV_AUTH` isn't `"1"` — see the rationale
/// in [`crate::Config::dev_auth_enabled`]. The 503 (vs. 401) intentionally
/// hides that the endpoints exist: a production probe can't tell
/// whether the dev shim is even compiled in.
async fn reject_dev_auth_disabled(_req: Request, _next: Next) -> Result<Response, StatusCode> {
    Err(StatusCode::SERVICE_UNAVAILABLE)
}
