//! The router `main` serves and the tests drive: every surface merged under
//! the layers they share, and the redaction applied to the paths it traces.

use axum::{extract::DefaultBodyLimit, http::Request, Router};
use tower::ServiceBuilder;
use tower_http::{
    cors::{AllowOrigin, CorsLayer},
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use utoipa::OpenApi;

use super::openapi::{openapi_response, ApiDoc, OPENAPI_JSON_PATH};
use super::state::AppState;
use crate::{api, media, public_share, reference, subsonic, webui, Config};

const REQUEST_ID_HEADER: &str = "x-request-id";

pub fn app(config: &Config, state: AppState) -> Router {
    let openapi = ApiDoc::openapi();
    let openapi_for_route = openapi.clone();
    let request_id_header = axum::http::HeaderName::from_static(REQUEST_ID_HEADER);
    let middleware = ServiceBuilder::new()
        // Before `SetRequestIdLayer`, which keeps whatever the caller sent and
        // only forges an id when the header is absent. Dropping the inbound one
        // unconditionally is what makes the layer below always mint, so the
        // value in every `http_request` span is one this server chose.
        //
        // `CLAUDE.md` says no header reaches a trace sink, without exception,
        // and an id narrow enough to look like an identifier is still a string
        // a stranger picked: send a well-formed UUID that somebody else's
        // requests are using and the trace record stops telling two callers
        // apart. Validating the shape would have addressed the noise and left
        // that.
        //
        // The correlation the header exists for is not lost, only reversed:
        // `PropagateRequestIdLayer` returns the minted id on the response, so
        // an upstream proxy records the name this server gave rather than
        // imposing one. That is the ordinary arrangement wherever a server does
        // not trust what is in front of it, and this one is routinely in front
        // of nothing at all.
        .map_request(|mut request: Request<_>| {
            request.headers_mut().remove(REQUEST_ID_HEADER);
            request
        })
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
        .route(
            OPENAPI_JSON_PATH,
            axum::routing::get(move || openapi_response(openapi_for_route.clone())),
        )
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            config.request_timeout,
        ));

    // What may be compressed, kept apart from what may not.
    //
    // The exclusion is **structural rather than conditional**: the media
    // routes and the Subsonic facade are merged outside this group, so no
    // predicate decides whether a stream is compressed — the layer never sees
    // one. Compressing an already-compressed body spends CPU for nothing, and
    // a body that arrives re-encoded is a body whose length and ranges no
    // longer describe the file a player is seeking through.
    //
    // `CompressionLayer::new()` carries tower-http's `DefaultPredicate`, which
    // leaves alone anything under 32 bytes, images, gRPC and — load-bearing
    // here — `text/event-stream`: the scan event feed lives on `/api/v2` and
    // would otherwise be buffered into silence by its own encoder.
    //
    // The Subsonic facade is out for a reason worth writing down, because it
    // costs a real gain: it serves every method through one route,
    // `/rest/{method}`, so a catalogue listing and an audio stream arrive by
    // the same path. Separating them there would mean trusting a content-type
    // predicate with the correctness of byte-range playback, which is the one
    // thing in this server that four clients were replayed to establish.
    let compressible = Router::new()
        .merge(ordinary)
        // A portrait search may take minutes; it keeps a timeout of its own.
        .merge(api::search_router(state.clone()))
        // Anything no API route claimed is served by the embedded web client:
        // a built asset, or the shell for a client-side route. This is what
        // the layer is mostly for — the built client leaves raw at roughly
        // 497 kB and about 152 kB gzipped.
        .fallback(webui::handler)
        .layer(tower_http::compression::CompressionLayer::new());

    let router = Router::new()
        .merge(compressible)
        // Media bodies can outlive the ordinary API timeout. FFmpeg and
        // disconnected consumers are bounded by MediaService itself.
        .merge(media::router(state.clone()))
        .merge(public_share::router(state.clone(), config.request_timeout))
        // The API reference and the script it loads, from the binary (RFC-023
        // decision 3). Outside the compression layer: the script is stored
        // compressed already.
        .merge(reference::router(&openapi))
        .merge(subsonic::router(state))
        .layer(DefaultBodyLimit::max(16 * 1024));

    // CORS goes on first so that the request-id and trace layers end up
    // outside it. `CorsLayer` answers a preflight itself without calling the
    // service under it, so anything applied inside it never runs for an
    // `OPTIONS` — which would leave preflights unnamed, against what the API
    // guide promises, and invisible in the traces. An origin rejected by
    // `WAVEFLOW_ALLOWED_ORIGINS` is exactly the thing an operator has to
    // diagnose, and it was the one request that left no record.
    let cors = if config.allowed_origins.is_empty() {
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
                // `x-request-id` is exposed and deliberately not allowed: a
                // browser client should be able to read the name this server
                // gave its request, which is what makes it quotable in a bug
                // report, but it has no business sending one — an inbound value
                // is dropped before anything reads it.
                .expose_headers([
                    axum::http::header::ACCEPT_RANGES,
                    axum::http::header::CONTENT_LENGTH,
                    axum::http::header::CONTENT_RANGE,
                    axum::http::HeaderName::from_static(REQUEST_ID_HEADER),
                ]),
        )
    };

    cors.layer(middleware)
}

/// Every prefix whose next path segment is a credential, and what a trace sink
/// sees instead.
///
/// **A table rather than a chain of `if`s, because the chain lost one.** This
/// function knew the audio ticket and the share token; the server has a third
/// such route, `/api/v2/canvas-stream/{ticket}`, minted beside the audio one in
/// `media.rs`, played by a `<video src>` that cannot send an `Authorization`
/// header, and listed beside its twin in `PUBLIC_OPERATIONS` two hundred lines
/// above. It was never added here, so the sealed ticket went into the
/// `http_request` span verbatim.
///
/// **What that actually cost, stated exactly.** The span is opened at INFO, but
/// nothing here asks for span lifecycle events, so opening one writes nothing on
/// its own: a canvas playback that succeeds quietly emitted no line. The ticket
/// reached a sink on every canvas request that logged anything *inside* that
/// span — a failed ticket lookup, a 5xx — and on every request at all under
/// `tower_http=debug`, where the tower layer's own request and response events
/// fire in it. An earlier version of this paragraph said "every canvas
/// playback", which is one notch stronger than the mechanism.
///
/// The shape is the fix. A new ticket route now has one obvious place to be
/// declared, next to the ones that look exactly like it, rather than a branch
/// somebody has to remember to add.
///
/// Matched on the raw path, so `/API/v2/stream/…` and a percent-encoded
/// separator miss every entry. Left that way deliberately: reaching those
/// spellings means already holding the ticket, so it is hardening rather than a
/// disclosure route, and normalising here would put a second opinion about what
/// a path means next to the router's.
pub(super) const CREDENTIAL_BEARING_PREFIXES: [(&str, &str); 4] = [
    ("/api/v2/stream/", "/api/v2/stream/{redacted}"),
    ("/api/v2/canvas-stream/", "/api/v2/canvas-stream/{redacted}"),
    ("/share/", "/share/{redacted}"),
    // The Last.fm return. Its `{state}` is a single-use random worth a
    // profile for a quarter of an hour, and the `token` Last.fm appends is
    // worth one for an hour — that one sits in the query string, which no
    // trace of this server keeps. The rule in `CLAUDE.md` has no exception for
    // a secret that only lasts an hour.
    (
        crate::services::LASTFM_CALLBACK_PREFIX,
        "/api/v2/scrobble-links/lastfm/callback/{redacted}",
    ),
];

pub(super) fn trace_path(path: &str) -> &str {
    for (prefix, redacted) in CREDENTIAL_BEARING_PREFIXES {
        if path.starts_with(prefix) {
            return redacted;
        }
    }
    path
}
