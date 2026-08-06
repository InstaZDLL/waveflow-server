//! M0 HTTP surface: probes, OpenAPI and local session lifecycle.

use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Response, Sse,
    },
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{authentication::AuthError, AppState};

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

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/v2/auth/login", post(login))
        .route("/api/v2/auth/refresh", post(refresh))
        .route("/api/v2/auth/logout", post(logout))
        .route("/api/v2/oauth/authorize", post(oauth_authorize))
        // No auth layer: the code plus its PKCE verifier are the credential.
        .route("/api/v2/oauth/token", post(oauth_token))
        .route("/api/v2/libraries/{library_id}/scans", post(start_scan))
        .route("/api/v2/scans/{scan_id}", get(scan_status))
        .route("/api/v2/scans/{scan_id}/events", get(scan_events))
        .route("/api/v2/libraries/{library_id}/tracks", get(list_tracks))
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
        .route("/api/v2/scrobbles", post(create_scrobble))
        .route("/api/v2/now-playing", get(list_now_playing))
        .route("/api/v2/queue", get(get_queue).put(save_queue))
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

#[utoipa::path(post, path = "/api/v2/libraries/{library_id}/scans", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 202, body = ScanQueuedResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
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

#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/tracks", tag = "catalog", params(("library_id" = Uuid, Path), ("q" = Option<String>, Query)), responses((status = 200, body = [crate::catalog::TrackRecord]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
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
    let tracks = match query.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
        Some(query) => {
            state
                .db
                .search_tracks_for_user(user.id, library_id, query)
                .await
        }
        None => state.db.list_tracks_for_user(user.id, library_id).await,
    }
    .map_err(db_error)?;
    Ok(Json(tracks))
}

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
    crate::oauth::validate_redirect_uri(&request.redirect_uri).map_err(|_| ApiError::Validation)?;
    crate::oauth::validate_challenge(&request.code_challenge_method, &request.code_challenge)
        .map_err(|_| ApiError::Validation)?;
    if request.client_id.trim().is_empty() || request.device_name.trim().is_empty() {
        return Err(ApiError::Validation);
    }

    let code = crate::security::generate_token("wfc_");
    let now = crate::authentication::now_ms();
    state
        .db
        .create_authorization(crate::database::NewAuthorization {
            code_hash: crate::security::token_hash(&code),
            user_id: user.id,
            client_id: request.client_id.trim(),
            redirect_uri: &request.redirect_uri,
            code_challenge: &request.code_challenge,
            device_name: request.device_name.trim(),
            now_ms: now,
            expires_at: now + crate::oauth::AUTHORIZATION_CODE_TTL_MS,
        })
        .await
        .map_err(db_error)?;
    Ok(Json(AuthorizeResponse {
        redirect_to: crate::oauth::redirect_with_code(
            &request.redirect_uri,
            &code,
            request.state.as_deref(),
        ),
    }))
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

#[utoipa::path(post, path = "/api/v2/playlists", tag = "user-data", request_body = CreatePlaylistRequest, responses((status = 201, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreatePlaylistRequest>,
) -> Result<(StatusCode, Json<crate::services::PlaylistItem>), ApiError> {
    let user = authenticated(&state, &headers).await?;
    let playlist = state
        .services
        .create_playlist(user.id, &request.name, &request.track_ids)
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

#[utoipa::path(patch, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), request_body = UpdatePlaylistRequest, responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdatePlaylistRequest>,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .update_playlist(
            user.id,
            playlist_id,
            request.name.as_deref(),
            request.comment.as_deref(),
            request.public,
            &request.add,
            &request.remove_indexes,
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
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .delete_playlist(user.id, playlist_id)
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

async fn set_favorite(
    state: AppState,
    headers: HeaderMap,
    entity_type: &str,
    entity_id: Uuid,
    starred: bool,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .set_star(user.id, entity_type, entity_id, starred)
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
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .set_rating(user.id, &entity_type, entity_id, request.rating)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/v2/scrobbles", tag = "user-data", request_body = ScrobbleRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn create_scrobble(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ScrobbleRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .scrobble(
            user.id,
            request.track_id,
            request.submission,
            request.played_at,
        )
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

#[utoipa::path(put, path = "/api/v2/queue", tag = "user-data", request_body = SaveQueueRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn save_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveQueueRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers).await?;
    state
        .services
        .save_queue(
            user.id,
            &request.track_ids,
            request.current,
            request.position_ms,
            request.client.as_deref(),
        )
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
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
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication failed",
            ),
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

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

async fn authenticated(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::authentication::AuthUser, ApiError> {
    let token = bearer_token(headers).ok_or(ApiError::Unauthorized)?;
    state.auth.authenticate(token).await.map_err(ApiError::from)
}

fn db_error(error: sqlx::Error) -> ApiError {
    tracing::error!(error = %error, "catalog database operation failed");
    ApiError::Unavailable
}

/// Maps a domain failure onto the HTTP surface. `Forbidden` deliberately answers
/// 404 like `NotFound`: telling a caller that a resource exists but belongs to
/// someone else would leak another tenant's catalogue, which is the same
/// no-existence-leak rule the Subsonic facade applies.
fn service_error(error: crate::services::ServiceError) -> ApiError {
    use crate::services::ServiceError;
    match error {
        ServiceError::NotFound | ServiceError::Forbidden => ApiError::NotFound,
        ServiceError::Invalid | ServiceError::Conflict => ApiError::Validation,
        ServiceError::Database(error) => db_error(error),
        ServiceError::Security(error) => {
            tracing::error!(error = %error, "catalog security operation failed");
            ApiError::Unavailable
        }
    }
}
