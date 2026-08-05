//! WaveFlow Server v2 library surface.

pub mod authentication;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod database;
pub mod http;
pub mod media;
pub mod scanner;
pub mod security;
pub mod services;
pub mod stream_ticket;
pub mod subsonic;
pub mod webui;

use std::{sync::Arc, time::Duration};

use axum::{
    extract::DefaultBodyLimit,
    http::Request,
    response::{IntoResponse, Response},
    Router,
};
use tower::ServiceBuilder;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

pub use config::Config;

pub const OPENAPI_JSON_PATH: &str = "/openapi.json";
pub const SCALAR_PATH: &str = "/reference";
const REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Clone)]
pub struct AppState {
    pub db: database::Database,
    pub auth: authentication::AuthService,
    pub secret_box: Arc<security::SecretBox>,
    pub scanner: scanner::ScanManager,
    pub media: media::MediaService,
    pub services: services::DomainServices,
    pub artwork_dir: std::path::PathBuf,
    pub instance_key_path: std::path::PathBuf,
    pub public_url: Option<String>,
    pub stream_ticket_ttl: std::time::Duration,
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "WaveFlow Server API",
        version = "2.0.0-beta.0",
        description = "Self-hosted WaveFlow music server v2.",
        license(name = "AGPL-3.0-only")
    ),
    paths(
        http::health,
        http::ready,
        http::login,
        http::refresh,
        http::logout,
        http::start_scan,
        http::scan_status,
        http::scan_events,
        http::list_tracks,
        http::list_albums,
        http::get_album,
        http::list_artists,
        http::get_artist,
        http::search_catalog,
        http::list_playlists,
        http::create_playlist,
        http::get_playlist,
        http::update_playlist,
        http::delete_playlist,
        http::list_favorites,
        http::add_favorite,
        http::remove_favorite,
        http::set_rating,
        http::create_scrobble,
        http::list_now_playing,
        http::get_queue,
        http::save_queue,
        media::stream_track,
        media::create_stream_ticket,
        media::stream_with_ticket
    ),
    components(schemas(
        http::ProbeResponse,
        http::ReadyResponse,
        http::LoginRequest,
        http::RefreshRequest,
        http::ErrorResponse,
        authentication::AuthTokens,
        authentication::AuthUser,
        database::AccountRole,
        http::ScanQueuedResponse,
        catalog::ScanJobRecord,
        catalog::TrackRecord,
        services::AlbumItem,
        services::ArtistItem,
        services::SongItem,
        services::ArtistSummary,
        services::AlbumDetail,
        services::ArtistDetail,
        services::SearchResult,
        services::PlaylistItem,
        services::QueueItem,
        http::CreatePlaylistRequest,
        http::UpdatePlaylistRequest,
        http::RatingRequest,
        http::ScrobbleRequest,
        http::SaveQueueRequest,
        http::StarredEntry,
        http::NowPlayingEntry,
        media::StreamTicketResponse,
        scanner::ScanProgress
    )),
    tags(
        (name = "probes", description = "Process and SQLite health"),
        (name = "authentication", description = "Local WaveFlow sessions")
        ,(name = "catalog", description = "Authoritative library scans and catalogue reads")
    )
)]
pub struct ApiDoc;

pub async fn initialize(config: &Config) -> anyhow::Result<AppState> {
    let db = database::Database::open(config).await?;
    db.migrate().await?;
    let secret_box = Arc::new(security::SecretBox::load_or_create(
        &config.instance_key_path,
    )?);
    let instance_key = std::fs::read(&config.instance_key_path)?;
    let fingerprint = security::bytes_hash(&instance_key);
    if !db
        .bind_instance_key(&fingerprint, authentication::now_ms())
        .await?
    {
        anyhow::bail!(
            "instance.key does not match waveflow.db; restore the database and key from the same backup bundle"
        );
    }
    let auth = authentication::AuthService::new(db.clone(), config);
    let scanner = scanner::ScanManager::new(
        db.clone(),
        config.artwork_dir.clone(),
        config.scan_parallelism,
    );
    let media = media::MediaService::initialize(config).await?;
    let services = services::DomainServices::new(db.clone(), Arc::clone(&secret_box));
    Ok(AppState {
        db,
        auth,
        secret_box,
        scanner,
        media,
        services,
        artwork_dir: config.artwork_dir.clone(),
        instance_key_path: config.instance_key_path.clone(),
        public_url: config.public_url.clone(),
        stream_ticket_ttl: config.stream_ticket_ttl,
    })
}

pub fn app(config: &Config, state: AppState) -> Router {
    let openapi = ApiDoc::openapi();
    let openapi_for_route = openapi.clone();
    let request_id_header = axum::http::HeaderName::from_static(REQUEST_ID_HEADER);
    let middleware = ServiceBuilder::new()
        .layer(SetRequestIdLayer::new(
            request_id_header.clone(),
            MakeRequestUuid,
        ))
        // Query strings may contain Subsonic credentials and public-share
        // paths contain bearer tokens. Neither may reach a trace sink.
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                let request_id = request
                    .headers()
                    .get(REQUEST_ID_HEADER)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or("");
                let path = trace_path(request.uri().path());
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    path = path,
                    request_id = %request_id
                )
            }),
        )
        .layer(PropagateRequestIdLayer::new(request_id_header));

    let ordinary = Router::new()
        .merge(http::router(state.clone()))
        .merge(Router::from(Scalar::with_url(SCALAR_PATH, openapi)))
        .route(
            OPENAPI_JSON_PATH,
            axum::routing::get(move || openapi_response(openapi_for_route.clone())),
        )
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ));

    let router = Router::new()
        .merge(ordinary)
        // Media bodies can outlive the ordinary API timeout. FFmpeg and
        // disconnected consumers are bounded by MediaService itself.
        .merge(media::router(state.clone()))
        .merge(subsonic::router(state))
        // Anything no API route claimed is served by the embedded web client:
        // a built asset, or the shell for a client-side route.
        .fallback(webui::handler)
        .layer(DefaultBodyLimit::max(16 * 1024))
        .layer(middleware);

    if config.allowed_origins.is_empty() {
        router
    } else {
        router.layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::list(config.allowed_origins.clone()))
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::RANGE,
                ])
                .expose_headers([
                    axum::http::header::ACCEPT_RANGES,
                    axum::http::header::CONTENT_LENGTH,
                    axum::http::header::CONTENT_RANGE,
                ]),
        )
    }
}

fn trace_path(path: &str) -> &str {
    if path.starts_with("/api/v2/stream/") {
        return "/api/v2/stream/{redacted}";
    }
    if path.starts_with("/share/") {
        "/share/{redacted}"
    } else {
        path
    }
}

async fn openapi_response(openapi: utoipa::openapi::OpenApi) -> Response {
    match serde_json::to_string(&openapi) {
        Ok(body) => (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(error) => {
            tracing::error!(error = %error, "OpenAPI serialization failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "OpenAPI serialization failed",
            )
                .into_response()
        }
    }
}

pub async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(error) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %error, "failed to install Ctrl-C handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(error) => tracing::error!(error = %error, "failed to install terminate handler"),
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tokio::time::sleep(Duration::from_millis(25)).await;
}

#[cfg(test)]
mod tests {
    use super::trace_path;

    #[test]
    fn trace_paths_redact_public_share_bearer_tokens() {
        assert_eq!(
            trace_path("/api/v2/stream/sealed-ticket"),
            "/api/v2/stream/{redacted}",
            "a stream ticket is a credential and must not reach a trace sink"
        );
        assert_eq!(trace_path("/share/wfs_secret"), "/share/{redacted}");
        assert_eq!(
            trace_path("/share/wfs_secret/tracks/id/stream"),
            "/share/{redacted}"
        );
        assert_eq!(trace_path("/rest/ping.view"), "/rest/ping.view");
    }
}
