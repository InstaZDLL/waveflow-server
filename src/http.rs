//! M0 HTTP surface: probes, OpenAPI and local session lifecycle.

use std::{convert::Infallible, time::Duration};

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

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
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
}

/// Builds the application router with health, authentication, catalog, user-data, synchronization, and administration endpoints.
///
/// # Examples
///
/// ```no_run
/// # use crate::{router, AppState};
/// # let state: AppState = todo!();
/// let app = router(state);
/// ```
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
        .route("/api/v2/albums", get(list_albums))
        .route("/api/v2/albums/{album_id}", get(get_album))
        .route("/api/v2/artists", get(list_artists))
        .route("/api/v2/artists/{artist_id}", get(get_artist))
        .route("/api/v2/search", get(search_catalog))
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

/// Checks whether the database is available for serving requests.
///
/// Responds with `200 OK` when the database is reachable, or `503 Service Unavailable`
/// when the database probe fails.
///
/// # Examples
///
/// ```no_run
/// let response = ready(State(state)).await;
/// assert!(response.status().is_success());
/// ```
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

/// Reports whether initial application setup is required.
///
/// # Examples
///
/// ```no_run
/// let response = setup_status(state).await?;
/// assert!(response.0.required || !response.0.required);
/// # Ok::<(), ApiError>(())
/// ```
#[utoipa::path(get, path = "/api/v2/setup", tag = "authentication", responses((status = 200, body = SetupStatusResponse)))]
pub async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    let required = state.db.setup_required().await.map_err(db_error)?;
    Ok(Json(SetupStatusResponse { required }))
}

/// Creates the initial administrator account during application setup.
///
/// The request must include a valid browser origin and administrator credentials.
///
/// # Returns
///
/// Returns HTTP 201 with the newly created administrator's user ID.
///
/// # Examples
///
/// ```no_run
/// # async fn example(state: AppState, headers: axum::http::HeaderMap) {
/// let request = axum::Json(SetupRequest {
///     username: "admin".to_owned(),
///     password: "change-me".to_owned(),
/// });
/// let result = setup(axum::extract::State(state), headers, request).await;
/// # }
/// ```
///
/// #[utoipa::path(post, path = "/api/v2/setup", tag = "authentication", params(("Origin" = String, Header, description = "Required browser origin")), request_body = SetupRequest, responses((status = 201, body = SetupResponse), (status = 403, description = "Origin header missing or rejected", body = ErrorResponse), (status = 422, body = ErrorResponse)))]
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

/// Revokes the authenticated bearer session.
///
/// The request must include a non-empty `Bearer` authorization value. On
/// success, the handler returns HTTP 204.
///
/// # Examples
///
/// ```text
/// POST /api/v2/auth/logout
/// Authorization: Bearer <access-token>
///
/// HTTP/1.1 204 No Content
/// ```
///
/// An absent or invalid bearer token produces HTTP 401. Authentication
/// service failures produce HTTP 503.
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

/// Authenticates a browser session and establishes refresh and CSRF cookies.
///
/// The response contains a short-lived access token, while the rotating refresh
/// token is stored in an HttpOnly, same-site cookie.
///
/// # Examples
///
/// ```no_run
/// // POST /api/v2/web/auth/login with username, password, and device_name.
/// ```
///
/// # Returns
///
/// The access-token response with authentication cookies attached.
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

/// Refreshes a browser session using the refresh-token cookie after validating the web request.
///
/// # Examples
///
/// ```no_run
/// // Send a POST request to `/api/v2/web/auth/refresh` with the refresh-token
/// // cookie and matching CSRF headers.
/// # let _endpoint = "/api/v2/web/auth/refresh";
/// ```
///
/// Returns an authentication response and refreshed cookies on success. Requests
/// with invalid origin, CSRF credentials, or refresh tokens are rejected.
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

/// Logs out the browser session associated with the refresh cookie and expires the session cookies.
///
/// # Examples
///
/// ```
/// let endpoint = "/api/v2/web/auth/logout";
/// assert_eq!(endpoint, "/api/v2/web/auth/logout");
/// ```
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

/// Queues a manual scan for a library accessible to the authenticated user.
///
/// # Errors
///
/// Returns an authentication error when the request lacks valid credentials, `ApiError::NotFound` when the library is unavailable to the user, or `ApiError::Unavailable` when the scan cannot be queued.
///
/// # Examples
///
/// ```text
/// POST /api/v2/libraries/{library_id}/scans
/// Authorization: Bearer <access-token>
/// ```
pub async fn start_scan(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<ScanQueuedResponse>), ApiError> {
    let user = authenticated(&state, &headers).await?;
    let library = state
        .db
        .library_for_user(user.id, library_id)
        .await
        .map_err(db_error)?
        .ok_or(ApiError::NotFound)?;
    let scan_id = state
        .scanner
        .trigger(library, Some(user.id), "manual")
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "scan queue failed");
            ApiError::Unavailable
        })?;
    Ok((StatusCode::ACCEPTED, Json(ScanQueuedResponse { scan_id })))
}

/// Lists the libraries accessible to the authenticated user.
///
/// # Returns
///
/// The libraries available to the authenticated user.
///
/// # Examples
///
/// ```ignore
/// let response = client
///     .get("/api/v2/libraries")
///     .bearer_auth(access_token)
///     .send()
///     .await?;
/// assert!(response.status().is_success());
/// ```
pub async fn list_libraries(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::catalog::LibraryAccess>>, ApiError> {
    let user = authenticated(&state, &headers).await?;
    state
        .db
        .libraries_for_user(user.id)
        .await
        .map(Json)
        .map_err(db_error)
}

/// Creates a library for the authenticated administrator and queues its initial scan.
///
/// The library path must identify an existing, non-symlink directory, and the library name
/// must not be empty or consist only of whitespace.
///
/// # Examples
///
/// ```text
/// POST /api/v2/libraries
/// {"name":"Music","path":"/srv/music","visibility":"private"}
/// ```
///
/// # Returns
///
/// The created library ID and the ID of its queued initial scan.
#[utoipa::path(post, path = "/api/v2/libraries", tag = "administration", request_body = CreateLibraryRequest, responses((status = 201, body = CreateLibraryResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_library(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateLibraryRequest>,
) -> Result<(StatusCode, Json<CreateLibraryResponse>), ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
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

/// Assigns a library role to a user.
///
/// The caller must be an administrator. The library and user must exist, and the
/// owner role cannot be assigned.
///
/// # Examples
///
/// ```
/// let path = "/api/v2/libraries/{library_id}/members/{user_id}";
/// assert!(path.contains("/libraries/"));
/// assert!(path.contains("/members/"));
/// ```
///
/// Returns [`StatusCode::NO_CONTENT`] when the membership is updated.
/// Returns [`ApiError::Validation`] for an owner role or unknown library or user.
pub async fn set_library_member(
    State(state): State<AppState>,
    Path((library_id, user_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<SetLibraryMemberRequest>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
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

/// Removes a user’s membership from a library.
///
/// Returns `404 Not Found` when the user is not a member of the library.
///
/// # Examples
///
/// ```ignore
/// let status = remove_library_member(state, (library_id, user_id), headers).await?;
/// assert_eq!(status, StatusCode::NO_CONTENT);
/// ```
#[utoipa::path(delete, path = "/api/v2/libraries/{library_id}/members/{user_id}", tag = "administration", params(("library_id" = Uuid, Path), ("user_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn remove_library_member(
    State(state): State<AppState>,
    Path((library_id, user_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
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

/// Retrieves a scan job visible to the authenticated user.
///
/// # Examples
///
/// ```ignore
/// let response = client
///     .get(format!("/api/v2/scans/{scan_id}"))
///     .send()
///     .await?;
/// ```
///
/// # Errors
///
/// Returns an unauthorized error when authentication fails, a not-found error
/// when the scan is unavailable to the user, or a database error when lookup
/// fails.
#[utoipa::path(get, path = "/api/v2/scans/{scan_id}", tag = "catalog", params(("scan_id" = Uuid, Path)), responses((status = 200, body = crate::catalog::ScanJobRecord), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn scan_status(
    State(state): State<AppState>,
    Path(scan_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::catalog::ScanJobRecord>, ApiError> {
    let user = authenticated(&state, &headers).await?;
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
    let user = authenticated(&state, &headers).await?;
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

/// Lists the tracks in a library that the authenticated user can access.
///
/// The optional search query is trimmed before filtering. Results use an offset
/// and a limit between 1 and 500; the default offset is 0 and the default
/// limit is 500.
///
/// # Examples
///
/// ```no_run
/// # async fn example(client: reqwest::Client, base_url: &str, library_id: &str) {
/// let response = client
///     .get(format!("{base_url}/api/v2/libraries/{library_id}/tracks?limit=100"))
///     .send()
///     .await
///     .unwrap();
/// assert!(response.status().is_success());
/// # }
/// ```
///
/// Returns the matching track records, or an API error when authentication,
/// library access, pagination, or database lookup fails.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/tracks", tag = "catalog", params(("library_id" = Uuid, Path), ("q" = Option<String>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::catalog::TrackRecord]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_tracks(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    Query(query): Query<TrackQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::catalog::TrackRecord>>, ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Retrieves a catalog track visible to the authenticated user.
///
/// # Examples
///
/// ```no_run
/// # use uuid::Uuid;
/// # let track_id = Uuid::nil();
/// // Request: GET /api/v2/tracks/{track_id}
/// let _track_id = track_id;
/// ```
///
/// Returns the requested track, or `ApiError::NotFound` when it is unavailable to the user.
///
/// # Errors
///
/// Returns an authentication error when the request lacks valid credentials.
#[utoipa::path(get, path = "/api/v2/tracks/{track_id}", tag = "catalog", params(("track_id" = Uuid, Path)), responses((status = 200, body = crate::services::SongItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_track(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::SongItem>, ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Lists albums accessible to the authenticated user, optionally filtered by library and paginated.
///
/// # Arguments
///
/// * `library_id` — Restricts results to a specific library.
/// * `offset` — Number of albums to skip.
/// * `limit` — Maximum number of albums to return.
///
/// # Returns
///
/// The accessible albums for the requested page.
///
/// # Examples
///
/// ```
/// let path = "/api/v2/albums?offset=0&limit=20";
/// assert!(path.starts_with("/api/v2/albums"));
/// ```
#[utoipa::path(get, path = "/api/v2/albums", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::AlbumItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_albums(
    State(state): State<AppState>,
    Query(query): Query<BrowseQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::AlbumItem>>, ApiError> {
    let user = authenticated(&state, &headers).await?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .list_albums(user.id, query.library_id, page)
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
    let user = authenticated(&state, &headers).await?;
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
    let user = authenticated(&state, &headers).await?;
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
    let user = authenticated(&state, &headers).await?;
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
    let user = authenticated(&state, &headers).await?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .search(user.id, &query.q, page)
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
    let user = authenticated(&state, &headers).await?;
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
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .playlists(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Creates a playlist for the authenticated user.
///
/// # Examples
///
/// ```
/// let request = CreatePlaylistRequest {
///     name: "Favorites".to_owned(),
///     track_ids: Vec::new(),
/// };
/// assert_eq!(request.name, "Favorites");
/// ```
///
/// Returns `201 Created` with the new playlist, or an error when authentication,
/// authorization, validation, or playlist creation fails.
#[utoipa::path(post, path = "/api/v2/playlists", tag = "user-data", request_body = CreatePlaylistRequest, responses((status = 201, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreatePlaylistRequest>,
) -> Result<(StatusCode, Json<crate::services::PlaylistItem>), ApiError> {
    let user = authenticated(&state, &headers).await?;
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
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .playlist(user.id, playlist_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Updates a playlist's metadata and track membership for the authenticated user.
///
/// # Examples
///
/// ```ignore
/// let playlist = update_playlist(state, playlist_id, headers, request)
///     .await
///     .expect("playlist update should succeed");
/// assert_eq!(playlist.0.id, playlist_id);
/// ```
///
/// Returns the updated playlist.
pub async fn update_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdatePlaylistRequest>,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers).await?;
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
            context,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

/// Deletes a playlist owned by the authenticated user.
///
/// # Examples
///
/// ```
/// use axum::http::StatusCode;
///
/// assert_eq!(StatusCode::NO_CONTENT, StatusCode::from_u16(204).unwrap());
/// ```
#[utoipa::path(delete, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
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
    let user = authenticated(&state, &headers).await?;
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

/// Sets or clears a user's favorite status for an entity.
///
/// # Examples
///
/// ```no_run
/// let status = set_favorite(state, headers, "track", entity_id, true).await?;
/// assert_eq!(status, StatusCode::NO_CONTENT);
/// # Ok::<(), ApiError>(())
/// ```
///
/// `entity_type` identifies the kind of entity being updated.
async fn set_favorite(
    state: AppState,
    headers: HeaderMap,
    entity_type: &str,
    entity_id: Uuid,
    starred: bool,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .set_star_with_context(user.id, entity_type, entity_id, starred, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sets a user's rating for a track, album, or artist.
///
/// `entity_type` must identify a supported entity kind, and `rating` is supplied
/// in the request body.
///
/// # Returns
///
/// `StatusCode::NO_CONTENT` when the rating is saved.
///
/// # Examples
///
/// ```no_run
/// // PUT /api/v2/ratings/track/{entity_id}
/// // {"rating": 5}
/// ```
#[utoipa::path(put, path = "/api/v2/ratings/{entity_type}/{entity_id}", tag = "user-data", params(("entity_type" = String, Path, description = "track, album or artist"), ("entity_id" = Uuid, Path)), request_body = RatingRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_rating(
    State(state): State<AppState>,
    Path((entity_type, entity_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<RatingRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .set_rating_with_context(user.id, &entity_type, entity_id, request.rating, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Lists ratings for the authenticated user.
///
/// # Returns
///
/// The user's ratings, or an API error if authentication or retrieval fails.
///
/// # Examples
///
/// ```
/// let endpoint = "/api/v2/ratings";
/// assert_eq!(endpoint, "/api/v2/ratings");
/// ```
pub async fn list_ratings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::RatingItem>>, ApiError> {
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .ratings(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Records a track playback or submission event for the authenticated user.
///
/// # Examples
///
/// ```ignore
/// let status = create_scrobble(state, headers, Json(request)).await?;
/// assert_eq!(status, StatusCode::NO_CONTENT);
/// # Ok::<(), ApiError>(())
/// ```
///
/// # Returns
///
/// `StatusCode::NO_CONTENT` when the scrobble is recorded successfully.
pub async fn create_scrobble(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ScrobbleRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Lists the authenticated user's listening history with an optional result limit.
///
/// The limit defaults to 200 and must be between 1 and the maximum synchronization
/// limit.
///
/// # Examples
///
/// ```text
/// GET /api/v2/history?limit=50
/// ```
///
/// The response contains the user's history entries.
///
/// # Errors
///
/// Returns an authentication error for unauthenticated requests or a validation
/// error when the limit is outside the allowed range.
#[utoipa::path(get, path = "/api/v2/history", tag = "user-data", params(("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::HistoryItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<crate::services::HistoryItem>>, ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Reports whether media transcoding is available and how many transcodes are currently active.
///
/// Authentication is required.
///
/// # Examples
///
/// ```text
/// GET /api/v2/transcode/status
/// Authorization: Bearer <access-token>
/// ```
///
/// The response contains `available` and `active` fields.
///
/// # Errors
///
/// Returns an authentication error when the request does not include valid credentials.
///
#[utoipa::path(get, path = "/api/v2/transcode/status", tag = "catalog", responses((status = 200, body = TranscodeStatusResponse), (status = 401, body = ErrorResponse)))]
pub async fn transcode_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TranscodeStatusResponse>, ApiError> {
    authenticated(&state, &headers).await?;
    Ok(Json(TranscodeStatusResponse {
        available: state.media.transcoding_available(),
        active: state.media.active_transcodes(),
    }))
}

/// Lists all users for an authenticated administrator.
///
/// # Returns
///
/// The users configured in the application.
///
/// # Examples
///
/// ```no_run
/// let users = list_users(state, headers).await?;
/// assert!(!users.0.is_empty());
/// # Ok::<(), ApiError>(())
/// ```
#[utoipa::path(get, path = "/api/v2/admin/users", tag = "administration", responses((status = 200, body = [crate::services::UserItem]), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse)))]
pub async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::UserItem>>, ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
    state
        .services
        .users(actor.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Creates a web user after verifying that the authenticated actor is an administrator.
///
/// # Examples
///
/// ```no_run
/// // POST /api/v2/admin/users with a `CreateUserRequest` JSON body.
/// ```
///
/// # Returns
///
/// The created user and HTTP status `201 Created`.
#[utoipa::path(post, path = "/api/v2/admin/users", tag = "administration", request_body = CreateUserRequest, responses((status = 201, body = crate::services::UserItem), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateUserRequest>,
) -> Result<(StatusCode, Json<crate::services::UserItem>), ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
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

/// Updates an existing user's role, account status, library access, and credentials.
///
/// The caller must be authenticated as an administrator.
///
/// # Parameters
///
/// * `username` - Username of the account to update.
/// * `request` - Account fields to change; omitted fields retain their current values.
///
/// # Returns
///
/// The updated user account.
///
/// # Examples
///
/// ```no_run
/// # use axum::{extract::{Path, State}, Json};
/// # use axum::http::HeaderMap;
/// # async fn example(state: AppState, headers: HeaderMap) {
/// let request = UpdateUserRequest {
///     role: None,
///     disabled: Some(false),
///     library_ids: None,
///     subsonic_password: None,
///     web_password: None,
/// };
///
/// let result = update_user(
///     State(state),
///     Path(String::from("alice")),
///     headers,
///     Json(request),
/// ).await;
/// # }
/// ```
#[utoipa::path(patch, path = "/api/v2/admin/users/{username}", tag = "administration", params(("username" = String, Path)), request_body = UpdateUserRequest, responses((status = 200, body = crate::services::UserItem), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<crate::services::UserItem>, ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
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

/// Deletes the specified user account when requested by an administrator.
///
/// # Errors
///
/// Returns an error if authentication fails, the authenticated user is not an
/// administrator, or the user does not exist.
///
/// # Examples
///
/// ```no_run
/// // Send an authenticated DELETE request to:
/// // DELETE /api/v2/admin/users/alice
/// ```
pub async fn delete_user(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
    state
        .services
        .delete_user(actor.id, &username)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Creates or replaces a user's Subsonic credential.
///
/// # Examples
///
/// ```no_run
/// # async fn example() -> Result<(), ApiError> {
/// let response = set_subsonic_credential(state, username, headers, request).await?;
/// assert!(!response.0.api_key.is_empty());
/// # Ok(())
/// # }
/// ```
///
/// The generated API key is returned in the response.
///
/// # Errors
///
/// Returns an error if authentication, authorization, credential creation, or request validation fails.
#[utoipa::path(put, path = "/api/v2/admin/users/{username}/subsonic-credential", tag = "administration", params(("username" = String, Path)), request_body = SetSubsonicCredentialRequest, responses((status = 200, body = SubsonicCredentialResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_subsonic_credential(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
    Json(request): Json<SetSubsonicCredentialRequest>,
) -> Result<Json<SubsonicCredentialResponse>, ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
    let api_key = state
        .services
        .set_subsonic_credential(actor.id, &username, &request.password)
        .await
        .map_err(service_error)?;
    Ok(Json(SubsonicCredentialResponse { api_key }))
}

/// Revokes the specified user's Subsonic credential.
///
/// # Arguments
///
/// * `username` - Username whose Subsonic credential should be revoked.
///
/// # Returns
///
/// The HTTP `204 No Content` status on success.
///
/// # Examples
///
/// ```text
/// DELETE /api/v2/admin/users/alice/subsonic-credential
/// ```
#[utoipa::path(delete, path = "/api/v2/admin/users/{username}/subsonic-credential", tag = "administration", params(("username" = String, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn revoke_subsonic_credential(
    State(state): State<AppState>,
    Path(username): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers).await?;
    require_admin(&actor)?;
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
    let user = authenticated(&state, &headers).await?;
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
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .queue(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Saves the authenticated user's playback queue.
///
/// # Examples
///
/// ```no_run
/// # async fn example(state: AppState, headers: HeaderMap, request: SaveQueueRequest) {
/// let status = save_queue(
///     axum::extract::State(state),
///     headers,
///     axum::Json(request),
/// )
/// .await
/// .unwrap();
/// assert_eq!(status, axum::http::StatusCode::NO_CONTENT);
/// # }
/// ```
///
/// # Returns
///
/// `StatusCode::NO_CONTENT` when the queue is saved successfully.
#[utoipa::path(put, path = "/api/v2/queue", tag = "user-data", request_body = SaveQueueRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn save_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveQueueRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Lists the authenticated user's shares.
///
/// # Returns
///
/// The user's shares as API response objects.
///
/// # Examples
///
/// ```no_run
/// # async fn example(state: AppState, headers: HeaderMap) {
/// let Json(shares) = list_shares(State(state), headers).await.unwrap();
/// # }
/// ```
#[utoipa::path(get, path = "/api/v2/shares", tag = "user-data", responses((status = 200, body = [ShareResponse]), (status = 401, body = ErrorResponse)))]
pub async fn list_shares(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ShareResponse>>, ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Creates a share for the requested tracks.
///
/// # Examples
///
/// ```no_run
/// // Submit a POST request to `/api/v2/shares` with the selected track IDs.
/// ```
///
/// # Returns
///
/// A `201 Created` response containing the created share.
pub async fn create_share(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateShareRequest>,
) -> Result<(StatusCode, Json<ShareResponse>), ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Updates a share owned by the authenticated user.
///
/// # Examples
///
/// ```no_run
/// // PATCH /api/v2/shares/{share_id}
/// // JSON body: { "description": "Shared playlist", "expires_at": null }
/// ```
#[utoipa::path(patch, path = "/api/v2/shares/{share_id}", tag = "user-data", params(("share_id" = Uuid, Path)), request_body = UpdateShareRequest, responses((status = 200, body = ShareResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn update_share(
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdateShareRequest>,
) -> Result<Json<ShareResponse>, ApiError> {
    let user = authenticated(&state, &headers).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let share = state
        .services
        .update_share_with_context(
            user.id,
            share_id,
            request.description.as_deref(),
            request.expires_at,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(Json(share_response(&state, share)))
}

/// Deletes a share owned by the authenticated user.
///
/// Returns `204 No Content` when the share is deleted, or an API error when
/// authentication fails or the share is unavailable.
///
/// # Examples
///
/// ```no_run
/// let share_id = uuid::Uuid::new_v4();
/// let endpoint = format!("/api/v2/shares/{share_id}");
/// assert!(endpoint.contains(&share_id.to_string()));
/// ```
#[utoipa::path(delete, path = "/api/v2/shares/{share_id}", tag = "user-data", params(("share_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_share(
    State(state): State<AppState>,
    Path(share_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .delete_share_with_context(user.id, share_id, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Converts a service share into its API representation, including its public URL when available.
///
/// # Examples
///
/// ```ignore
/// let response = share_response(&state, share);
/// assert_eq!(response.id, share_id);
/// ```
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

/// Retrieves durable synchronization changes after a cursor.
///
/// `after` defaults to the beginning of the change log, and `limit` defaults to
/// the standard synchronization page size. The limit must be greater than zero
/// and no greater than the maximum synchronization page size.
///
/// # Examples
///
/// ```text
/// GET /api/v2/sync/changes?after=42&limit=100
/// ```
///
/// # Returns
///
/// A page of synchronization changes after the requested cursor.
#[utoipa::path(
get,
path = "/api/v2/sync/changes",
tag = "sync",
params(("after" = Option<i64>, Query), ("limit" = Option<i64>, Query)),
responses(
(status = 200, body = crate::sync::SyncPage),
(status = 401, body = ErrorResponse),
(status = 422, body = ErrorResponse)
)
)]
pub async fn sync_changes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SyncQuery>,
) -> Result<Json<crate::sync::SyncPage>, ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Retrieves the authenticated user's synchronization snapshot.
///
/// # Examples
///
/// ```no_run
/// let result = sync_snapshot(
///     axum::extract::State(todo!()),
///     axum::http::HeaderMap::new(),
/// ).await;
/// assert!(result.is_ok());
/// ```
///
/// The snapshot includes the synchronization cursor, playlists, favorites, ratings,
/// queue, listening history, and shares.
pub async fn sync_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SyncSnapshot>, ApiError> {
    let user = authenticated(&state, &headers).await?;
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
    }))
}

/// Records the synchronization cursor acknowledged by a device.
///
/// # Errors
///
/// Returns a validation error if the synchronization service rejects the
/// acknowledgement.
///
/// # Examples
///
/// A client acknowledges a cursor with a request such as:
///
/// ```text
/// PUT /api/v2/sync/ack
/// Content-Type: application/json
///
/// {"device_id":"<device-id>","cursor":42}
/// ```
///
/// On success, the endpoint responds with `204 No Content`.
pub async fn sync_ack(
pub async fn sync_ack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SyncAckRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
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

/// Upgrades an authenticated request to a WebSocket that delivers synchronization cursor notifications.
///
/// Clients should retrieve durable changes after receiving a notification; WebSocket delivery is only a wake-up signal.
///
/// # Examples
///
/// ```no_run
/// # use axum::extract::Query;
/// # let query = Query(SyncQuery { after: Some(0) });
/// # let _ = query;
/// ```
///
/// # Errors
///
/// Returns a validation error when `after` is negative and an authentication error when the request lacks valid credentials.
///
/// #[utoipa::path(
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
    let user = authenticated(&state, &headers).await?;
    let after = query.after.unwrap_or(0);
    if after < 0 {
        return Err(ApiError::Validation);
    }
    Ok(upgrade
        .on_upgrade(move |socket| serve_sync_socket(socket, state, user.id, after))
        .into_response())
}

/// Serves a synchronization WebSocket for an authenticated user.
///
/// Sends updates newer than the supplied cursor, forwards subsequent synchronization
/// notifications, responds to WebSocket control frames, and closes when the connection
/// or synchronization subscription ends.
///
/// # Examples
///
/// ```no_run
/// # async fn example(socket: WebSocket, state: AppState, user_id: Uuid) {
/// serve_sync_socket(socket, state, user_id, 0).await;
/// # }
/// ```
async fn serve_sync_socket(socket: WebSocket, state: AppState, user_id: Uuid, after: i64) {
    let (mut sender, mut receiver) = socket.split();
    let mut notices = state.sync.subscribe();
    if let Ok(cursor) = state.sync.latest_cursor(user_id).await {
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

/// Classifies a synchronization notice for a user connection.
///
/// User-specific notices produce a cursor notification, unrelated notices are
/// skipped, lagged subscriptions recover using the user's latest durable
/// cursor, and closed subscriptions terminate the connection.
///
/// # Examples
///
/// ```ignore
/// let action = sync_notice_action(&sync, user_id, notice).await?;
/// assert!(matches!(action, SyncNoticeAction::Send(_) | SyncNoticeAction::Continue));
/// # Ok::<(), sqlx::Error>(())
/// ```
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
            .latest_cursor(user_id)
            .await
            .map(SyncNoticeAction::Send),
        Err(tokio::sync::broadcast::error::RecvError::Closed) => Ok(SyncNoticeAction::Close),
    }
}

/// Sends a synchronization cursor notification through a WebSocket connection.
///
/// # Parameters
///
/// * `sender` - WebSocket sink used to deliver the notification.
/// * `cursor` - Durable synchronization cursor to include in the notification.
///
/// # Returns
///
/// `Ok(())` when the notification is sent successfully; otherwise, the WebSocket send error.
///
/// # Examples
///
/// ```no_run
/// # use axum::extract::ws::{Message, WebSocket};
/// # use futures_util::stream::SplitSink;
/// # async fn example(
/// #     sender: &mut SplitSink<WebSocket, Message>,
/// # ) -> Result<(), axum::Error> {
/// send_sync_notice(sender, 42).await?;
/// # Ok(())
/// # }
/// ```
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
    /// Converts the API error into an HTTP response with its status code and error payload.
    ///
    /// # Examples
    ///
    /// ```
    /// let response = ApiError::NotFound.into_response();
    ///
    /// assert_eq!(response.status(), StatusCode::NOT_FOUND);
    /// ```
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

/// Builds the web authentication response and sets the refresh-token and CSRF cookies.
///
/// # Examples
///
/// ```no_run
/// let response = web_auth_response(&state, &headers, tokens)?;
/// # Ok::<(), ApiError>(())
/// ```
///
/// The refresh token is stored in an `HttpOnly` cookie, while the CSRF token is
/// returned in a cookie available to browser scripts. Both cookies use the
/// configured token lifetime and HTTPS security settings.
///
/// # Returns
///
/// The authentication response containing the access-token payload and
/// authentication cookies.
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

/// Appends a `Set-Cookie` header to an HTTP response.
///
/// Returns an error when the cookie value cannot be represented as an HTTP header value.
///
/// # Examples
///
/// ```
/// let mut response = Response::new(Body::empty());
/// append_cookie(&mut response, "session=abc".to_owned()).unwrap();
///
/// assert_eq!(
///     response.headers().get("set-cookie").unwrap(),
///     "session=abc"
/// );
/// ```
fn append_cookie(response: &mut Response, value: String) -> Result<(), ApiError> {
    let value = HeaderValue::from_str(&value).map_err(|_| ApiError::Unavailable)?;
    response.headers_mut().append(header::SET_COOKIE, value);
    Ok(())
}

/// Builds a `Set-Cookie` value that immediately expires the named cookie.
///
/// HttpOnly cookies use the web-auth path; other cookies use the root path.
/// The resulting cookie may also include `HttpOnly` and `Secure` attributes.
///
/// # Examples
///
/// ```
/// assert_eq!(
///     expired_cookie("session", true, true),
///     "session=; Path=/api/v2/web/auth; SameSite=Strict; Max-Age=0; HttpOnly; Secure"
/// );
/// ```
fn expired_cookie(name: &str, http_only: bool, secure: bool) -> String {
    format!(
        "{name}=; Path={}; SameSite=Strict; Max-Age=0{}{}",
        if http_only { "/api/v2/web/auth" } else { "/" },
        if http_only { "; HttpOnly" } else { "" },
        if secure { "; Secure" } else { "" }
    )
}

/// Determines whether cookies should be marked as secure based on the configured public URL.
///
/// # Examples
///
/// ```
/// let state = AppState {
///     public_url: Some("https://example.com".to_owned()),
///     ..Default::default()
/// };
///
/// assert!(secure_cookies(&state));
/// ```
fn secure_cookies(state: &AppState) -> bool {
    public_url_is_https(state.public_url.as_deref())
}

/// Determines whether a configured public URL uses HTTPS.
///
/// # Examples
///
/// ```
/// assert!(public_url_is_https(Some("https://example.com")));
/// assert!(!public_url_is_https(Some("http://example.com")));
/// assert!(!public_url_is_https(None));
/// ```
fn public_url_is_https(public_url: Option<&str>) -> bool {
    public_url
        .and_then(|url| url::Url::parse(url).ok())
        .is_some_and(|url| url.scheme() == "https")
}

/// Extracts a non-empty cookie value by name from the request headers.
///
/// # Examples
///
/// ```
/// use axum::http::{header, HeaderMap, HeaderValue};
///
/// let mut headers = HeaderMap::new();
/// headers.insert(header::COOKIE, HeaderValue::from_static("session=abc123"));
///
/// assert_eq!(cookie_value(&headers, "session"), Some("abc123"));
/// ```
fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name && !value.is_empty()).then_some(value))
}

/// Validates the request origin and CSRF credentials for a browser-authenticated request.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] when the origin, CSRF cookie, or CSRF header is
/// missing or invalid.
///
/// # Examples
///
/// ```rust,ignore
/// let result = validate_web_request(&state, &headers);
/// assert!(result.is_ok());
/// ```
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

/// Validates that a web request has an acceptable origin.
///
/// The origin must use HTTP or HTTPS, contain no path, query, or fragment, and
/// match the configured public origin or the request's `Host` header.
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] when the origin is missing, malformed, or
/// does not match the expected host. Returns [`ApiError::Unavailable`] when
/// the configured public URL is invalid.
///
/// # Examples
///
/// ```no_run
/// # let state: AppState = todo!();
/// let headers = axum::http::HeaderMap::new();
/// validate_web_origin(&state, &headers)?;
/// # Ok::<(), ApiError>(())
/// ```
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

/// Extracts a non-empty bearer token from the `Authorization` header.
///
/// # Examples
///
/// ```
/// use http::{header, HeaderMap, HeaderValue};
///
/// let mut headers = HeaderMap::new();
/// headers.insert(
///     header::AUTHORIZATION,
///     HeaderValue::from_static("Bearer example-token"),
/// );
///
/// assert_eq!(bearer_token(&headers), Some("example-token"));
/// ```
///
/// # Returns
///
/// The bearer token when the header contains a non-empty `Bearer ` value, or `None` otherwise.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

/// Authenticates a request using its bearer token.
///
/// Returns an unauthorized error when the request has no bearer token or when authentication fails.
///
/// # Examples
///
/// ```ignore
/// let user = authenticated(&state, &headers).await?;
/// ```
async fn authenticated(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::authentication::AuthUser, ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    state.auth.authenticate(token).await.map_err(ApiError::from)
}

/// Ensures that the authenticated user has administrator privileges.
///
/// # Examples
///
/// ```ignore
/// require_admin(&user)?;
/// ```
///
/// # Errors
///
/// Returns [`ApiError::Forbidden`] when the user does not have the administrator role.
fn require_admin(user: &crate::authentication::AuthUser) -> Result<(), ApiError> {
    if user.role == crate::database::AccountRole::Admin {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

/// Builds the mutation context for an authenticated user's request, validating any supplied device identifier.
///
/// # Errors
///
/// Returns [`ApiError::Validation`] when the specified device does not belong to the user.
///
/// # Examples
///
/// ```no_run
/// let context = mutation_context(&state, &headers, user_id).await?;
/// assert!(context.origin_device_id.is_none());
/// # Ok::<(), ApiError>(())
/// ```
async fn mutation_context(
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

/// Parses an optional UUID from an HTTP header.
///
/// An absent header produces `None`. A present header must contain a valid UUID;
/// otherwise, the function returns a validation error.
///
/// # Examples
///
/// ```
/// use axum::http::{HeaderMap, HeaderValue};
/// use uuid::Uuid;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("x-device-id", HeaderValue::from_static(
///     "550e8400-e29b-41d4-a716-446655440000",
/// ));
///
/// let device_id = optional_uuid_header(&headers, "x-device-id").unwrap();
/// assert_eq!(
///     device_id,
///     Some(Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap())
/// );
/// ```
///
/// # Errors
///
/// Returns `ApiError::Validation` when the header value is not valid UTF-8 or
/// does not contain a valid UUID.
///
/// # Returns
///
/// The parsed UUID when the header is present and valid, or `None` when the
/// header is absent.
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

/// Converts a database error into an unavailable API error.
///
/// # Examples
///
/// ```
/// let error = sqlx::Error::RowNotFound;
/// assert!(matches!(db_error(error), ApiError::Unavailable));
/// ```
fn db_error(error: sqlx::Error) -> ApiError {
    tracing::error!(error = %error, "catalog database operation failed");
    ApiError::Unavailable
}

/// Converts a synchronization error into the corresponding API error.
///
/// # Examples
///
/// ```
/// let error = sync_error(crate::sync::SyncError::Invalid);
/// assert!(matches!(error, ApiError::Validation));
/// ```
fn sync_error(error: crate::sync::SyncError) -> ApiError {
    match error {
        crate::sync::SyncError::Invalid => ApiError::Validation,
        crate::sync::SyncError::Conflict => ApiError::Validation,
        crate::sync::SyncError::Database(error) => db_error(error),
    }
}

/// Converts service-layer failures into API errors while hiding whether a forbidden resource exists.
///
/// Forbidden resources are reported as not found to prevent resource-existence leaks.
///
/// # Examples
///
/// ```
/// let error = service_error(crate::services::ServiceError::Forbidden);
/// assert!(matches!(error, ApiError::NotFound));
/// ```
fn service_error(error: crate::services::ServiceError) -> ApiError {
    use crate::services::ServiceError;
    match error {
        ServiceError::NotFound | ServiceError::Forbidden => ApiError::NotFound,
        ServiceError::Invalid | ServiceError::Conflict => ApiError::Validation,
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
