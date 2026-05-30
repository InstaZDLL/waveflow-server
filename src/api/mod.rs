//! HTTP API surface.
//!
//! The router is split per-resource so new endpoints land in their own
//! file: `health.rs` covers liveness, `ready.rs` covers DB-aware
//! readiness, `users.rs` mints dev-shim users, `profiles.rs` covers
//! tenant-scoped profile CRUD, `libraries.rs` covers tenant-scoped
//! library CRUD nested under a profile, `tracks.rs` covers
//! tenant-scoped track CRUD nested under a library, `playlists.rs`
//! covers tenant-scoped playlist CRUD nested under a profile (same
//! depth as library). Future modules will cover `auth`, `sync`,
//! `stream` (per RFC-001 §6 / §7).
//!
//! Versioning policy: every resource module mounts under `/api/v1/`
//! (except `/health` and `/ready`, which are unversioned by convention
//! — they're infrastructure probes, not part of the public API contract).
//!
//! Each module returns a `utoipa_axum::OpenApiRouter` so endpoints
//! tagged with `#[utoipa::path]` show up in the generated OpenAPI spec
//! automatically — no parallel `paths(...)` list to keep in sync.
//!
//! Auth: every `/api/v1/profiles/*` (and its nested resources)
//! rides behind [`crate::middleware::authenticate`], which tries
//! JWT verification first and falls back to the dev `X-User-Id`
//! shim. Phase 1.d.2 retires the shim once Better Auth is the only
//! configured auth path. `/api/v1/users` stays open when the dev
//! shim is enabled (it's the test/bootstrap user-mint path) and
//! 503's otherwise.

use axum::{extract::Request, http::StatusCode, middleware, middleware::Next, response::Response};
use utoipa_axum::router::OpenApiRouter;

use crate::{middleware as auth_middleware, AppState, Config};

mod health;
mod libraries;
mod playlists;
mod profiles;
mod ready;
mod tracks;
mod users;

/// Combined router for every API module. Mounted at the root by
/// [`crate::app`]; sub-routers prefix their own paths and contribute
/// their `#[utoipa::path]` declarations to the merged OpenAPI spec.
///
/// `/api/v1/profiles/*`, its nested resources, and
/// `/api/v1/profiles/{profile_id}/playlists/*` all ride behind the
/// unified [`crate::middleware::authenticate`] layer. That layer
/// short-circuits to **503** when neither auth path is configured —
/// see [`Config::auth_disabled_at_boot`]. Without that gate a forged
/// `X-User-Id` header on a publicly-exposed instance would walk
/// straight into another tenant's data.
///
/// `/api/v1/users` stays open when [`Config::dev_auth_enabled`] is
/// true (it's the test/bootstrap user-mint path) and answers **503**
/// otherwise. The JWT path doesn't gate it because production
/// onboarding happens at Better Auth, not at this endpoint —
/// Phase 1.d.2 will retire it alongside the shim.
pub fn router(state: AppState, config: &Config) -> OpenApiRouter {
    let users_router = if config.dev_auth_enabled {
        users::router(state.clone())
    } else {
        users::router(state.clone()).layer(middleware::from_fn(reject_dev_auth_disabled))
    };

    // Single auth layer shared across every tenant-scoped resource
    // — replaces the per-resource fork between `require_user_id`
    // and `reject_dev_auth_disabled` that pre-PR3 mod.rs carried.
    // The middleware reads `state.jwt_verifier` + `dev_auth_enabled`
    // and decides per request.
    let auth_layer = middleware::from_fn_with_state(state.clone(), auth_middleware::authenticate);

    let profiles_router = profiles::router(state.clone()).layer(auth_layer.clone());
    let libraries_router = libraries::router(state.clone()).layer(auth_layer.clone());
    let tracks_router = tracks::router(state.clone()).layer(auth_layer.clone());
    let playlists_router = playlists::router(state.clone()).layer(auth_layer);

    OpenApiRouter::new()
        // Probes — no auth, no gate.
        .merge(health::router())
        .merge(ready::router(state))
        .merge(users_router)
        .merge(profiles_router)
        .merge(libraries_router)
        .merge(tracks_router)
        .merge(playlists_router)
}

/// Reject every request with **503 Service Unavailable**. Mounted on
/// `/api/v1/*` when `WAVEFLOW_DEV_AUTH` isn't `"1"` — see the rationale
/// in [`crate::Config::dev_auth_enabled`]. The 503 (vs. 401) intentionally
/// hides that the endpoints exist: a production probe can't tell
/// whether the dev shim is even compiled in.
async fn reject_dev_auth_disabled(_req: Request, _next: Next) -> Result<Response, StatusCode> {
    Err(StatusCode::SERVICE_UNAVAILABLE)
}
