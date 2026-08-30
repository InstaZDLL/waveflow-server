//! WaveFlow Server v2 library surface.

pub mod api;
pub mod authentication;
pub mod catalog;
pub mod cli;
pub mod config;
pub mod database;
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
    /// The body ceiling the upload negotiation route carries, derived from how
    /// many offers a batch may hold.
    ///
    /// The router's global limit is sixteen kilobytes and stays there: an API
    /// whose every route accepts that much cannot be drowned in a request body,
    /// and that property is worth more than the convenience of raising it once.
    /// A batch of two hundred offers does not fit in it, so the route that
    /// needs the room asks for it alone.
    pub upload_batch_body_limit: usize,
    /// The body ceiling one fragment may occupy.
    ///
    /// A little above the advertised fragment size, for the framing rather than
    /// for slack: a client that sends more than it was told to is refused by
    /// the route before the service has to read it.
    pub upload_chunk_body_limit: usize,
    /// The body ceiling the canvas route carries, derived from
    /// `WAVEFLOW_CANVAS_MAX_BYTES` so the two cannot disagree.
    pub canvas_body_limit: usize,
}

/// Roughly one offer's worth of JSON — a sixty-four character hash, a size, a
/// short extension and their punctuation — times the batch, with a fixed
/// allowance for the envelope. Derived rather than configured, so a raised
/// batch limit cannot quietly start rejecting the batches it just permitted.
fn negotiation_body_limit(limits: &config::UploadLimits) -> usize {
    limits.batch_limit.saturating_mul(160).saturating_add(1024)
}

/// One fragment, plus room for its framing.
///
/// `Config` refuses a fragment size this platform cannot represent, so the
/// fallback below is unreachable. It is a small finite number rather than
/// `usize::MAX` anyway: a ceiling that falls back to "no ceiling" is the one
/// mistake this value must never make, and an unreachable branch is exactly
/// where that would go unnoticed.
fn chunk_body_limit(limits: &config::UploadLimits) -> usize {
    const FALLBACK: usize = 16 * 1024 * 1024;
    usize::try_from(limits.chunk_bytes)
        .unwrap_or(FALLBACK)
        .saturating_add(4096)
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
        api::health,
        api::ready,
        api::setup_status,
        api::setup,
        api::login,
        api::refresh,
        api::logout,
        api::web_login,
        api::web_refresh,
        api::web_logout,
        api::oauth_authorize,
        api::oauth_token,
        api::start_scan,
        api::list_libraries,
        api::create_library,
        api::set_library_member,
        api::remove_library_member,
        api::scan_status,
        api::scan_events,
        api::library_events,
        api::negotiate_uploads,
        api::upload_session,
        api::upload_chunk,
        api::commit_upload,
        api::list_tracks,
        api::get_track,
        api::update_track,
        api::get_track_lyrics,
        api::list_albums,
        api::list_genres,
        api::get_album,
        api::list_artists,
        api::get_artist,
        api::search_catalog,
        api::list_random_songs,
        api::list_songs_by_genre,
        api::list_playlists,
        api::create_playlist,
        api::get_playlist,
        api::update_playlist,
        api::delete_playlist,
        api::list_favorites,
        api::add_favorite,
        api::remove_favorite,
        api::set_rating,
        api::list_ratings,
        api::create_scrobble,
        api::list_history,
        api::list_now_playing,
        api::get_queue,
        api::save_queue,
        api::list_shares,
        api::create_share,
        api::update_share,
        api::delete_share,
        api::sync_changes,
        api::sync_snapshot,
        api::sync_ack,
        api::sync_socket,
        api::transcode_status,
        api::list_users,
        api::create_user,
        api::update_user,
        api::delete_user,
        api::set_subsonic_credential,
        api::revoke_subsonic_credential,
        api::list_bookmarks,
        api::set_bookmark,
        api::delete_bookmark,
        api::list_api_tokens,
        api::create_api_token,
        api::revoke_api_token,
        media::stream_track,
        media::create_stream_ticket,
        media::stream_with_ticket,
        media::artwork,
        media::canvas_by_hash,
        media::canvas_for_track,
        media::put_canvas,
        media::delete_canvas,
        media::create_canvas_ticket,
        media::canvas_with_ticket
    ),
    components(schemas(
        api::ProbeResponse,
        api::ReadyResponse,
        api::SetupStatusResponse,
        api::SetupRequest,
        api::SetupResponse,
        api::LoginRequest,
        api::RefreshRequest,
        api::WebAuthResponse,
        api::ErrorResponse,
        authentication::AuthTokens,
        authentication::AuthUser,
        database::AccountRole,
        api::ScanQueuedResponse,
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
        api::CreatePlaylistRequest,
        api::UpdatePlaylistRequest,
        api::RatingRequest,
        api::ScrobbleRequest,
        api::SaveQueueRequest,
        api::CreateShareRequest,
        api::UpdateShareRequest,
        api::ShareResponse,
        api::AuthorizeRequest,
        api::AuthorizeResponse,
        api::TokenRequest,
        api::StarredEntry,
        api::NowPlayingEntry,
        api::SyncAckRequest,
        api::SyncSnapshot,
        services::LibraryEventPage,
        services::TrackMetadataPatch,
        services::UploadOffer,
        services::UploadVerdict,
        services::UploadDecision,
        services::UploadSessionState,
        services::CommittedUpload,
        api::NegotiateUploadsRequest,
        api::NegotiateUploadsResponse,
        services::LibraryEvent,
        api::TranscodeStatusResponse,
        api::CreateUserRequest,
        api::UpdateUserRequest,
        api::SetSubsonicCredentialRequest,
        api::SubsonicCredentialResponse,
        api::BookmarkRequest,
        api::CreateApiTokenRequest,
        api::CreateApiTokenResponse,
        database::ApiTokenRecord,
        api::CreateLibraryRequest,
        api::CreateLibraryResponse,
        api::SetLibraryMemberRequest,
        sync::SyncChange,
        sync::SyncPage,
        media::StreamTicketResponse,
        media::CanvasResponse,
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
                api::OPERATION_ID_HEADER,
                "Stable id for this logical mutation. Repeating it replays the original \
                 outcome instead of applying twice; reusing it for a different payload is \
                 rejected as a conflict. Generated server-side when absent.",
            ));
            parameters.push(header(
                api::DEVICE_ID_HEADER,
                "Device originating the mutation. Rejected when it belongs to another \
                 account. Lets other devices skip their own echo in the sync journal.",
            ));
        }
    }
}

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
    // After the key check, not before: a mismatched pair aborts the boot, and
    // scheduling a rescan on a database this instance is about to refuse would
    // be writing to a catalogue that is not ours.
    db.reconcile_catalog_identity(&config.pid).await?;
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
        config,
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
        upload_batch_body_limit: negotiation_body_limit(&config.uploads),
        upload_chunk_body_limit: chunk_body_limit(&config.uploads),
        // Refused at startup if it does not fit, so this conversion cannot be
        // the place that decides what a ceiling falls back to.
        canvas_body_limit: usize::try_from(config.canvas.max_bytes).unwrap_or(usize::MAX),
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
        .merge(api::router(state.clone()))
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
                    axum::http::HeaderName::from_static(api::WEB_CSRF_HEADER),
                    axum::http::HeaderName::from_static(api::OPERATION_ID_HEADER),
                    axum::http::HeaderName::from_static(api::DEVICE_ID_HEADER),
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
