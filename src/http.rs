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

const WEB_REFRESH_COOKIE: &str = "waveflow-refresh";
const WEB_CSRF_COOKIE: &str = "waveflow-csrf";
pub const WEB_CSRF_HEADER: &str = "x-waveflow-csrf";
pub const OPERATION_ID_HEADER: &str = "x-waveflow-operation-id";
pub const DEVICE_ID_HEADER: &str = "x-waveflow-device-id";

#[derive(Debug, Serialize, ToSchema)]
pub struct ProbeResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub schema: u8,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ReadyResponse {
    pub status: &'static str,
    pub database: &'static str,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub device_name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebAuthResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub user: crate::authentication::AuthUser,
    pub device_id: Uuid,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ScanQueuedResponse {
    pub scan_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct TrackQuery {
    pub q: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct BrowseQuery {
    pub library_id: Option<Uuid>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// Album discovery parameters. `sort` accepts the same vocabulary as the
/// Subsonic `type` parameter — both surfaces resolve to [`AlbumOrder`], so the
/// web client can build a home screen ("recently added", "most played") in one
/// call instead of paging the whole catalogue and sorting locally.
#[derive(Debug, Deserialize)]
pub struct AlbumBrowseQuery {
    pub library_id: Option<Uuid>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub sort: Option<String>,
    /// Required by `sort=byGenre`, ignored otherwise.
    pub genre: Option<String>,
    pub from_year: Option<i64>,
    pub to_year: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GenreQuery {
    pub library_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    /// Applied to every kind unless the per-kind offset below overrides it.
    pub offset: Option<i64>,
    pub limit: Option<i64>,
    pub artist_offset: Option<i64>,
    pub album_offset: Option<i64>,
    pub song_offset: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct RandomSongQuery {
    pub library_id: Option<Uuid>,
    pub genre: Option<String>,
    pub from_year: Option<i64>,
    pub to_year: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct GenreSongQuery {
    pub genre: String,
    pub library_id: Option<Uuid>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlaylistRequest {
    pub name: String,
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePlaylistRequest {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
    /// Track ids appended to the end, applied after `remove_indexes`.
    #[serde(default)]
    pub add: Vec<Uuid>,
    /// Zero-based positions removed before `add` is applied.
    #[serde(default)]
    pub remove_indexes: Vec<usize>,
    /// Optional fields to blank out, by name. Currently `comment`.
    ///
    /// Omitting a field leaves it untouched, so clearing needs its own verb:
    /// naming it here is the only way to distinguish "unchanged" from "empty",
    /// and it cannot fire by accident on a client that simply omits the field.
    #[serde(default)]
    pub clear: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RatingRequest {
    /// 1 to 5 stars; 0 clears the rating.
    pub rating: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScrobbleRequest {
    pub track_id: Uuid,
    /// `false` records a "now playing" ping, `true` a completed listen.
    #[serde(default)]
    pub submission: bool,
    pub played_at: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SaveQueueRequest {
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
    pub current: Option<Uuid>,
    #[serde(default)]
    pub position_ms: i64,
    pub client: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateShareRequest {
    pub track_ids: Vec<Uuid>,
    pub description: Option<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateShareRequest {
    pub description: Option<String>,
    pub expires_at: Option<i64>,
    /// Optional fields to blank out, by name: `description`, `expires_at`.
    ///
    /// Without this, an expiry set by mistake is permanent — `COALESCE` reads an
    /// absent field and an explicit null identically, so the owner's only
    /// recourse would be deleting the share and publishing a different URL.
    #[serde(default)]
    pub clear: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShareResponse {
    pub id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub description: Option<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub visit_count: i64,
    pub track_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthorizeRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    #[serde(default = "default_challenge_method")]
    pub code_challenge_method: String,
    pub state: Option<String>,
    /// Name recorded for the device this grant will create a session for.
    pub device_name: String,
}

fn default_challenge_method() -> String {
    "S256".into()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorizeResponse {
    /// Where the consent screen must send the user agent.
    pub redirect_to: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenRequest {
    pub code: String,
    pub code_verifier: String,
    pub client_id: String,
    pub redirect_uri: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StarredEntry {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub starred_at: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NowPlayingEntry {
    pub username: String,
    pub song: crate::services::SongItem,
    pub started_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct SyncQuery {
    pub after: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TranscodeStatusResponse {
    pub available: bool,
    pub active: usize,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateUserRequest {
    pub username: String,
    pub web_password: String,
    pub role: crate::database::AccountRole,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub role: Option<crate::database::AccountRole>,
    pub disabled: Option<bool>,
    pub library_ids: Option<Vec<Uuid>>,
    pub subsonic_password: Option<String>,
    pub web_password: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetSubsonicCredentialRequest {
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SubsonicCredentialResponse {
    /// Shown once. Only its SHA-256 hash is stored by the server.
    pub api_key: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatusResponse {
    pub required: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupResponse {
    pub user_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLibraryRequest {
    pub name: String,
    pub path: String,
    pub visibility: crate::database::LibraryVisibility,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateLibraryResponse {
    pub library_id: Uuid,
    pub scan_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetLibraryMemberRequest {
    pub role: crate::database::LibraryRole,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SyncAckRequest {
    pub device_id: Uuid,
    pub cursor: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SyncSnapshot {
    pub cursor: i64,
    pub playlists: Vec<crate::services::PlaylistItem>,
    pub favorites: Vec<StarredEntry>,
    pub ratings: Vec<crate::services::RatingItem>,
    pub queue: Option<crate::services::QueueItem>,
    pub history: Vec<crate::services::HistoryItem>,
    pub shares: Vec<ShareResponse>,
    pub bookmarks: Vec<crate::services::BookmarkItem>,
}

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

#[utoipa::path(
    get,
    path = "/health",
    tag = "probes",
    responses((status = 200, body = ProbeResponse))
)]
pub async fn health() -> Json<ProbeResponse> {
    Json(ProbeResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        schema: 2,
    })
}

#[utoipa::path(
    get,
    path = "/ready",
    tag = "probes",
    responses(
        (status = 200, body = ReadyResponse),
        (status = 503, body = ReadyResponse)
    )
)]
pub async fn ready(State(state): State<AppState>) -> Response {
    match state.db.ping().await {
        Ok(()) => (
            StatusCode::OK,
            Json(ReadyResponse {
                status: "ready",
                database: "ok",
            }),
        )
            .into_response(),
        Err(error) => {
            tracing::warn!(error = %error, "readiness database probe failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ReadyResponse {
                    status: "unavailable",
                    database: "unavailable",
                }),
            )
                .into_response()
        }
    }
}

#[utoipa::path(get, path = "/api/v2/setup", tag = "authentication", responses((status = 200, body = SetupStatusResponse)))]
pub async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    let required = state.db.setup_required().await.map_err(db_error)?;
    Ok(Json(SetupStatusResponse { required }))
}

#[utoipa::path(post, path = "/api/v2/setup", tag = "authentication", params(("Origin" = String, Header, description = "Required browser origin")), request_body = SetupRequest, responses((status = 201, body = SetupResponse), (status = 403, description = "Origin header missing or rejected", body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetupRequest>,
) -> Result<(StatusCode, Json<SetupResponse>), ApiError> {
    validate_web_origin(&state, &headers)?;
    let user_id = state
        .services
        .bootstrap_admin(&request.username, &request.password)
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(SetupResponse { user_id })))
}

#[utoipa::path(
    post,
    path = "/api/v2/auth/login",
    tag = "authentication",
    request_body = LoginRequest,
    responses(
        (status = 200, body = crate::authentication::AuthTokens),
        (status = 401, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<crate::authentication::AuthTokens>, ApiError> {
    state
        .auth
        .login(&request.username, &request.password, &request.device_name)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

#[utoipa::path(
    post,
    path = "/api/v2/auth/refresh",
    tag = "authentication",
    request_body = RefreshRequest,
    responses(
        (status = 200, body = crate::authentication::AuthTokens),
        (status = 401, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Result<Json<crate::authentication::AuthTokens>, ApiError> {
    state
        .auth
        .refresh(&request.refresh_token)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

#[utoipa::path(
    post,
    path = "/api/v2/auth/logout",
    tag = "authentication",
    responses(
        (status = 204),
        (status = 401, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let access_token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    state
        .auth
        .logout(access_token)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Browser sessions keep only the short-lived access token in JavaScript. The
/// rotating refresh token is an HttpOnly, same-site cookie and is therefore
/// never exposed to the embedded SPA.
#[utoipa::path(
    post,
    path = "/api/v2/web/auth/login",
    tag = "authentication",
    request_body = LoginRequest,
    responses(
        (status = 200, body = WebAuthResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse)
    )
)]
pub async fn web_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    validate_web_origin(&state, &headers)?;
    let tokens = state
        .auth
        .login(&request.username, &request.password, &request.device_name)
        .await
        .map_err(ApiError::from)?;
    web_auth_response(&state, &headers, tokens)
}

#[utoipa::path(
    post,
    path = "/api/v2/web/auth/refresh",
    tag = "authentication",
    responses(
        (status = 200, body = WebAuthResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse)
    )
)]
pub async fn web_refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_web_request(&state, &headers)?;
    let refresh_token = cookie_value(&headers, WEB_REFRESH_COOKIE).ok_or(ApiError::Unauthorized)?;
    let tokens = state
        .auth
        .refresh(refresh_token)
        .await
        .map_err(ApiError::from)?;
    web_auth_response(&state, &headers, tokens)
}

#[utoipa::path(
    post,
    path = "/api/v2/web/auth/logout",
    tag = "authentication",
    responses(
        (status = 204),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse)
    )
)]
pub async fn web_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_web_request(&state, &headers)?;
    let result = match cookie_value(&headers, WEB_REFRESH_COOKIE) {
        Some(refresh_token) => state.auth.revoke_refresh(refresh_token).await,
        None => Err(AuthError::InvalidRefreshToken),
    };
    let mut response = match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => ApiError::from(error).into_response(),
    };
    let secure = secure_cookies(&state);
    append_cookie(
        &mut response,
        expired_cookie(WEB_REFRESH_COOKIE, true, secure),
    )?;
    append_cookie(
        &mut response,
        expired_cookie(WEB_CSRF_COOKIE, false, secure),
    )?;
    Ok(response)
}

#[utoipa::path(post, path = "/api/v2/libraries/{library_id}/scans", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 202, body = ScanQueuedResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn start_scan(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<ScanQueuedResponse>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let scan_id = state
        .services
        .start_library_scan(user.id, library_id)
        .await
        .map_err(service_error)?;
    Ok((StatusCode::ACCEPTED, Json(ScanQueuedResponse { scan_id })))
}

#[utoipa::path(get, path = "/api/v2/libraries", tag = "catalog", responses((status = 200, body = [crate::catalog::LibraryAccess]), (status = 401, body = ErrorResponse)))]
pub async fn list_libraries(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::catalog::LibraryAccess>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .db
        .libraries_for_user(user.id)
        .await
        .map(Json)
        .map_err(db_error)
}

#[utoipa::path(post, path = "/api/v2/libraries", tag = "administration", request_body = CreateLibraryRequest, responses((status = 201, body = CreateLibraryResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_library(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateLibraryRequest>,
) -> Result<(StatusCode, Json<CreateLibraryResponse>), ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    let path = std::path::PathBuf::from(&request.path);
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| ApiError::Validation)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || request.name.trim().is_empty() {
        return Err(ApiError::Validation);
    }
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| ApiError::Validation)?;
    let library_id = state
        .db
        .create_library(
            actor.id,
            &request.name,
            &canonical,
            request.visibility,
            crate::authentication::now_ms(),
        )
        .await
        .map_err(db_error)?;
    let scan_id = state
        .scanner
        .trigger(
            crate::catalog::LibraryRecord {
                id: library_id,
                name: request.name,
                root_path: canonical,
            },
            Some(actor.id),
            "library_added",
        )
        .await
        .map_err(|error| {
            tracing::error!(error = %error, library_id = %library_id, "initial scan queue failed");
            ApiError::Unavailable
        })?;
    Ok((
        StatusCode::CREATED,
        Json(CreateLibraryResponse {
            library_id,
            scan_id,
        }),
    ))
}

#[utoipa::path(put, path = "/api/v2/libraries/{library_id}/members/{user_id}", tag = "administration", params(("library_id" = Uuid, Path), ("user_id" = Uuid, Path)), request_body = SetLibraryMemberRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_library_member(
    State(state): State<AppState>,
    Path((library_id, user_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<SetLibraryMemberRequest>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    if request.role == crate::database::LibraryRole::Owner
        || state
            .db
            .account_by_id(user_id)
            .await
            .map_err(db_error)?
            .is_none()
        || !state
            .db
            .all_libraries()
            .await
            .map_err(db_error)?
            .iter()
            .any(|library| library.id == library_id)
    {
        return Err(ApiError::Validation);
    }
    state
        .db
        .add_library_member(
            actor.id,
            library_id,
            user_id,
            request.role,
            crate::authentication::now_ms(),
        )
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v2/libraries/{library_id}/members/{user_id}", tag = "administration", params(("library_id" = Uuid, Path), ("user_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn remove_library_member(
    State(state): State<AppState>,
    Path((library_id, user_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    if state
        .db
        .remove_library_member(
            actor.id,
            library_id,
            user_id,
            crate::authentication::now_ms(),
        )
        .await
        .map_err(db_error)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(get, path = "/api/v2/scans/{scan_id}", tag = "catalog", params(("scan_id" = Uuid, Path)), responses((status = 200, body = crate::catalog::ScanJobRecord), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn scan_status(
    State(state): State<AppState>,
    Path(scan_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::catalog::ScanJobRecord>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .db
        .scan_job_for_user(user.id, scan_id)
        .await
        .map_err(db_error)?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(get, path = "/api/v2/scans/{scan_id}/events", tag = "catalog", params(("scan_id" = Uuid, Path)), responses((status = 200, description = "Server-sent scan progress events", content_type = "text/event-stream"), (status = 404, body = ErrorResponse)))]
pub async fn scan_events(
    State(state): State<AppState>,
    Path(scan_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let initial = state
        .db
        .scan_job_for_user(user.id, scan_id)
        .await
        .map_err(db_error)?
        .ok_or(ApiError::NotFound)?;
    let mut receiver = state.scanner.subscribe(scan_id);
    let output = async_stream::stream! {
        yield Ok(Event::default().event("snapshot").json_data(initial).expect("scan snapshot serializes"));
        if let Some(ref mut receiver) = receiver {
            loop {
                match receiver.recv().await {
                    Ok(progress) => yield Ok(Event::default().event("progress").json_data(progress).expect("scan progress serializes")),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    };
    Ok(Sse::new(output).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/tracks", tag = "catalog", params(("library_id" = Uuid, Path), ("q" = Option<String>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::catalog::TrackRecord]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_tracks(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    Query(query): Query<TrackQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::catalog::TrackRecord>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    if state
        .db
        .library_for_user(user.id, library_id)
        .await
        .map_err(db_error)?
        .is_none()
    {
        return Err(ApiError::NotFound);
    }
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(500);
    if offset < 0 || !(1..=500).contains(&limit) {
        return Err(ApiError::Validation);
    }
    let query = query.q.as_deref().map(str::trim).filter(|q| !q.is_empty());
    let tracks = state
        .db
        .browse_tracks_for_user(user.id, library_id, query, offset, limit)
        .await
        .map_err(db_error)?;
    Ok(Json(tracks))
}

#[utoipa::path(get, path = "/api/v2/tracks/{track_id}", tag = "catalog", params(("track_id" = Uuid, Path)), responses((status = 200, body = crate::services::SongItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_track(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::SongItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .songs_by_ids(user.id, &[track_id])
        .await
        .map_err(service_error)?
        .into_iter()
        .next()
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(get, path = "/api/v2/tracks/{track_id}/lyrics", tag = "catalog", params(("track_id" = Uuid, Path)), responses((status = 200, body = crate::lyrics::LyricsList), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_track_lyrics(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::lyrics::LyricsList>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .lyrics(user.id, track_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/albums", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query), ("sort" = Option<String>, Query), ("genre" = Option<String>, Query), ("from_year" = Option<i64>, Query), ("to_year" = Option<i64>, Query)), responses((status = 200, body = [crate::services::AlbumItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_albums(
    State(state): State<AppState>,
    Query(query): Query<AlbumBrowseQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::AlbumItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let order = query
        .sort
        .as_deref()
        .map(crate::services::AlbumOrder::from_str)
        .transpose()
        .map_err(service_error)?
        .unwrap_or_default();
    let request = crate::services::AlbumListQuery {
        library_ids: query.library_id.into_iter().collect(),
        order,
        genre: query.genre,
        from_year: query.from_year,
        to_year: query.to_year,
        page: crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?,
    };
    state
        .services
        .list_albums(user.id, &request)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/genres", tag = "catalog", params(("library_id" = Option<Uuid>, Query)), responses((status = 200, body = [crate::services::GenreItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_genres(
    State(state): State<AppState>,
    Query(query): Query<GenreQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::GenreItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let libraries = query.library_id.into_iter().collect::<Vec<_>>();
    state
        .services
        .list_genres(user.id, &libraries)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/albums/{album_id}", tag = "catalog", params(("album_id" = Uuid, Path)), responses((status = 200, body = crate::services::AlbumDetail), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_album(
    State(state): State<AppState>,
    Path(album_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::AlbumDetail>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .album(user.id, album_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/artists", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::ArtistSummary]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_artists(
    State(state): State<AppState>,
    Query(query): Query<BrowseQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::ArtistSummary>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .list_artists(user.id, query.library_id, page)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/artists/{artist_id}", tag = "catalog", params(("artist_id" = Uuid, Path)), responses((status = 200, body = crate::services::ArtistDetail), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_artist(
    State(state): State<AppState>,
    Path(artist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::ArtistDetail>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .artist(user.id, artist_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/search", tag = "catalog", params(("q" = String, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = crate::services::SearchResult), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn search_catalog(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
    headers: HeaderMap,
) -> Result<Json<crate::services::SearchResult>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    // One offset for all three kinds unless the caller names one, which is
    // what `search3` has always allowed and what a client paging songs past
    // the end of the artists needs.
    let page = |offset: Option<i64>| {
        crate::services::BrowsePage::new(offset.or(query.offset), query.limit)
            .map_err(service_error)
    };
    state
        .services
        .search(
            user.id,
            &query.q,
            page(query.artist_offset)?,
            page(query.album_offset)?,
            page(query.song_offset)?,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

/// The native form of `getRandomSongs`.
///
/// The selection is drawn in SQL, so a request for ten reads ten. `genre`
/// matches the canonical name, like every other genre filter on either
/// surface, and a reversed year range is read as a range rather than as an
/// empty one.
#[utoipa::path(get, path = "/api/v2/songs/random", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("genre" = Option<String>, Query), ("from_year" = Option<i64>, Query), ("to_year" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::SongItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_random_songs(
    State(state): State<AppState>,
    Query(query): Query<RandomSongQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SongItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .random_songs(
            user.id,
            query.library_id.as_slice(),
            query.genre.as_deref(),
            query.from_year,
            query.to_year,
            query.limit.unwrap_or(10),
        )
        .await
        .map(Json)
        .map_err(service_error)
}

/// The native form of `getSongsByGenre`. `genre` is required: answering an
/// unfiltered catalogue would drop the filter in silence.
#[utoipa::path(get, path = "/api/v2/songs", tag = "catalog", params(("genre" = String, Query), ("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::SongItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_songs_by_genre(
    State(state): State<AppState>,
    Query(query): Query<GenreSongQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SongItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .songs_by_genre(user.id, query.library_id.as_slice(), &query.genre, page)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/oauth/authorize", tag = "authentication", request_body = AuthorizeRequest, responses((status = 200, body = AuthorizeResponse), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn oauth_authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AuthorizeRequest>,
) -> Result<Json<AuthorizeResponse>, ApiError> {
    // The browser session is the proof of identity; the consent screen is a
    // route of the embedded client, so this is a JSON call rather than a form.
    let user = authenticated(&state, &headers, Access::Write).await?;
    let redirect_to = state
        .services
        .authorize_native_client(
            user.id,
            crate::services::AuthorizationRequest {
                client_id: &request.client_id,
                redirect_uri: &request.redirect_uri,
                code_challenge: &request.code_challenge,
                code_challenge_method: &request.code_challenge_method,
                device_name: &request.device_name,
                state: request.state.as_deref(),
            },
        )
        .await
        .map_err(service_error)?;
    Ok(Json(AuthorizeResponse { redirect_to }))
}

#[utoipa::path(post, path = "/api/v2/oauth/token", tag = "authentication", request_body = TokenRequest, responses((status = 200, body = crate::authentication::AuthTokens), (status = 401, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub async fn oauth_token(
    State(state): State<AppState>,
    Json(request): Json<TokenRequest>,
) -> Result<Json<crate::authentication::AuthTokens>, ApiError> {
    // Mounted without authentication by design: the code plus the verifier are
    // the credential. Every rejection below is the same 401 so a caller cannot
    // learn whether a code existed, expired, or was already spent.
    let now = crate::authentication::now_ms();
    let grant = state
        .db
        .redeem_authorization(&crate::security::token_hash(&request.code), now)
        .await
        .map_err(db_error)?
        .ok_or(ApiError::Unauthorized)?;
    if grant.client_id != request.client_id.trim()
        || grant.redirect_uri != request.redirect_uri
        || crate::oauth::verify_challenge(&grant.code_challenge, &request.code_verifier).is_err()
    {
        return Err(ApiError::Unauthorized);
    }
    state
        .auth
        .issue_session_for_account(grant.user_id, &grant.device_name)
        .await
        .map(Json)
        .map_err(|error| match error {
            crate::authentication::AuthError::Unavailable => ApiError::Unavailable,
            _ => ApiError::Unauthorized,
        })
}

#[utoipa::path(get, path = "/api/v2/playlists", tag = "user-data", responses((status = 200, body = [crate::services::PlaylistItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_playlists(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::PlaylistItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .playlists(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/playlists", tag = "user-data", request_body = CreatePlaylistRequest, responses((status = 201, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreatePlaylistRequest>,
) -> Result<(StatusCode, Json<crate::services::PlaylistItem>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let playlist = state
        .services
        .create_playlist_with_context(user.id, &request.name, &request.track_ids, context)
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(playlist)))
}

#[utoipa::path(get, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .playlist(user.id, playlist_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// An unknown name is refused rather than ignored: a client asking to clear
/// `expiresAt` instead of `expires_at` would otherwise be told it succeeded
/// while the field stayed put.
fn playlist_clear(names: &[String]) -> Result<crate::services::PlaylistClear, ApiError> {
    let mut clear = crate::services::PlaylistClear::default();
    for name in names {
        match name.as_str() {
            "comment" => clear.comment = true,
            _ => return Err(ApiError::Validation),
        }
    }
    Ok(clear)
}

/// See [`playlist_clear`].
fn share_clear(names: &[String]) -> Result<crate::services::ShareClear, ApiError> {
    let mut clear = crate::services::ShareClear::default();
    for name in names {
        match name.as_str() {
            "description" => clear.description = true,
            "expires_at" => clear.expires_at = true,
            _ => return Err(ApiError::Validation),
        }
    }
    Ok(clear)
}

#[utoipa::path(patch, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), request_body = UpdatePlaylistRequest, responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdatePlaylistRequest>,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .update_playlist_with_context(
            user.id,
            playlist_id,
            request.name.as_deref(),
            request.comment.as_deref(),
            request.public,
            &request.add,
            &request.remove_indexes,
            playlist_clear(&request.clear)?,
            context,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(delete, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .delete_playlist_with_context(user.id, playlist_id, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/favorites", tag = "user-data", responses((status = 200, body = [StarredEntry]), (status = 401, body = ErrorResponse)))]
pub async fn list_favorites(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StarredEntry>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let entries = state
        .services
        .starred_ids(user.id)
        .await
        .map_err(service_error)?
        .into_iter()
        .map(|(entity_type, entity_id, starred_at)| StarredEntry {
            entity_type,
            entity_id,
            starred_at,
        })
        .collect();
    Ok(Json(entries))
}

#[utoipa::path(put, path = "/api/v2/favorites/{entity_type}/{entity_id}", tag = "user-data", params(("entity_type" = String, Path, description = "track, album or artist"), ("entity_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn add_favorite(
    State(state): State<AppState>,
    Path((entity_type, entity_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    set_favorite(state, headers, &entity_type, entity_id, true).await
}

#[utoipa::path(delete, path = "/api/v2/favorites/{entity_type}/{entity_id}", tag = "user-data", params(("entity_type" = String, Path, description = "track, album or artist"), ("entity_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn remove_favorite(
    State(state): State<AppState>,
    Path((entity_type, entity_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    set_favorite(state, headers, &entity_type, entity_id, false).await
}

async fn set_favorite(
    state: AppState,
    headers: HeaderMap,
    entity_type: &str,
    entity_id: Uuid,
    starred: bool,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .set_star_with_context(user.id, entity_type, entity_id, starred, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(put, path = "/api/v2/ratings/{entity_type}/{entity_id}", tag = "user-data", params(("entity_type" = String, Path, description = "track, album or artist"), ("entity_id" = Uuid, Path)), request_body = RatingRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_rating(
    State(state): State<AppState>,
    Path((entity_type, entity_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<RatingRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .set_rating_with_context(user.id, &entity_type, entity_id, request.rating, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A playback position to store on a track.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BookmarkRequest {
    /// Milliseconds from the start of the file. Negative positions are refused.
    pub position_ms: i64,
    /// Free text. Omitting it clears whatever comment the bookmark carried,
    /// because a bookmark is replaced rather than patched.
    #[serde(default)]
    pub comment: Option<String>,
}

#[utoipa::path(get, path = "/api/v2/bookmarks", tag = "user-data", responses((status = 200, body = [crate::services::BookmarkItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_bookmarks(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::BookmarkItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .bookmarks(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// One bookmark per account and track, so this replaces rather than adds.
///
/// `PUT` and not `POST` for that reason: the track names the resource, and
/// sending the same position twice leaves the same single bookmark. Backed by
/// the same `DomainServices` method as the Subsonic `createBookmark`, so the
/// two surfaces cannot disagree about what a second call does.
#[utoipa::path(put, path = "/api/v2/bookmarks/{track_id}", tag = "user-data", params(("track_id" = Uuid, Path)), request_body = BookmarkRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_bookmark(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<BookmarkRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .set_bookmark_with_context(
            user.id,
            track_id,
            request.position_ms,
            request.comment.as_deref(),
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deleting a bookmark that is not there succeeds: the caller asked for the
/// track to carry none, and it does not. It also avoids answering a question
/// about a track the account cannot reach.
#[utoipa::path(delete, path = "/api/v2/bookmarks/{track_id}", tag = "user-data", params(("track_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_bookmark(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .delete_bookmark_with_context(user.id, track_id, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// A token to issue.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateApiTokenRequest {
    /// What the token is for. Shown in the listing so a stale one can be told
    /// apart from a live one before it is revoked.
    pub name: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// An issued token. The secret appears here and nowhere else, ever again.
#[derive(Debug, Serialize, ToSchema)]
pub struct CreateApiTokenResponse {
    #[serde(flatten)]
    pub token: crate::database::ApiTokenRecord,
    /// Shown once. Only its SHA-256 hash is stored, so it cannot be recovered.
    pub secret: String,
}

#[utoipa::path(get, path = "/api/v2/admin/users/{username}/tokens", tag = "administration", params(("username" = String, Path)), responses((status = 200, body = [crate::database::ApiTokenRecord]), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn list_api_tokens(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::database::ApiTokenRecord>>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .api_tokens(actor.id, &username)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Issues an API token without a shell on the host.
///
/// The `token create` CLI command remains, for bootstrapping an instance that
/// has no administrator session yet; from here on the two share
/// `DomainServices::create_api_token`, so a token minted either way carries the
/// same scopes and the same audit trail.
#[utoipa::path(post, path = "/api/v2/admin/users/{username}/tokens", tag = "administration", params(("username" = String, Path)), request_body = CreateApiTokenRequest, responses((status = 201, body = CreateApiTokenResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_api_token(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(request): Json<CreateApiTokenRequest>,
) -> Result<(StatusCode, Json<CreateApiTokenResponse>), ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    let (token, secret) = state
        .services
        .create_api_token(actor.id, &username, &request.name, &request.scopes)
        .await
        .map_err(service_error)?;
    Ok((
        StatusCode::CREATED,
        Json(CreateApiTokenResponse { token, secret }),
    ))
}

#[utoipa::path(delete, path = "/api/v2/admin/users/{username}/tokens/{token_id}", tag = "administration", params(("username" = String, Path), ("token_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn revoke_api_token(
    State(state): State<AppState>,
    Path((username, token_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .revoke_api_token(actor.id, &username, token_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/ratings", tag = "user-data", responses((status = 200, body = [crate::services::RatingItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_ratings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::RatingItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .ratings(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/scrobbles", tag = "user-data", request_body = ScrobbleRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_scrobble(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ScrobbleRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .scrobble_with_context(
            user.id,
            request.track_id,
            request.submission,
            request.played_at,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/history", tag = "user-data", params(("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::HistoryItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<crate::services::HistoryItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let limit = query.limit.unwrap_or(200);
    if !(1..=crate::sync::MAX_SYNC_LIMIT).contains(&limit) {
        return Err(ApiError::Validation);
    }
    state
        .services
        .history(user.id, limit)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/transcode/status", tag = "catalog", responses((status = 200, body = TranscodeStatusResponse), (status = 401, body = ErrorResponse)))]
pub async fn transcode_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TranscodeStatusResponse>, ApiError> {
    authenticated(&state, &headers, Access::Read).await?;
    Ok(Json(TranscodeStatusResponse {
        available: state.media.transcoding_available(),
        active: state.media.active_transcodes(),
    }))
}

#[utoipa::path(get, path = "/api/v2/admin/users", tag = "administration", responses((status = 200, body = [crate::services::UserItem]), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse)))]
pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::UserItem>>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .users(actor.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/admin/users", tag = "administration", request_body = CreateUserRequest, responses((status = 201, body = crate::services::UserItem), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<crate::services::UserItem>), ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    let user = state
        .services
        .create_web_user(
            actor.id,
            &request.username,
            &request.web_password,
            request.role,
        )
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(user)))
}

#[utoipa::path(patch, path = "/api/v2/admin/users/{username}", tag = "administration", params(("username" = String, Path)), request_body = UpdateUserRequest, responses((status = 200, body = crate::services::UserItem), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<crate::services::UserItem>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .update_user(
            actor.id,
            &username,
            crate::services::UserUpdate {
                admin: request
                    .role
                    .map(|role| role == crate::database::AccountRole::Admin),
                disabled: request.disabled,
                folder_ids: request.library_ids.as_deref(),
                subsonic_password: request.subsonic_password.as_deref(),
                web_password: request.web_password.as_deref(),
            },
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(delete, path = "/api/v2/admin/users/{username}", tag = "administration", params(("username" = String, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .delete_user(actor.id, &username)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(put, path = "/api/v2/admin/users/{username}/subsonic-credential", tag = "administration", params(("username" = String, Path)), request_body = SetSubsonicCredentialRequest, responses((status = 200, body = SubsonicCredentialResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_subsonic_credential(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SetSubsonicCredentialRequest>,
) -> Result<Json<SubsonicCredentialResponse>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    let api_key = state
        .services
        .set_subsonic_credential(actor.id, &username, &request.password)
        .await
        .map_err(service_error)?;
    Ok(Json(SubsonicCredentialResponse { api_key }))
}

#[utoipa::path(delete, path = "/api/v2/admin/users/{username}/subsonic-credential", tag = "administration", params(("username" = String, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn revoke_subsonic_credential(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .revoke_subsonic_credential(actor.id, &username)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/now-playing", tag = "user-data", responses((status = 200, body = [NowPlayingEntry]), (status = 401, body = ErrorResponse)))]
pub async fn list_now_playing(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<NowPlayingEntry>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let entries = state
        .services
        .now_playing(user.id)
        .await
        .map_err(service_error)?
        .into_iter()
        .map(|(username, song, started_at)| NowPlayingEntry {
            username,
            song,
            started_at,
        })
        .collect();
    Ok(Json(entries))
}

#[utoipa::path(get, path = "/api/v2/queue", tag = "user-data", responses((status = 200, body = Option<crate::services::QueueItem>), (status = 401, body = ErrorResponse)))]
pub async fn get_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Option<crate::services::QueueItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .queue(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(put, path = "/api/v2/queue", tag = "user-data", request_body = SaveQueueRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn save_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveQueueRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .save_queue_with_context(
            user.id,
            &request.track_ids,
            request.current,
            request.position_ms,
            request.client.as_deref(),
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/shares", tag = "user-data", responses((status = 200, body = [ShareResponse]), (status = 401, body = ErrorResponse)))]
pub async fn list_shares(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ShareResponse>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let shares = state
        .services
        .shares(user.id)
        .await
        .map_err(service_error)?
        .into_iter()
        .map(|share| share_response(&state, share))
        .collect();
    Ok(Json(shares))
}

#[utoipa::path(post, path = "/api/v2/shares", tag = "user-data", request_body = CreateShareRequest, responses((status = 201, body = ShareResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<ShareResponse>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let share = state
        .services
        .create_share_with_context(
            user.id,
            &request.track_ids,
            request.description.as_deref(),
            request.expires_at,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(share_response(&state, share))))
}

#[utoipa::path(patch, path = "/api/v2/shares/{share_id}", tag = "user-data", params(("share_id" = Uuid, Path)), request_body = UpdateShareRequest, responses((status = 200, body = ShareResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn update_share(
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdateShareRequest>,
) -> Result<Json<ShareResponse>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let share = state
        .services
        .update_share_with_context(
            user.id,
            share_id,
            request.description.as_deref(),
            request.expires_at,
            share_clear(&request.clear)?,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(Json(share_response(&state, share)))
}

#[utoipa::path(delete, path = "/api/v2/shares/{share_id}", tag = "user-data", params(("share_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_share(
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .delete_share_with_context(user.id, share_id, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn share_response(state: &AppState, share: crate::services::ShareItem) -> ShareResponse {
    let url = share.url_token.map(|token| {
        let path = format!("/share/{token}");
        state
            .public_url
            .as_ref()
            .map_or_else(|| path.clone(), |base| format!("{base}{path}"))
    });
    ShareResponse {
        id: share.id,
        url,
        description: share.description,
        expires_at: share.expires_at,
        created_at: share.created_at,
        visit_count: share.visit_count,
        track_ids: share.songs.into_iter().map(|song| song.id).collect(),
    }
}

#[utoipa::path(
    get,
    path = "/api/v2/sync/changes",
    tag = "sync",
    params(("after" = Option<i64>, Query), ("limit" = Option<i64>, Query)),
    responses(
        (status = 200, body = crate::sync::SyncPage),
        (status = 401, body = ErrorResponse),
        (
            status = 409,
            description = "`code` is `cursor_expired`: the cursor precedes the oldest \
                           retained event, so the gap cannot be replayed. Discard the local \
                           projection, take a fresh /sync/snapshot and resume from its \
                           cursor. Distinct from `conflict`, which is about operation ids.",
            body = ErrorResponse
        ),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn sync_changes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SyncQuery>,
) -> Result<Json<crate::sync::SyncPage>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let after = query.after.unwrap_or(0);
    let limit = query.limit.unwrap_or(crate::sync::DEFAULT_SYNC_LIMIT);
    if after < 0 || limit <= 0 || limit > crate::sync::MAX_SYNC_LIMIT {
        return Err(ApiError::Validation);
    }
    state
        .sync
        .changes(user.id, after, limit)
        .await
        .map(Json)
        .map_err(sync_error)
}

#[utoipa::path(
    get,
    path = "/api/v2/sync/snapshot",
    tag = "sync",
    responses((status = 200, body = SyncSnapshot), (status = 401, body = ErrorResponse))
)]
pub async fn sync_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SyncSnapshot>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let snapshot = state
        .services
        .sync_snapshot(user.id, crate::sync::MAX_SYNC_LIMIT)
        .await
        .map_err(service_error)?;
    let favorites = snapshot
        .favorites
        .into_iter()
        .map(|(entity_type, entity_id, starred_at)| StarredEntry {
            entity_type,
            entity_id,
            starred_at,
        })
        .collect();
    let shares = snapshot
        .shares
        .into_iter()
        .map(|share| share_response(&state, share))
        .collect();
    Ok(Json(SyncSnapshot {
        cursor: snapshot.cursor,
        playlists: snapshot.playlists,
        favorites,
        ratings: snapshot.ratings,
        queue: snapshot.queue,
        history: snapshot.history,
        shares,
        bookmarks: snapshot.bookmarks,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v2/sync/ack",
    tag = "sync",
    request_body = SyncAckRequest,
    responses(
        (status = 204),
        (status = 401, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn sync_ack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SyncAckRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let acknowledged = state
        .sync
        .acknowledge(user.id, request.device_id, request.cursor)
        .await
        .map_err(db_error)?;
    if !acknowledged {
        return Err(ApiError::Validation);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The socket is an edge-triggered wake-up channel. A client always follows a
/// notice with `GET /sync/changes`; the durable cursor, not socket delivery, is
/// the synchronization guarantee.
#[utoipa::path(
    get,
    path = "/api/v2/sync/socket",
    tag = "sync",
    params(("after" = Option<i64>, Query)),
    responses(
        (status = 101, description = "WebSocket cursor notifications"),
        (status = 401, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn sync_socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SyncQuery>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let after = query.after.unwrap_or(0);
    if after < 0 {
        return Err(ApiError::Validation);
    }
    Ok(upgrade
        .on_upgrade(move |socket| serve_sync_socket(socket, state, user.id, after))
        .into_response())
}

async fn serve_sync_socket(socket: WebSocket, state: AppState, user_id: Uuid, after: i64) {
    let (mut sender, mut receiver) = socket.split();
    let mut notices = state.sync.subscribe();
    if let Ok(cursor) = state.sync.latest_user_cursor(user_id).await {
        if cursor > after && send_sync_notice(&mut sender, cursor).await.is_err() {
            return;
        }
    }
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    let mut awaiting_pong = false;
    loop {
        tokio::select! {
            incoming = receiver.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(Message::Pong(_))) => awaiting_pong = false,
                Some(Ok(Message::Ping(payload))) => {
                    if sender.send(Message::Pong(payload)).await.is_err() {
                        break;
                    }
                }
                Some(Ok(_)) => {}
            },
            notice = notices.recv() => match sync_notice_action(&state.sync, user_id, notice).await {
                Ok(SyncNoticeAction::Send(cursor)) => {
                    if send_sync_notice(&mut sender, cursor).await.is_err() {
                        break;
                    }
                }
                Ok(SyncNoticeAction::Continue) => {}
                Ok(SyncNoticeAction::Close) | Err(_) => break,
            },
            _ = heartbeat.tick() => {
                if awaiting_pong || sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
                awaiting_pong = true;
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum SyncNoticeAction {
    Send(i64),
    Continue,
    Close,
}

async fn sync_notice_action(
    sync: &crate::sync::SyncService,
    user_id: Uuid,
    notice: Result<(Uuid, crate::sync::SyncNotice), tokio::sync::broadcast::error::RecvError>,
) -> Result<SyncNoticeAction, sqlx::Error> {
    match notice {
        Ok((notice_user, notice)) if notice_user == user_id => {
            Ok(SyncNoticeAction::Send(notice.cursor))
        }
        Ok(_) => Ok(SyncNoticeAction::Continue),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => sync
            .latest_user_cursor(user_id)
            .await
            .map(SyncNoticeAction::Send),
        Err(tokio::sync::broadcast::error::RecvError::Closed) => Ok(SyncNoticeAction::Close),
    }
}

async fn send_sync_notice(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    cursor: i64,
) -> Result<(), axum::Error> {
    let body =
        serde_json::to_string(&crate::sync::SyncNotice { cursor }).expect("sync notice serializes");
    sender.send(Message::Text(body.into())).await
}

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    Forbidden,
    Validation,
    /// The request is well formed but collides with existing state: an
    /// operation id replayed with a different payload, or a name already taken.
    /// Distinct from `Validation` so a client can tell "my request is malformed"
    /// from "my retry is inconsistent" — both permanent, different fixes.
    Conflict,
    /// The sync cursor precedes the oldest retained event. Same 409 status as
    /// `Conflict` but a distinct code, because the reactions are opposite:
    /// a conflict means mint a new operation id, this one means discard the
    /// local projection and take a fresh snapshot.
    CursorExpired,
    Unavailable,
    NotFound,
}

impl From<AuthError> for ApiError {
    fn from(value: AuthError) -> Self {
        match value {
            AuthError::InvalidCredentials | AuthError::InvalidRefreshToken => Self::Unauthorized,
            AuthError::InvalidDeviceName => Self::Validation,
            AuthError::Unavailable => Self::Unavailable,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication failed",
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Request rejected"),
            Self::Validation => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                "The request is invalid",
            ),
            Self::Conflict => (
                StatusCode::CONFLICT,
                "conflict",
                "The request conflicts with existing state",
            ),
            Self::CursorExpired => (
                StatusCode::CONFLICT,
                "cursor_expired",
                "The cursor precedes the oldest retained event; take a fresh snapshot",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "Authentication is temporarily unavailable",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
        };
        (status, Json(ErrorResponse { code, message })).into_response()
    }
}

fn web_auth_response(
    state: &AppState,
    _headers: &HeaderMap,
    tokens: crate::authentication::AuthTokens,
) -> Result<Response, ApiError> {
    let csrf_token = crate::security::generate_token("wfcsrf_");
    let secure = secure_cookies(state);
    let refresh_cookie = format!(
        "{WEB_REFRESH_COOKIE}={}; Path=/api/v2/web/auth; HttpOnly; SameSite=Strict; Max-Age={}{}",
        tokens.refresh_token,
        state.refresh_token_ttl.as_secs(),
        if secure { "; Secure" } else { "" }
    );
    let csrf_cookie = format!(
        "{WEB_CSRF_COOKIE}={csrf_token}; Path=/; SameSite=Strict; Max-Age={}{}",
        state.refresh_token_ttl.as_secs(),
        if secure { "; Secure" } else { "" }
    );
    let body = WebAuthResponse {
        access_token: tokens.access_token,
        token_type: tokens.token_type,
        expires_in: tokens.expires_in,
        user: tokens.user,
        device_id: tokens.device_id,
    };
    let mut response = Json(body).into_response();
    append_cookie(&mut response, refresh_cookie)?;
    append_cookie(&mut response, csrf_cookie)?;
    Ok(response)
}

fn append_cookie(response: &mut Response, value: String) -> Result<(), ApiError> {
    let value = HeaderValue::from_str(&value).map_err(|_| ApiError::Unavailable)?;
    response.headers_mut().append(header::SET_COOKIE, value);
    Ok(())
}

fn expired_cookie(name: &str, http_only: bool, secure: bool) -> String {
    format!(
        "{name}=; Path={}; SameSite=Strict; Max-Age=0{}{}",
        if http_only { "/api/v2/web/auth" } else { "/" },
        if http_only { "; HttpOnly" } else { "" },
        if secure { "; Secure" } else { "" }
    )
}

fn secure_cookies(state: &AppState) -> bool {
    public_url_is_https(state.public_url.as_deref())
}

fn public_url_is_https(public_url: Option<&str>) -> bool {
    public_url
        .and_then(|url| url::Url::parse(url).ok())
        .is_some_and(|url| url.scheme() == "https")
}

fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name && !value.is_empty()).then_some(value))
}

fn validate_web_request(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    validate_web_origin(state, headers)?;
    let cookie = cookie_value(headers, WEB_CSRF_COOKIE).ok_or(ApiError::Forbidden)?;
    let supplied = headers
        .get(WEB_CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Forbidden)?;
    if !crate::security::constant_time_bytes_eq(cookie.as_bytes(), supplied.as_bytes()) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

fn validate_web_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Forbidden)?;
    let parsed = url::Url::parse(origin).map_err(|_| ApiError::Forbidden)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ApiError::Forbidden);
    }
    if let Some(public_url) = state.public_url.as_deref() {
        let expected = url::Url::parse(public_url).map_err(|_| ApiError::Unavailable)?;
        return if parsed.origin() == expected.origin() {
            Ok(())
        } else {
            Err(ApiError::Forbidden)
        };
    }
    let authority = &parsed[url::Position::BeforeHost..url::Position::AfterPort];
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Forbidden)?;
    if authority.eq_ignore_ascii_case(host) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

/// What a route needs of the credential it was called with.
///
/// Chosen at every call of [`authenticated`], which is the only way into a
/// route, so a new route cannot be written without deciding: the compiler asks
/// the question. That is the whole reason this is a parameter rather than a
/// second helper a handler may forget to call — which is exactly what happened
/// to the scope list, stored since the foundations and read by nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Reads the caller's own catalogue and user data.
    Read,
    /// Writes on the caller's behalf: playlists, favorites, ratings, the queue,
    /// bookmarks, shares, scrobbles, scans, and issuing an OAuth code.
    Write,
    /// Acts on the instance: accounts, libraries, memberships, credentials.
    Admin,
}

/// The scope that admits the administrative routes.
const ADMIN_SCOPE: &str = "admin";
/// The scope that admits any mutation.
const WRITE_SCOPE: &str = "write";

impl Access {
    /// Whether a credential carrying `scopes` may do this.
    ///
    /// An empty list is unrestricted: a session, an OAuth grant and a token
    /// issued without scopes all carry the account's full authority, so nothing
    /// that works today stops working.
    ///
    /// A non-empty list grants only what it names, and a name this server does
    /// not know grants nothing — so `catalog:read` reads and does no more,
    /// without needing a vocabulary of every possible scope. `admin` implies
    /// `write`: a credential trusted to create accounts is not usefully barred
    /// from creating a playlist, and the surprise would be the other way round.
    fn granted_by(self, scopes: &[String]) -> bool {
        if scopes.is_empty() {
            return true;
        }
        let holds = |wanted: &str| scopes.iter().any(|scope| scope == wanted);
        match self {
            Self::Read => true,
            Self::Write => holds(WRITE_SCOPE) || holds(ADMIN_SCOPE),
            Self::Admin => holds(ADMIN_SCOPE),
        }
    }
}

/// Resolves the caller and checks, in one place, that the credential may do
/// what the route is about to do.
///
/// Both halves of administrative authority live here: an active administrator,
/// on a credential that has not been narrowed away from it. Being an
/// administrator does not widen a token, and a token cannot promote an
/// ordinary account.
async fn authenticated(
    state: &AppState,
    headers: &HeaderMap,
    access: Access,
) -> Result<crate::authentication::AuthUser, ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    let user = state
        .auth
        .authenticate(token)
        .await
        .map_err(ApiError::from)?;
    let role_ok = access != Access::Admin || user.role == crate::database::AccountRole::Admin;
    if role_ok && access.granted_by(&user.scopes) {
        Ok(user)
    } else {
        Err(ApiError::Forbidden)
    }
}

async fn mutation_context(
    state: &AppState,
    headers: &HeaderMap,
    user_id: Uuid,
) -> Result<crate::sync::MutationContext, ApiError> {
    let operation_id =
        optional_uuid_header(headers, OPERATION_ID_HEADER)?.unwrap_or_else(Uuid::new_v4);
    let origin_device_id = optional_uuid_header(headers, DEVICE_ID_HEADER)?;
    if let Some(device_id) = origin_device_id {
        let owned = state
            .sync
            .device_belongs_to_user(user_id, device_id)
            .await
            .map_err(db_error)?;
        if !owned {
            return Err(ApiError::Validation);
        }
    }
    Ok(crate::sync::MutationContext {
        operation_id,
        origin_device_id,
    })
}

fn optional_uuid_header(headers: &HeaderMap, name: &'static str) -> Result<Option<Uuid>, ApiError> {
    headers
        .get(name)
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or(ApiError::Validation)
        })
        .transpose()
}

fn db_error(error: sqlx::Error) -> ApiError {
    tracing::error!(error = %error, "catalog database operation failed");
    ApiError::Unavailable
}

fn sync_error(error: crate::sync::SyncError) -> ApiError {
    match error {
        crate::sync::SyncError::Invalid => ApiError::Validation,
        crate::sync::SyncError::Conflict => ApiError::Conflict,
        crate::sync::SyncError::CursorExpired => ApiError::CursorExpired,
        crate::sync::SyncError::Database(error) => db_error(error),
    }
}

/// Maps a domain failure onto the HTTP surface. `Forbidden` deliberately answers
/// 404 like `NotFound`: telling a caller that a resource exists but belongs to
/// someone else would leak another tenant's catalogue, which is the same
/// no-existence-leak rule the Subsonic facade applies.
fn service_error(error: crate::services::ServiceError) -> ApiError {
    use crate::services::ServiceError;
    match error {
        ServiceError::NotFound | ServiceError::Forbidden => ApiError::NotFound,
        ServiceError::Invalid => ApiError::Validation,
        ServiceError::Conflict => ApiError::Conflict,
        ServiceError::Unavailable => ApiError::Unavailable,
        ServiceError::Database(error) => db_error(error),
        ServiceError::Security(error) => {
            tracing::error!(error = %error, "catalog security operation failed");
            ApiError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{public_url_is_https, sync_notice_action, SyncNoticeAction};

    #[test]
    fn secure_cookie_detection_uses_the_parsed_url_scheme() {
        assert!(public_url_is_https(Some("HTTPS://waveflow.test/")));
        assert!(!public_url_is_https(Some("http://waveflow.test")));
        assert!(!public_url_is_https(Some("not a URL")));
        assert!(!public_url_is_https(None));
    }

    #[tokio::test]
    async fn lagged_sync_socket_recovers_from_the_durable_cursor() {
        let temp = tempfile::tempdir().unwrap();
        let config = crate::Config::for_data_dir(temp.path().join("data"));
        let db = crate::database::Database::open(&config).await.unwrap();
        db.migrate().await.unwrap();
        let sync = crate::sync::SyncService::new(db);
        let action = sync_notice_action(
            &sync,
            uuid::Uuid::new_v4(),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(3)),
        )
        .await
        .unwrap();

        assert_eq!(action, SyncNoticeAction::Send(0));
    }
}
