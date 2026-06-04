//! `/api/v1/share/*` — public share-link surface per Phase 1.g.
//!
//! Five routes split across the same auth-vs-public boundary the
//! streaming module uses:
//!
//! - **Mint** (`POST /api/v1/profiles/{profile_id}/playlists/{playlist_id}/share`):
//!   JWT-authed. Verifies tenant ownership of the playlist, then
//!   atomically generates (or returns the existing) opaque token via
//!   `db::share::mint_or_get_token`. Idempotent — a second call for
//!   the same playlist returns the same token rather than rotating
//!   it.
//! - **Revoke** (`DELETE /api/v1/profiles/{profile_id}/playlists/{playlist_id}/share`):
//!   JWT-authed. Sets `share_token = NULL`, instantly closing any
//!   public URL pointing at this playlist.
//! - **Mint by canonical**
//!   (`POST /api/v1/share/playlists/by-canonical/{profile_canonical_id}/{playlist_canonical_id}`):
//!   JWT-authed. Same semantics as `mint`, but keyed on the desktop's
//!   canonical UUIDs (Phase 1.g.0). The desktop never sees the
//!   server-side BIGSERIAL ids the apply pipeline assigns, so this
//!   variant skips the lookup round-trip the desktop would otherwise
//!   need to translate canonical → server id before calling the
//!   classic endpoint.
//! - **Revoke by canonical**
//!   (`DELETE /api/v1/share/playlists/by-canonical/{profile_canonical_id}/{playlist_canonical_id}`):
//!   Mirror of revoke for the canonical-id surface.
//! - **Public read** (`GET /api/v1/share/playlists/{token}`): NOT
//!   behind the JWT middleware — the token IS the auth. A miss (no
//!   row matches the token) returns 404 with no body so an attacker
//!   can't distinguish "revoked" from "never minted".
//!
//! Wire format intentionally minimal: name + description + cover +
//! brand tokens + timestamps. Track list is NOT returned today
//! because the server doesn't materialise `playlist_track` yet
//! (the desktop is still the source of truth for the join). When
//! Phase 1.g.2 brings server-side materialisation, this DTO grows a
//! `tracks: Vec<PublicTrack>` field without a wire-break.

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{middleware::UserId, AppState};

/// Body of the mint response. `token` is the opaque ID the web
/// client uses for the public route (`/p/<token>` on
/// waveflow-web). The server does NOT return the full URL because
/// it doesn't know the web origin — clients combine this token
/// with their persisted `app_setting['app.waveflow_web_url']` to
/// build the shareable link.
#[derive(Debug, Serialize, ToSchema)]
pub struct MintResponse {
    /// Opaque URL-safe token (32 alphanumeric chars). Stable for
    /// the lifetime of the link — a second mint call returns the
    /// same value rather than rotating.
    pub token: String,
}

/// Public preview of a shared playlist. Mirrors the desktop's
/// `Playlist` DTO minus the `profile_id` (the share owner isn't
/// exposed to anonymous viewers) and minus the smart-playlist
/// machinery (smart playlists can't be shared — they materialise
/// per-device).
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicPlaylistResponse {
    pub id: i64,
    pub name: String,
    pub description: Option<String>,
    pub color_id: String,
    pub icon_id: String,
    /// BLAKE3 hash of the cover image in the shared metadata cache.
    /// `None` until the artwork pipeline ships on the server.
    pub cover_hash: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Track list. Always empty today — server-side
    /// `playlist_track` materialisation is Phase 1.g.2. The field
    /// is present so a future server release can populate it
    /// without breaking the wire shape.
    pub tracks: Vec<PublicTrack>,
}

/// Placeholder shape for the track list — kept minimal so the
/// initial wire contract doesn't pre-commit to a richer DTO that
/// would need re-negotiating once the join lands server-side.
#[derive(Debug, Serialize, ToSchema)]
pub struct PublicTrack {
    pub title: String,
    pub artist: Option<String>,
    pub duration_ms: i64,
}

pub fn auth_router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(mint_share_token))
        .routes(routes!(revoke_share_token))
        .routes(routes!(mint_share_token_by_canonical))
        .routes(routes!(revoke_share_token_by_canonical))
        .with_state(state)
}

pub fn public_router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_public_playlist))
        .with_state(state)
}

#[utoipa::path(
    post,
    path = "/api/v1/profiles/{profile_id}/playlists/{playlist_id}/share",
    tag = "share",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("playlist_id" = i64, Path, description = "Playlist to publish"),
    ),
    responses(
        (status = 200, description = "Token minted or echoed back if one already existed", body = MintResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Playlist not found in the requested profile or not owned by the caller"),
        (status = 500, description = "Database or internal failure"),
    ),
)]
async fn mint_share_token(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, playlist_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    match crate::db::share::mint_or_get_token(&state.db, user_id, profile_id, playlist_id).await {
        Ok(Some(token)) => (StatusCode::OK, Json(MintResponse { token })).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "playlist not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, user_id, profile_id, playlist_id, "share mint failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "mint failed").into_response()
        }
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/profiles/{profile_id}/playlists/{playlist_id}/share",
    tag = "share",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("playlist_id" = i64, Path, description = "Playlist whose link should be closed"),
    ),
    responses(
        (status = 204, description = "Token cleared (idempotent — also returned if the link was already private)"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Playlist not found in the requested profile or not owned by the caller"),
        (status = 500, description = "Database or internal failure"),
    ),
)]
async fn revoke_share_token(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, playlist_id)): Path<(i64, i64)>,
) -> impl IntoResponse {
    match crate::db::share::revoke_token(&state.db, user_id, profile_id, playlist_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "playlist not found").into_response(),
        Err(err) => {
            tracing::error!(error = %err, user_id, profile_id, playlist_id, "share revoke failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "revoke failed").into_response()
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/share/playlists/by-canonical/{profile_canonical_id}/{playlist_canonical_id}",
    tag = "share",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_canonical_id" = String, Path, description = "Desktop profile's canonical UUID"),
        ("playlist_canonical_id" = String, Path, description = "Desktop playlist's canonical UUID"),
    ),
    responses(
        (status = 200, description = "Token minted or echoed back", body = MintResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Profile or playlist not found (or not owned by the caller)"),
        (status = 500, description = "Database or internal failure"),
    ),
)]
async fn mint_share_token_by_canonical(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_canonical_id, playlist_canonical_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match crate::db::share::mint_or_get_token_by_canonical(
        &state.db,
        user_id,
        &profile_canonical_id,
        &playlist_canonical_id,
    )
    .await
    {
        Ok(Some(token)) => (StatusCode::OK, Json(MintResponse { token })).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "playlist not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                user_id,
                profile_canonical_id = %profile_canonical_id,
                playlist_canonical_id = %playlist_canonical_id,
                "share mint (by canonical) failed",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "mint failed").into_response()
        }
    }
}

#[utoipa::path(
    delete,
    path = "/api/v1/share/playlists/by-canonical/{profile_canonical_id}/{playlist_canonical_id}",
    tag = "share",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_canonical_id" = String, Path, description = "Desktop profile's canonical UUID"),
        ("playlist_canonical_id" = String, Path, description = "Desktop playlist's canonical UUID"),
    ),
    responses(
        (status = 204, description = "Token cleared (idempotent)"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Profile or playlist not found (or not owned by the caller)"),
        (status = 500, description = "Database or internal failure"),
    ),
)]
async fn revoke_share_token_by_canonical(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_canonical_id, playlist_canonical_id)): Path<(String, String)>,
) -> impl IntoResponse {
    match crate::db::share::revoke_token_by_canonical(
        &state.db,
        user_id,
        &profile_canonical_id,
        &playlist_canonical_id,
    )
    .await
    {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "playlist not found").into_response(),
        Err(err) => {
            tracing::error!(
                error = %err,
                user_id,
                profile_canonical_id = %profile_canonical_id,
                playlist_canonical_id = %playlist_canonical_id,
                "share revoke (by canonical) failed",
            );
            (StatusCode::INTERNAL_SERVER_ERROR, "revoke failed").into_response()
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/share/playlists/{token}",
    tag = "share",
    params(
        ("token" = String, Path, description = "Opaque share token minted by POST /share"),
    ),
    responses(
        (status = 200, description = "Public preview of the shared playlist", body = PublicPlaylistResponse),
        (status = 404, description = "Token unknown (never minted or revoked)"),
        (status = 500, description = "Database or internal failure"),
    ),
)]
async fn get_public_playlist(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> impl IntoResponse {
    match crate::db::share::fetch_public_by_token(&state.db, &token).await {
        Ok(Some((
            id,
            name,
            description,
            color_id,
            icon_id,
            cover_hash,
            created_at,
            updated_at,
        ))) => (
            StatusCode::OK,
            Json(PublicPlaylistResponse {
                id,
                name,
                description,
                color_id,
                icon_id,
                cover_hash,
                created_at,
                updated_at,
                tracks: Vec::new(),
            }),
        )
            .into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => {
            tracing::error!(error = %err, "share public lookup failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "lookup failed").into_response()
        }
    }
}
