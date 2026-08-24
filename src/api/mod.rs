//! M0 HTTP surface: probes, OpenAPI and local session lifecycle.

use std::{convert::Infallible, str::FromStr, time::Duration};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Response, Sse,
    },
    routing::{get, post, put},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{authentication::AuthError, AppState};

mod access;
mod auth;
mod bookmarks;
mod catalog;
mod error;
mod favorites;
mod libraries;
mod oauth;
mod playback;
mod playlists;
mod probes;
mod setup;
mod shares;
mod sync;
mod tokens;
mod tracks;
mod users;
mod web_session;

pub(crate) use access::*;
pub use auth::*;
pub use bookmarks::*;
pub use catalog::*;
pub use error::*;
pub use favorites::*;
pub use libraries::*;
pub use oauth::*;
pub use playback::*;
pub use playlists::*;
pub use probes::*;
pub use setup::*;
pub use shares::*;
pub use sync::*;
pub use tokens::*;
pub use tracks::*;
pub use users::*;
pub use web_session::*;

const WEB_REFRESH_COOKIE: &str = "waveflow-refresh";

const WEB_CSRF_COOKIE: &str = "waveflow-csrf";

pub const WEB_CSRF_HEADER: &str = "x-waveflow-csrf";

pub const OPERATION_ID_HEADER: &str = "x-waveflow-operation-id";

pub const DEVICE_ID_HEADER: &str = "x-waveflow-device-id";

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/v2/setup", get(setup_status).post(setup))
        .route("/api/v2/auth/login", post(login))
        .route("/api/v2/auth/refresh", post(refresh))
        .route("/api/v2/auth/logout", post(logout))
        .route("/api/v2/web/auth/login", post(web_login))
        .route("/api/v2/web/auth/refresh", post(web_refresh))
        .route("/api/v2/web/auth/logout", post(web_logout))
        .route("/api/v2/oauth/authorize", post(oauth_authorize))
        // No auth layer: the code plus its PKCE verifier are the credential.
        .route("/api/v2/oauth/token", post(oauth_token))
        .route("/api/v2/libraries/{library_id}/scans", post(start_scan))
        .route(
            "/api/v2/libraries",
            get(list_libraries).post(create_library),
        )
        .route(
            "/api/v2/libraries/{library_id}/members/{user_id}",
            put(set_library_member).delete(remove_library_member),
        )
        .route("/api/v2/scans/{scan_id}", get(scan_status))
        .route("/api/v2/scans/{scan_id}/events", get(scan_events))
        .route("/api/v2/libraries/{library_id}/tracks", get(list_tracks))
        .route("/api/v2/tracks/{track_id}", get(get_track))
        .route("/api/v2/tracks/{track_id}/lyrics", get(get_track_lyrics))
        .route("/api/v2/albums", get(list_albums))
        .route("/api/v2/genres", get(list_genres))
        .route("/api/v2/albums/{album_id}", get(get_album))
        .route("/api/v2/artists", get(list_artists))
        .route("/api/v2/artists/{artist_id}", get(get_artist))
        .route("/api/v2/search", get(search_catalog))
        .route("/api/v2/songs", get(list_songs_by_genre))
        .route("/api/v2/songs/random", get(list_random_songs))
        .route(
            "/api/v2/playlists",
            get(list_playlists).post(create_playlist),
        )
        .route(
            "/api/v2/playlists/{playlist_id}",
            get(get_playlist)
                .patch(update_playlist)
                .delete(delete_playlist),
        )
        .route("/api/v2/favorites", get(list_favorites))
        .route(
            "/api/v2/favorites/{entity_type}/{entity_id}",
            put(add_favorite).delete(remove_favorite),
        )
        .route("/api/v2/ratings/{entity_type}/{entity_id}", put(set_rating))
        .route("/api/v2/ratings", get(list_ratings))
        .route("/api/v2/bookmarks", get(list_bookmarks))
        .route(
            "/api/v2/bookmarks/{track_id}",
            put(set_bookmark).delete(delete_bookmark),
        )
        .route("/api/v2/scrobbles", post(create_scrobble))
        .route("/api/v2/history", get(list_history))
        .route("/api/v2/now-playing", get(list_now_playing))
        .route("/api/v2/queue", get(get_queue).put(save_queue))
        .route("/api/v2/shares", get(list_shares).post(create_share))
        .route(
            "/api/v2/shares/{share_id}",
            axum::routing::patch(update_share).delete(delete_share),
        )
        .route("/api/v2/sync/changes", get(sync_changes))
        .route("/api/v2/sync/snapshot", get(sync_snapshot))
        .route("/api/v2/sync/ack", put(sync_ack))
        .route("/api/v2/sync/socket", get(sync_socket))
        .route("/api/v2/transcode/status", get(transcode_status))
        .route("/api/v2/admin/users", get(list_users).post(create_user))
        .route(
            "/api/v2/admin/users/{username}",
            axum::routing::patch(update_user).delete(delete_user),
        )
        .route(
            "/api/v2/admin/users/{username}/subsonic-credential",
            put(set_subsonic_credential).delete(revoke_subsonic_credential),
        )
        .route(
            "/api/v2/admin/users/{username}/tokens",
            get(list_api_tokens).post(create_api_token),
        )
        .route(
            "/api/v2/admin/users/{username}/tokens/{token_id}",
            axum::routing::delete(revoke_api_token),
        )
        .with_state(state)
}
