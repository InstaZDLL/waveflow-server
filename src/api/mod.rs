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
    routing::{delete, get, post, put},
    Json, Router,
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{authentication::AuthError, AppState};

pub(crate) mod access;
mod attention;
mod auth;
mod bookmarks;
mod catalog;
mod discovery;
mod error;
mod favorites;
mod folders;
mod libraries;
mod oauth;
mod playback;
mod playlist_moves;
mod playlists;
mod podcasts;
mod probes;
mod quality;
mod radio;
mod scrobbling;
mod setup;
mod shares;
mod smart_playlists;
mod subsonic_credential;
mod sync;
mod tokens;
mod tracks;
mod uploads;
mod users;
mod web_session;

pub(crate) use access::*;
pub use attention::*;
pub use auth::*;
pub use bookmarks::*;
pub use catalog::*;
pub use discovery::*;
pub use error::*;
pub use favorites::*;
pub use folders::*;
pub use libraries::*;
pub use oauth::*;
pub use playback::*;
pub use playlist_moves::*;
pub use playlists::*;
pub use podcasts::*;
pub use probes::*;
pub use quality::*;
pub use radio::*;
pub use scrobbling::*;
pub use setup::*;
pub use shares::*;
pub use smart_playlists::*;
pub use subsonic_credential::*;
pub use sync::*;
pub use tokens::*;
pub use tracks::*;
pub use uploads::*;
pub use users::*;
pub use web_session::*;

const WEB_REFRESH_COOKIE: &str = "waveflow-refresh";

const WEB_CSRF_COOKIE: &str = "waveflow-csrf";

pub const WEB_CSRF_HEADER: &str = "x-waveflow-csrf";

pub const OPERATION_ID_HEADER: &str = "x-waveflow-operation-id";

pub const DEVICE_ID_HEADER: &str = "x-waveflow-device-id";

pub fn router(state: AppState) -> Router {
    // Read before the state is moved into the router: the ceiling follows the
    // batch limit, so the two cannot disagree.
    let negotiation_body_limit = state.upload_batch_body_limit;
    let chunk_body_limit = state.upload_chunk_body_limit;
    let playlist_body_limit = crate::services::playlist_import_body_limit();
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
            "/api/v2/libraries/{library_id}",
            axum::routing::patch(update_library).delete(delete_library),
        )
        .route(
            "/api/v2/libraries/{library_id}/settings",
            get(library_settings),
        )
        .route(
            "/api/v2/libraries/{library_id}/unavailable-tracks",
            get(unavailable_tracks),
        )
        .route(
            "/api/v2/libraries/{library_id}/unavailable-tracks/purge",
            post(purge_unavailable_tracks),
        )
        .route(
            "/api/v2/libraries/{library_id}/purged-tracks",
            get(purged_tracks),
        )
        .route(
            "/api/v2/libraries/{library_id}/purged-tracks/restore",
            post(restore_purged_tracks),
        )
        .route(
            "/api/v2/libraries/{library_id}/members",
            get(list_library_members),
        )
        .route(
            "/api/v2/libraries/{library_id}/members/{user_id}",
            put(set_library_member).delete(remove_library_member),
        )
        .route("/api/v2/scans/{scan_id}", get(scan_status))
        .route("/api/v2/scans/{scan_id}/events", get(scan_events))
        .route("/api/v2/libraries/{library_id}/tracks", get(list_tracks))
        .route("/api/v2/libraries/{library_id}/tree", get(library_tree))
        .route(
            "/api/v2/libraries/{library_id}/folders",
            get(list_folders).post(create_folder),
        )
        .route(
            "/api/v2/libraries/{library_id}/folders/{folder}/tracks",
            get(folder_tracks),
        )
        .route(
            "/api/v2/libraries/{library_id}/placements",
            put(place_tracks).layer(axum::extract::DefaultBodyLimit::max(
                folders::placement_body_limit(),
            )),
        )
        .route(
            "/api/v2/folders/{folder_id}",
            axum::routing::patch(update_folder).delete(delete_folder),
        )
        .route(
            "/api/v2/libraries/{library_id}/attention",
            get(attention_summary),
        )
        .route(
            "/api/v2/libraries/{library_id}/attention/credits",
            get(attention_credits),
        )
        .route(
            "/api/v2/libraries/{library_id}/attention/tracks",
            get(attention_tracks),
        )
        .route(
            "/api/v2/libraries/{library_id}/credit-decisions",
            get(list_credit_decisions).post(create_credit_decision),
        )
        .route(
            "/api/v2/libraries/{library_id}/credit-decisions/{decision_id}",
            delete(delete_credit_decision),
        )
        .route(
            "/api/v2/libraries/{library_id}/quality",
            get(quality_summary),
        )
        .route(
            "/api/v2/libraries/{library_id}/quality/tracks",
            get(quality_tracks),
        )
        .route(
            "/api/v2/libraries/{library_id}/quality/analysis",
            post(request_quality_analysis),
        )
        .route("/api/v2/tracks/{track_id}/quality", get(track_quality))
        .route(
            "/api/v2/tracks/{track_id}/spectrogram",
            get(track_spectrogram),
        )
        .route("/api/v2/libraries/{library_id}/events", get(library_events))
        .route(
            "/api/v2/libraries/{library_id}/events/ack",
            put(library_events_ack),
        )
        // Its own body ceiling, and only its own. Raising the router's would
        // hand every route on the server a surface none of them asked for.
        .route(
            "/api/v2/libraries/{library_id}/uploads",
            post(negotiate_uploads)
                .layer(axum::extract::DefaultBodyLimit::max(negotiation_body_limit)),
        )
        .route("/api/v2/uploads/{session_id}", get(upload_session))
        // The one route on the server that carries a payload worth megabytes,
        // and the only one that asks for the room.
        .route(
            "/api/v2/uploads/{session_id}/chunks/{index}",
            put(upload_chunk).layer(axum::extract::DefaultBodyLimit::max(chunk_body_limit)),
        )
        .route("/api/v2/uploads/{session_id}/commit", post(commit_upload))
        .route(
            "/api/v2/tracks/{track_id}",
            get(get_track).patch(update_track),
        )
        .route("/api/v2/tracks/{track_id}/lyrics", get(get_track_lyrics))
        // A sheet is text, but a mebibyte of it: the one route besides the
        // upload chunks that asks for more than the global 16 KiB.
        .route(
            "/api/v2/tracks/{track_id}/lyrics/{format}",
            put(put_track_lyrics).delete(delete_track_lyrics).layer(
                axum::extract::DefaultBodyLimit::max(crate::services::MAX_LYRICS_UPLOAD_BYTES),
            ),
        )
        .route(
            "/api/v2/tracks/{track_id}/lyrics/file",
            get(get_track_lyrics_file),
        )
        .route(
            "/api/v2/tracks/{track_id}/overrides",
            get(get_track_overrides),
        )
        .route("/api/v2/albums", get(list_albums))
        .route("/api/v2/genres", get(list_genres))
        .route("/api/v2/albums/{album_id}", get(get_album))
        .route("/api/v2/artists", get(list_artists))
        .route("/api/v2/artists/{artist_id}", get(get_artist))
        .route(
            "/api/v2/artists/{artist_id}/image/{kind}",
            put(put_artist_image).delete(delete_artist_image).layer(
                axum::extract::DefaultBodyLimit::max(crate::portrait::MAX_IMAGE_BYTES),
            ),
        )
        .route(
            "/api/v2/artists/{artist_id}/image/portrait/deezer",
            put(put_artist_deezer_portrait),
        )
        .route(
            "/api/v2/artists/{artist_id}/top-songs",
            get(list_artist_top_songs),
        )
        .route(
            "/api/v2/artists/{artist_id}/albums",
            get(list_artist_albums),
        )
        .route("/api/v2/search", get(search_catalog))
        .route("/api/v2/songs", get(list_songs))
        .route("/api/v2/songs/by-genre", get(list_songs_by_genre))
        .route("/api/v2/songs/random", get(list_random_songs))
        // RFC-024: what the history and the catalogue suggest.
        .route("/api/v2/stats", get(get_listening_stats))
        .route("/api/v2/mixes", get(list_mixes))
        .route("/api/v2/mixes/{mix_id}", get(get_mix))
        .route("/api/v2/similar-songs/{id}", get(list_similar_songs))
        .route(
            "/api/v2/playlists",
            get(list_playlists).post(create_playlist),
        )
        // Its own ceiling, derived from the playlist ceiling rather than
        // configured: a file holding the most tracks a playlist may hold must
        // fit, and raising that limit must not quietly start refusing the files
        // it just permitted.
        .route(
            "/api/v2/playlists/import",
            post(import_playlist).layer(axum::extract::DefaultBodyLimit::max(playlist_body_limit)),
        )
        .route(
            "/api/v2/playlists/{playlist_id}/export",
            get(export_playlist),
        )
        .route(
            "/api/v2/playlists/{playlist_id}/move",
            post(move_playlist_entry),
        )
        .route(
            "/api/v2/playlists/{playlist_id}/remove",
            post(remove_playlist_entry),
        )
        .route(
            "/api/v2/playlists/{playlist_id}",
            get(get_playlist)
                .patch(update_playlist)
                .delete(delete_playlist),
        )
        .route(
            "/api/v2/api-tokens",
            get(list_own_api_tokens).post(create_own_api_token),
        )
        .route(
            "/api/v2/api-tokens/{token_id}",
            delete(revoke_own_api_token),
        )
        // Self-scoped, beside the API tokens and for the same reason: the
        // credential belongs to the account, so answering for it does not need
        // the instance. No POST — creating one where none exists opens the
        // facade to an account, which is the operator's decision.
        .route(
            "/api/v2/subsonic-credential",
            get(own_subsonic_credential)
                .put(rotate_own_subsonic_credential)
                .delete(revoke_own_subsonic_credential),
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
        // One list per account, and `PUT` on the station because the protocol's
        // update carries every field — see `src/api/radio.rs`.
        .route(
            "/api/v2/radio-stations",
            get(list_radio_stations).post(create_radio_station),
        )
        .route(
            "/api/v2/radio-stations/{station_id}",
            get(get_radio_station)
                .put(replace_radio_station)
                .delete(delete_radio_station),
        )
        .route("/api/v2/scrobbles", post(create_scrobble))
        .route("/api/v2/history", get(list_history))
        .route("/api/v2/now-playing", get(list_now_playing))
        .route("/api/v2/scrobble-links", get(list_scrobble_links))
        .route(
            "/api/v2/scrobble-destinations",
            get(list_scrobble_destinations),
        )
        // The instance is in the path, not implied. `/{provider}` named nothing
        // precise the moment a recipient could have two, and `DELETE` has no
        // body to name one in. No implicit default either: a path without an
        // instance is refused rather than attached to whichever came first —
        // that slide is the substitution decision 4 exists to prevent.
        .route(
            "/api/v2/scrobble-links/{provider}/{destination}",
            put(link_scrobble).delete(unlink_scrobble),
        )
        // The literal before the parameter, so a destination named `callback`
        // cannot produce a path the return route claims as well.
        .route(
            "/api/v2/scrobble-links/lastfm/authorize/{destination}",
            post(authorize_lastfm),
        )
        // **Both spellings, and that is not a matter of tidiness.** Last.fm
        // documents that it appends `/?token=…` to the callback, so what the
        // browser asks for is `…/callback/{state}/` and not
        // `…/callback/{state}`. `axum` does not normalise trailing slashes: a
        // route declared without one would not answer, and the journey would
        // fail at its last step, for everybody, over a single character.
        .route(
            "/api/v2/scrobble-links/lastfm/callback/{state}",
            get(lastfm_callback),
        )
        .route(
            "/api/v2/scrobble-links/lastfm/callback/{state}/",
            get(lastfm_callback),
        )
        // Deliberately not under `scrobble-links/`: `uncertain` would sit in the
        // same position as `{provider}` and the two would be one segment read
        // two ways.
        .route(
            "/api/v2/scrobble-queue/uncertain",
            get(list_uncertain_scrobbles),
        )
        .route(
            "/api/v2/scrobble-queue/uncertain/{entry_id}",
            axum::routing::delete(discard_uncertain_scrobble),
        )
        .route(
            "/api/v2/scrobble-queue/uncertain/{entry_id}/retry",
            post(retry_uncertain_scrobble),
        )
        // Its own ceiling, derived from the queue's (RFC-022): a queue as long
        // as the largest playlist does not fit the 16 KiB every other JSON
        // body is held to.
        .route(
            "/api/v2/queue",
            get(get_queue)
                .put(save_queue)
                .layer(axum::extract::DefaultBodyLimit::max(
                    crate::services::queue_body_limit(),
                )),
        )
        // Smart playlists and podcasts (RFC-025), beside the playlists rather
        // than inside them: their own tables, their own tags.
        .route(
            "/api/v2/smart-playlists",
            get(list_smart_playlists).post(create_smart_playlist),
        )
        .route(
            "/api/v2/smart-playlists/preview",
            post(preview_smart_playlist),
        )
        .route(
            "/api/v2/smart-playlists/{playlist_id}",
            get(get_smart_playlist)
                .patch(update_smart_playlist)
                .delete(delete_smart_playlist),
        )
        .route(
            "/api/v2/podcasts",
            get(podcast_overview).post(create_podcast_channel),
        )
        .route("/api/v2/podcasts/refresh", post(refresh_podcasts))
        .route(
            "/api/v2/podcasts/{channel_id}",
            get(get_podcast_channel).delete(delete_podcast_channel),
        )
        .route(
            "/api/v2/podcast-episodes/newest",
            get(newest_podcast_episodes),
        )
        .route(
            "/api/v2/podcast-episodes/{episode_id}",
            get(get_podcast_episode).delete(delete_podcast_episode),
        )
        .route(
            "/api/v2/podcast-episodes/{episode_id}/download",
            post(download_podcast_episode),
        )
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

/// The one `/api/v2` route that may outlive the ordinary request timeout.
///
/// A Deezer portrait search (RFC-013) waits for the task fetching it, which
/// runs for up to its own five-minute deadline at the provider's pace. Under
/// the ordinary thirty seconds the client would get a `408` while the task
/// went on, and its retry a `429` until the task ended. So it is merged beside
/// the ordinary API rather than inside it, as the media routes are, under a
/// timeout of its own just past that deadline: the request answers what the
/// task did.
pub fn search_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/api/v2/artists/{artist_id}/image/candidates",
            get(list_artist_image_candidates),
        )
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            crate::services::ARTIST_IMAGE_SEARCH_DEADLINE + Duration::from_secs(30),
        ))
        .with_state(state)
}
