//! HTTP API surface.
//!
//! The router is split per-resource so new endpoints land in their own
//! file: `health.rs` covers liveness, future modules will cover
//! `profiles`, `libraries`, `tracks`, `playlists`, `auth`, `sync`,
//! `stream` (per RFC-001 §6 / §7).
//!
//! Versioning policy: every resource module mounts under `/api/v1/`
//! (except `/health` which is unversioned by convention — it's an
//! infrastructure probe, not part of the public API contract).

use axum::Router;

mod health;

/// Combined router for every API module. Mounted at the root by
/// [`crate::app`]; sub-routers prefix their own paths.
pub fn router() -> Router {
    Router::new().merge(health::router())
}
