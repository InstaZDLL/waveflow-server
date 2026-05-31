//! HTTP API surface.
//!
//! The router is split per-resource so new endpoints land in their own
//! file: `health.rs` covers liveness, `ready.rs` covers DB-aware
//! readiness, `profiles.rs` covers tenant-scoped profile CRUD,
//! `libraries.rs` covers tenant-scoped library CRUD nested under a
//! profile, `tracks.rs` covers tenant-scoped track CRUD nested under
//! a library, `playlists.rs` covers tenant-scoped playlist CRUD
//! nested under a profile. Future modules will cover `sync`, `stream`
//! (per RFC-001 §6 / §7).
//!
//! Versioning policy: every resource module mounts under `/api/v1/`
//! (except `/health` and `/ready`, which are unversioned by
//! convention — they're infrastructure probes, not part of the
//! public API contract).
//!
//! Each module returns a `utoipa_axum::OpenApiRouter` so endpoints
//! tagged with `#[utoipa::path]` show up in the generated OpenAPI spec
//! automatically — no parallel `paths(...)` list to keep in sync.
//!
//! Auth: every `/api/v1/*` data route rides behind
//! [`crate::middleware::authenticate`] which requires a valid Bearer
//! JWT (Phase 1.d.2 retired the dev `X-User-Id` shim). User rows are
//! lazy-provisioned on first authenticated request via the JWT path,
//! so there's no separate user-creation endpoint to gate.

use axum::middleware;
use utoipa_axum::router::OpenApiRouter;

use crate::{middleware as auth_middleware, AppState};

mod health;
mod libraries;
mod playlists;
mod profiles;
mod ready;
mod tracks;

/// Combined router for every API module. Mounted at the root by
/// [`crate::app`]; sub-routers prefix their own paths and contribute
/// their `#[utoipa::path]` declarations to the merged OpenAPI spec.
///
/// `/api/v1/*` rides behind the unified
/// [`crate::middleware::authenticate`] layer — the only auth path
/// after Phase 1.d.2. Boot requires the `WAVEFLOW_JWT_*` triple, so
/// reaching this function with a non-functional verifier is
/// impossible.
pub fn router(state: AppState) -> OpenApiRouter {
    let auth_layer = middleware::from_fn_with_state(state.clone(), auth_middleware::authenticate);

    let profiles_router = profiles::router(state.clone()).layer(auth_layer.clone());
    let libraries_router = libraries::router(state.clone()).layer(auth_layer.clone());
    let tracks_router = tracks::router(state.clone()).layer(auth_layer.clone());
    let playlists_router = playlists::router(state.clone()).layer(auth_layer);

    OpenApiRouter::new()
        // Probes — no auth, no gate.
        .merge(health::router())
        .merge(ready::router(state))
        .merge(profiles_router)
        .merge(libraries_router)
        .merge(tracks_router)
        .merge(playlists_router)
}
