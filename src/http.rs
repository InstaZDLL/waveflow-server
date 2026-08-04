//! M0 HTTP surface: probes, OpenAPI and local session lifecycle.

use std::{convert::Infallible, time::Duration};

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive},
        IntoResponse, Response, Sse,
    },
    routing::{get, post},
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

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/api/v2/auth/login", post(login))
        .route("/api/v2/auth/refresh", post(refresh))
        .route("/api/v2/auth/logout", post(logout))
        .route("/api/v2/libraries/{library_id}/scans", post(start_scan))
        .route("/api/v2/scans/{scan_id}", get(scan_status))
        .route("/api/v2/scans/{scan_id}/events", get(scan_events))
        .route("/api/v2/libraries/{library_id}/tracks", get(list_tracks))
        .route("/api/v2/albums", get(list_albums))
        .route("/api/v2/albums/{album_id}", get(get_album))
        .route("/api/v2/artists", get(list_artists))
        .route("/api/v2/artists/{artist_id}", get(get_artist))
        .route("/api/v2/search", get(search_catalog))
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
