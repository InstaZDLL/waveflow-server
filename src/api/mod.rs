//! HTTP API surface.
//!
//! The router is split per-resource so new endpoints land in their own
//! file: `health.rs` covers liveness, `ready.rs` covers DB-aware
//! readiness, `profiles.rs` covers tenant-scoped profile CRUD,
//! `libraries.rs` covers tenant-scoped library CRUD nested under a
//! profile, `tracks.rs` covers tenant-scoped track CRUD nested under
//! a library, `albums.rs` + `artists.rs` cover the read-only browse
//! surface materialised by the sync apply pipeline (phase 4.d.0.4),
//! `playlists.rs` covers tenant-scoped playlist CRUD nested under a
//! profile, `sync.rs` carries the apply pipeline + WebSocket fan-out
//! (RFC-001 §6), `stream.rs` carries the HMAC-gated audio streaming
//! surface (RFC-001 §7), `artwork.rs` carries the shared artwork
//! cache (phase 1.h), `share.rs` carries the public playlist share
//! surface (phase 1.g).
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

mod albums;
mod artists;
mod artwork;
mod health;
mod libraries;
mod playlists;
mod profiles;
mod ready;
mod share;
mod stream;
mod sync;
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
    let albums_router = albums::router(state.clone()).layer(auth_layer.clone());
    let artists_router = artists::router(state.clone()).layer(auth_layer.clone());
    let playlists_router = playlists::router(state.clone()).layer(auth_layer.clone());
    let sync_router = sync::router(state.clone()).layer(auth_layer.clone());
    // Mint + revoke stay JWT-authed (verify tenant ownership before
    // mutating the share_token column). Same auth-vs-public split as
    // the streaming surface.
    let share_mint_router = share::auth_router(state.clone()).layer(auth_layer.clone());
    // Public read of a shared playlist by opaque token — no JWT
    // gate, the token IS the auth.
    let share_public_router = share::public_router(state.clone());
    // Mint stays JWT-authed (verifies tenant ownership before signing).
    let stream_mint_router = stream::auth_router(state.clone()).layer(auth_layer.clone());
    // The stream endpoint itself is HMAC-authed by the token in the
    // URL — mounting it OUTSIDE the JWT layer is what lets a browser
    // hit it from `<audio src>` without a Bearer header.
    let stream_public_router = stream::public_router(state.clone());

    // Artwork — upload stays JWT-authed (any logged-in client can
    // contribute to the shared cache); public read is hash-gated
    // (the 64-hex BLAKE3 hash IS the credential, same model as the
    // share token).
    let artwork_auth_router = artwork::auth_router(state.clone()).layer(auth_layer);
    let artwork_public_router = artwork::public_router(state.clone());

    OpenApiRouter::new()
        // Probes — no auth, no gate.
        .merge(health::router())
        .merge(ready::router(state))
        .merge(profiles_router)
        .merge(libraries_router)
        .merge(tracks_router)
        .merge(albums_router)
        .merge(artists_router)
        .merge(playlists_router)
        .merge(sync_router)
        .merge(share_mint_router)
        .merge(share_public_router)
        .merge(stream_mint_router)
        .merge(stream_public_router)
        .merge(artwork_auth_router)
        .merge(artwork_public_router)
}
