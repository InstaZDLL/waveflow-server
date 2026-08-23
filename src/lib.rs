//! WaveFlow Server v2 library surface.

pub mod authentication;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod database;
pub mod http;
pub mod lyrics;
pub mod media;
pub mod oauth;
pub mod pid;
pub mod scanner;
pub mod security;
pub mod services;
pub mod stream_ticket;
pub mod subsonic;
pub mod sync;
pub mod tags;
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
    pub sync: sync::SyncService,
    pub artwork_dir: std::path::PathBuf,
    pub instance_key_path: std::path::PathBuf,
    pub public_url: Option<String>,
    pub stream_ticket_ttl: std::time::Duration,
    pub refresh_token_ttl: std::time::Duration,
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
        http::setup_status,
        http::setup,
        http::login,
        http::refresh,
        http::logout,
        http::web_login,
        http::web_refresh,
        http::web_logout,
        http::oauth_authorize,
        http::oauth_token,
        http::start_scan,
        http::list_libraries,
        http::create_library,
        http::set_library_member,
        http::remove_library_member,
        http::scan_status,
        http::scan_events,
        http::list_tracks,
        http::get_track,
        http::get_track_lyrics,
        http::list_albums,
        http::list_genres,
        http::get_album,
        http::list_artists,
        http::get_artist,
        http::search_catalog,
        http::list_random_songs,
        http::list_songs_by_genre,
        http::list_playlists,
        http::create_playlist,
        http::get_playlist,
        http::update_playlist,
        http::delete_playlist,
        http::list_favorites,
        http::add_favorite,
        http::remove_favorite,
        http::set_rating,
        http::list_ratings,
        http::create_scrobble,
        http::list_history,
        http::list_now_playing,
        http::get_queue,
        http::save_queue,
        http::list_shares,
        http::create_share,
        http::update_share,
        http::delete_share,
        http::sync_changes,
        http::sync_snapshot,
        http::sync_ack,
        http::sync_socket,
        http::transcode_status,
        http::list_users,
        http::create_user,
        http::update_user,
        http::delete_user,
        http::set_subsonic_credential,
        http::revoke_subsonic_credential,
        http::list_bookmarks,
        http::set_bookmark,
        http::delete_bookmark,
        http::list_api_tokens,
        http::create_api_token,
        http::revoke_api_token,
        media::stream_track,
        media::create_stream_ticket,
        media::stream_with_ticket,
        media::artwork
    ),
    components(schemas(
        http::ProbeResponse,
        http::ReadyResponse,
        http::SetupStatusResponse,
        http::SetupRequest,
        http::SetupResponse,
        http::LoginRequest,
        http::RefreshRequest,
        http::WebAuthResponse,
        http::ErrorResponse,
        authentication::AuthTokens,
        authentication::AuthUser,
        database::AccountRole,
        http::ScanQueuedResponse,
        catalog::ScanJobRecord,
        catalog::TrackRecord,
        catalog::LibraryAccess,
        services::AlbumItem,
        services::ArtistItem,
        services::GenreItem,
        services::SongItem,
        lyrics::LyricsList,
        lyrics::StructuredLyrics,
        lyrics::LyricsLine,
        services::ArtistSummary,
        services::AlbumDetail,
        services::ArtistDetail,
        services::SearchResult,
        services::PlaylistItem,
        services::QueueItem,
        services::RatingItem,
        services::HistoryItem,
        services::BookmarkItem,
        services::UserItem,
        http::CreatePlaylistRequest,
        http::UpdatePlaylistRequest,
        http::RatingRequest,
        http::ScrobbleRequest,
        http::SaveQueueRequest,
        http::CreateShareRequest,
        http::UpdateShareRequest,
        http::ShareResponse,
        http::AuthorizeRequest,
        http::AuthorizeResponse,
        http::TokenRequest,
        http::StarredEntry,
        http::NowPlayingEntry,
        http::SyncAckRequest,
        http::SyncSnapshot,
        http::TranscodeStatusResponse,
        http::CreateUserRequest,
        http::UpdateUserRequest,
        http::SetSubsonicCredentialRequest,
        http::SubsonicCredentialResponse,
        http::BookmarkRequest,
        http::CreateApiTokenRequest,
        http::CreateApiTokenResponse,
        database::ApiTokenRecord,
        http::CreateLibraryRequest,
        http::CreateLibraryResponse,
        http::SetLibraryMemberRequest,
        sync::SyncChange,
        sync::SyncPage,
        media::StreamTicketResponse,
        scanner::ScanProgress
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "probes", description = "Process and SQLite health"),
        (name = "authentication", description = "Local WaveFlow sessions")
        ,(name = "catalog", description = "Authoritative library scans and catalogue reads")
        ,(name = "user-data", description = "Cross-protocol playlists and playback state")
        ,(name = "sync", description = "Durable WaveFlow Desktop user-data synchronization")
        ,(name = "administration", description = "Administrative user and credential management")
    )
)]
pub struct ApiDoc;

/// Declares the bearer scheme and the two mutation headers.
///
/// Without this, a client generated from the document alone authenticates
/// nowhere and never sends `X-WaveFlow-Operation-Id` — losing replay safety,
/// which is the one thing the native API offers over the Subsonic facade.
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};

        let components = openapi.components.get_or_insert_with(Default::default);
        components.add_security_scheme(
            BEARER_SECURITY_SCHEME,
            SecurityScheme::Http(
                Http::builder()
                    .scheme(HttpAuthScheme::Bearer)
                    .description(Some(
                        "Access token from /api/v2/auth/login, /api/v2/auth/refresh or the \
                         PKCE exchange, or a long-lived `wfapi_` token minted by the CLI.",
                    ))
                    .build(),
            ),
        );
        openapi.security = Some(vec![utoipa::openapi::security::SecurityRequirement::new(
            BEARER_SECURITY_SCHEME,
            Vec::<String>::new(),
        )]);

        clear_security_on_public_operations(openapi);
        annotate_mutation_headers(openapi);
    }
}

const BEARER_SECURITY_SCHEME: &str = "bearer";

/// Operations that carry their own credential, or none at all. The global
/// requirement above would otherwise claim a token is needed to log in.
///
/// `/api/v2/stream/{ticket}` belongs here on purpose: the sealed ticket in the
/// path *is* the credential, because `<audio src>` cannot send a header.
const PUBLIC_OPERATIONS: &[(&str, &str)] = &[
    ("/health", "get"),
    ("/ready", "get"),
    ("/api/v2/setup", "get"),
    ("/api/v2/setup", "post"),
    ("/api/v2/auth/login", "post"),
    ("/api/v2/auth/refresh", "post"),
    ("/api/v2/auth/logout", "post"),
    ("/api/v2/web/auth/login", "post"),
    ("/api/v2/web/auth/refresh", "post"),
    ("/api/v2/web/auth/logout", "post"),
    ("/api/v2/oauth/token", "post"),
    ("/api/v2/stream/{ticket}", "get"),
];

fn clear_security_on_public_operations(openapi: &mut utoipa::openapi::OpenApi) {
    for (path, method) in PUBLIC_OPERATIONS {
        let Some(item) = openapi.paths.paths.get_mut(*path) else {
            continue;
        };
        let operation = match *method {
            "get" => item.get.as_mut(),
            "post" => item.post.as_mut(),
            _ => None,
        };
        if let Some(operation) = operation {
            operation.security = Some(Vec::new());
        }
    }
}

/// Attaches the two optional mutation headers, and the 409 they can produce, to
/// every user-data *write*. `mutation_context` accepts them on any such route,
/// so they are declared per operation rather than described in prose nobody
/// generates a client from.
///
/// Reads are skipped: a GET carries no operation id and cannot conflict.
fn annotate_mutation_headers(openapi: &mut utoipa::openapi::OpenApi) {
    use utoipa::openapi::path::{ParameterBuilder, ParameterIn};
    use utoipa::openapi::{Required, ResponseBuilder, Schema, Type};

    let header = |name: &str, description: &str| {
        ParameterBuilder::new()
            .name(name)
            .parameter_in(ParameterIn::Header)
            .required(Required::False)
            .description(Some(description.to_owned()))
            .schema(Some(Schema::Object({
                let mut object = utoipa::openapi::Object::new();
                object.schema_type = utoipa::openapi::schema::SchemaType::new(Type::String);
                object.format = Some(utoipa::openapi::SchemaFormat::Custom("uuid".to_owned()));
                object
            })))
            .build()
    };

    for item in openapi.paths.paths.values_mut() {
        // GET excluded on purpose: reads carry no operation id.
        let operations = [
            item.put.as_mut(),
            item.post.as_mut(),
            item.delete.as_mut(),
            item.patch.as_mut(),
        ];
        for operation in operations.into_iter().flatten() {
            if !operation
                .tags
                .as_ref()
                .is_some_and(|tags| tags.iter().any(|tag| tag == "user-data"))
            {
                continue;
            }
            operation
                .responses
                .responses
                .entry("409".to_owned())
                .or_insert_with(|| {
                    ResponseBuilder::new()
                        .description(
                            "Conflict. `code` is `conflict`: the operation id was already used \
                         for a different payload. Retrying verbatim will fail again — mint \
                         a new operation id.",
                        )
                        .build()
                        .into()
                });
            let parameters = operation.parameters.get_or_insert_with(Vec::new);
            parameters.push(header(
                http::OPERATION_ID_HEADER,
                "Stable id for this logical mutation. Repeating it replays the original \
                 outcome instead of applying twice; reusing it for a different payload is \
                 rejected as a conflict. Generated server-side when absent.",
            ));
            parameters.push(header(
                http::DEVICE_ID_HEADER,
                "Device originating the mutation. Rejected when it belongs to another \
                 account. Lets other devices skip their own echo in the sync journal.",
            ));
        }
    }
}

pub async fn initialize(config: &Config) -> anyhow::Result<AppState> {
    let db = database::Database::open(config).await?;
    db.migrate().await?;
    db.reconcile_catalog_identity(&config.pid).await?;
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
    let sync = sync::SyncService::new(db.clone());
    let services = services::DomainServices::new(
        db.clone(),
        Arc::clone(&secret_box),
        sync.clone(),
        scanner.clone(),
    );
    Ok(AppState {
        db,
        auth,
        secret_box,
        scanner,
        media,
        services,
        sync,
        artwork_dir: config.artwork_dir.clone(),
        instance_key_path: config.instance_key_path.clone(),
        public_url: config.public_url.clone(),
        stream_ticket_ttl: config.stream_ticket_ttl,
        refresh_token_ttl: config.refresh_token_ttl,
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
                    axum::http::Method::PUT,
                    axum::http::Method::PATCH,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::RANGE,
                    axum::http::HeaderName::from_static(http::WEB_CSRF_HEADER),
                    axum::http::HeaderName::from_static(http::OPERATION_ID_HEADER),
                    axum::http::HeaderName::from_static(http::DEVICE_ID_HEADER),
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
