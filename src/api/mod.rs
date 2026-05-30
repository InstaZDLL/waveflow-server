//! HTTP API surface.
//!
//! The router is split per-resource so new endpoints land in their own
//! file: `health.rs` covers liveness, `ready.rs` covers DB-aware
//! readiness, future modules will cover `profiles`, `libraries`,
//! `tracks`, `playlists`, `auth`, `sync`, `stream` (per RFC-001 §6 / §7).
//!
//! Versioning policy: every resource module mounts under `/api/v1/`
//! (except `/health` and `/ready`, which are unversioned by convention
//! — they're infrastructure probes, not part of the public API contract).

use axum::Router;

use crate::AppState;

mod health;
mod ready;

/// Combined router for every API module. Mounted at the root by
/// [`crate::app`]; sub-routers prefix their own paths.
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(health::router())
        .merge(ready::router(state))
}
