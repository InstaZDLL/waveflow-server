//! waveflow-server library entrypoint.
//!
//! The public API is intentionally tight: callers (the binary in
//! `main.rs` + integration tests) construct a [`Config`] and obtain a
//! ready-to-serve axum router via [`app`]. Internal modules stay
//! private until something outside this crate needs them.
//!
//! The full architectural intent lives in [RFC-001][rfc] — read that
//! before opening a PR that adds a new module.
//!
//! [rfc]: https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md

use axum::{extract::Request, response::IntoResponse, Router};
use sqlx::PgPool;
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::field::Empty;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

pub mod api;
pub mod apply;
pub mod auth;
pub mod config;
pub mod db;
pub mod middleware;
pub mod storage;
pub mod stream_token;
pub mod sync;

pub use config::Config;

/// OpenAPI document shell. Tagged endpoints come from each module via
/// `OpenApiRouter::routes(routes!(handler))`, so this struct only
/// declares the shared metadata (title, version, description, tags).
/// The actual `paths(...)` list is filled by [`utoipa_axum`] at router-
/// build time — no parallel list to keep in sync when adding handlers.
#[derive(OpenApi)]
#[openapi(
    info(
        title = "waveflow-server",
        description = "Self-hosted backend for WaveFlow. \
            See https://github.com/InstaZDLL/WaveFlow/blob/main/docs/rfcs/RFC-001-waveflow-server.md \
            for the architectural intent.",
        license(
            name = "AGPL-3.0-only",
            url = "https://www.gnu.org/licenses/agpl-3.0.html",
        ),
    ),
    tags(
        (name = "probes", description = "Liveness / readiness endpoints for orchestrators."),
    ),
)]
pub struct ApiDoc;

/// State threaded through the axum router. Holds the singletons
/// every handler / middleware needs. Cheap to clone — the pool is
/// `Arc`-backed and the verifier sits behind an `Arc`.
///
/// `Debug` is deliberately NOT derived: `JwtVerifier` opts out of
/// `Debug` to keep raw key bytes off log sinks (see the rationale in
/// `auth.rs`), so the parent state inherits the same restriction.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Verifier built at boot from the required `WAVEFLOW_JWT_*`
    /// triple. Phase 1.d.2 made this non-optional — the dev
    /// `X-User-Id` shim retired, so the JWT path is the only auth
    /// channel and a missing verifier means the server shouldn't
    /// have booted in the first place.
    pub jwt_verifier: std::sync::Arc<auth::JwtVerifier>,
    /// Streaming configuration. `Some` when both
    /// `WAVEFLOW_MUSIC_ROOT` and `WAVEFLOW_STREAM_SECRET` are set at
    /// boot; `None` disables both the mint and the stream endpoints
    /// (they answer 503).
    pub stream_ctx: Option<std::sync::Arc<StreamCtx>>,
    /// Multi-device sync hub (Phase 1.f). Owns the broadcast channel
    /// every WebSocket subscribes to, the in-memory ACK buffer the
    /// 5 s flusher drains, and the daily compaction task. Cheap to
    /// clone — every inner field is `Arc`-backed.
    pub sync: sync::SyncHub,
    /// Artwork storage handle (Phase 1.h.1). `Some` when
    /// `WAVEFLOW_ARTWORK_LOCAL_DIR` is set at boot; `None` disables
    /// both the upload and the public-read endpoint (they answer
    /// 503). Cheap to clone — the inner `ObjectStore` is `Arc`-backed.
    pub artwork: Option<storage::ArtworkStorage>,
}

/// Per-process streaming context built once at boot. Wraps the HMAC
/// key and canonicalised music root behind an `Arc` so the
/// per-request hot path stays a pointer copy.
///
/// `Debug` is deliberately NOT derived: a derived impl would dump the
/// `secret` bytes into any `tracing` field or panic message that
/// happens to print the state, and that's exactly the leak this
/// type is supposed to prevent.
pub struct StreamCtx {
    pub music_root: std::path::PathBuf,
    pub secret: Vec<u8>,
}

impl std::fmt::Debug for StreamCtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StreamCtx")
            .field("music_root", &self.music_root)
            .field("secret", &"<redacted>")
            .finish()
    }
}

/// Header used for inbound + propagated request IDs. UUIDv4 by default
/// (via `MakeRequestUuid`), but a client / upstream proxy can supply
/// its own — useful for stitching traces across multiple services.
const REQUEST_ID_HEADER: &str = "x-request-id";

/// Path the generated OpenAPI 3.1 document is served at.
pub const OPENAPI_JSON_PATH: &str = "/openapi.json";

/// Path the Scalar UI (`utoipa-scalar`) is mounted at.
pub const SCALAR_PATH: &str = "/reference";

/// Build the axum router. Wired with:
/// - per-request UUID via `x-request-id` (generated if absent, echoed back).
/// - structured access logging keyed on the request id.
/// - configurable timeout (default 30 s, set via `WAVEFLOW_REQUEST_TIMEOUT_SECS`).
/// - shared [`AppState`] (Postgres pool) attached via `with_state`.
/// - OpenAPI doc at [`OPENAPI_JSON_PATH`] and Scalar UI at [`SCALAR_PATH`].
///
/// `Config` is consumed at build time for the middleware bounds;
/// runtime singletons live in the [`AppState`] threaded through the
/// router.
pub fn app(config: Config, state: AppState) -> Router {
    let middleware = ServiceBuilder::new()
        .layer(SetRequestIdLayer::new(
            axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
            MakeRequestUuid,
        ))
        // Custom span builder so every emitted trace carries the
        // request id as a structured field. The default `MakeSpan`
        // doesn't include headers at all; `include_headers(true)` would
        // dump *every* header (incl. Authorization / Cookie) into log
        // sinks, which is the opposite of what we want. Extract just
        // the one field we care about.
        .layer(TraceLayer::new_for_http().make_span_with(|req: &Request| {
            let request_id = req
                .headers()
                .get(REQUEST_ID_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            tracing::info_span!(
                "http_request",
                method = %req.method(),
                uri = %req.uri(),
                version = ?req.version(),
                request_id = %request_id,
                status = Empty,
            )
        }))
        .layer(PropagateRequestIdLayer::new(
            axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
        ))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(config.request_timeout_secs),
        ));

    // Seed the API router with the ApiDoc shell so every module's
    // `#[utoipa::path]` declarations merge into it, then split into
    // `(Router, OpenApi)` for axum + spec consumption. The doc is
    // serialised under `/openapi.json` and rendered by Scalar at
    // `/reference`; both stay outside the `/api/v1/*` namespace so a
    // future Better-Auth middleware (1.d) gates only the data routes.
    let (api_router, openapi) = utoipa_axum::router::OpenApiRouter::with_openapi(ApiDoc::openapi())
        .merge(api::router(state))
        .split_for_parts();

    Router::new()
        .merge(api_router)
        .merge(Router::from(Scalar::with_url(SCALAR_PATH, openapi.clone())))
        .route(
            OPENAPI_JSON_PATH,
            axum::routing::get(move || {
                // `serde_json::to_string` could fail in theory; in
                // practice utoipa-built specs always serialise (every
                // type comes from a derive macro). Surface the error
                // as a 500 if it ever happens so an integration test
                // catches a regression.
                let openapi = openapi.clone();
                async move {
                    serde_json::to_string(&openapi)
                        .map(|s| {
                            ([(axum::http::header::CONTENT_TYPE, "application/json")], s)
                                .into_response()
                        })
                        .unwrap_or_else(|err| {
                            tracing::error!(error = %err, "openapi serialize failed");
                            (
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                "openapi serialize failed",
                            )
                                .into_response()
                        })
                }
            }),
        )
        .layer(middleware)
}
