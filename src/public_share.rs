//! The public share: the page a recipient opens, and the streams it plays.
//!
//! Not a Subsonic route, although `createShare` is what hands its address out:
//! a share is opened by someone with no account and no client, in a browser
//! (RFC-020). It lived in the façade's router until that RFC, which is why the
//! paths are the ones Subsonic clients were already given.
//!
//! **One address, two answers** (decision 3). `/share/{token}` answers the web
//! client to a browser that asks for HTML, and the share's JSON to everyone
//! else — as it always has. The client then asks the same address for the
//! JSON, and that request is the visit.
//!
//! **A visit is counted once, when the share is opened** (decision 4). The
//! opening hands out one sealed pass per track, and the stream route redeems
//! the pass without writing anything: a seek is a range request, and it must
//! neither count a visit nor queue behind a scan for the writer gate.

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::{
    authentication::now_ms,
    media::{OutputFormat, StreamQuery},
    services::ServiceError,
    stream_ticket::{self, TicketKind},
    AppState,
};

/// How long the passes of one visit keep playing.
///
/// Longer than a stream ticket's hour, because what it covers is a sitting
/// rather than a track: a share holds up to four hundred of them, and a
/// visitor who opened it in the evening should not find the player dead after
/// the first album. It is not what keeps a share closed — deleting it or
/// letting it expire stops every pass at once, since each stream request reads
/// the share again — so all it bounds is how long one counted visit lasts.
const VISIT_TTL_MS: i64 = 12 * 60 * 60 * 1000;

/// The one transcode an anonymous visitor may ask for (decision 5).
///
/// The server's default MP3 profile, so the cache entry it fills is the one
/// every other default MP3 request of the same file already uses, and a
/// second visitor costs no encode.
const VISIT_MP3_BITRATE: u32 = 192;

/// The two routes, under two different time limits.
///
/// **The opening is bounded by the ordinary request timeout.** It counts a
/// visit, so it waits for the writer gate, and a scan can hold that gate for
/// minutes: without a bound, every anonymous opening during a long scan kept
/// its connection open for as long as the scan ran. **The stream is not**, for
/// the reason every media route is not: a body may rightly outlive any fixed
/// request timeout, and `MediaService` bounds what it spawns itself.
///
/// **Every answer of both carries the share headers** (RFC-019 decision 2),
/// applied outermost so that a timeout, a refused query and a 404 carry them
/// too: a share's token is its path, and no response under it may hand that
/// path away as a referrer or be framed.
pub fn router(state: AppState, request_timeout: std::time::Duration) -> Router {
    let opening = Router::new().route("/share/{token}", get(open)).layer(
        tower_http::timeout::TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            request_timeout,
        ),
    );
    Router::new()
        .route("/share/{token}/tracks/{track_id}/stream", get(stream))
        .route("/share/{token}/tracks/{track_id}/cover", get(cover))
        .merge(opening)
        .layer(axum::middleware::map_response(
            crate::security_headers::share,
        ))
        .with_state(state)
}

/// Whether the caller asked for a page rather than a document.
///
/// Only an explicit `text/html` counts. A browser following a link says so
/// before anything else; a script says `*/*`, `application/json` or nothing,
/// and every one of those keeps the JSON it was getting. Matching the wildcard
/// too would have handed HTML to every consumer that never asked.
fn wants_page(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|range| {
            let mut parts = range.split(';').map(str::trim);
            let media = parts.next().unwrap_or_default();
            let refused = parts.any(|parameter| {
                parameter
                    .strip_prefix("q=")
                    .and_then(|q| q.parse::<f32>().ok())
                    .is_some_and(|q| q <= 0.0)
            });
            media.eq_ignore_ascii_case("text/html") && !refused
        })
}

/// What every answer of the opening carries, beside the share headers the
/// router adds to everything under `/share/`.
///
/// `Vary`, because one address answers two representations and a cache that
/// ignored the header would hand a browser the JSON. `no-store`, because both
/// are a bearer's view.
fn finish(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(header::VARY, HeaderValue::from_static("accept"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn open(
    State(state): State<AppState>,
    Path(token): Path<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    // A `HEAD` is answered like the page, which counts nothing. Axum hands it
    // to this `get` route with the body dropped, and it rarely names
    // `text/html`, so it used to fall through to the document — the visit —
    // and an uptime monitor or a link previewer probing the address spent a
    // capped share's visits on nothing anyone heard. Its status is the
    // page's: 200 for a share that can be opened, 404 for one that cannot.
    if method == Method::HEAD || wants_page(&headers) {
        return finish(page(&state, &token).await);
    }
    finish(document(&state, &token).await)
}

/// The web client, which renders the share at this address.
///
/// Counts nothing: a link preview fetching the page to draw a card has not
/// listened to anything, and a cap must not be spent on one. A share that
/// cannot be opened answers the same page with a 404, which the client reads
/// as "this link opens nothing" — the status says so to anything else.
async fn page(state: &AppState, token: &str) -> Response {
    let open = match state.services.share_is_open(token).await {
        Ok(open) => open,
        Err(error) => {
            tracing::error!(error = %error, "public share page lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let mut response = crate::webui::share_page();
    if !open && response.status() == StatusCode::OK {
        *response.status_mut() = StatusCode::NOT_FOUND;
    }
    response
}

/// The share as JSON, and the visit.
async fn document(state: &AppState, token: &str) -> Response {
    let share = match state.services.public_share(token).await {
        Ok(share) => share,
        Err(ServiceError::NotFound) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(error = %error, "public share lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let Some(expires_at) = now_ms().checked_add(VISIT_TTL_MS) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut tracks = Vec::with_capacity(share.songs.len());
    for song in &share.songs {
        let pass = match stream_ticket::mint(
            &state.secret_box,
            TicketKind::ShareVisit,
            share.id,
            song.id,
            expires_at,
        ) {
            Ok(pass) => pass,
            Err(error) => {
                tracing::error!(error = %error, "share visit pass minting failed");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
        let path = format!("/share/{token}/tracks/{}/stream?visit={pass}", song.id);
        let absolute = |path: &str| {
            state
                .public_url
                .as_deref()
                .map_or_else(|| path.to_owned(), |base| format!("{base}{path}"))
        };
        let mut value = serde_json::to_value(song).expect("song serialization");
        if let Value::Object(object) = &mut value {
            object.insert("streamUrl".into(), Value::String(absolute(&path)));
            // The cover, under the same pass (see `cover`). Only for a track
            // that has one, so the page asks for nothing it would be refused.
            if song.artwork_hash.is_some() {
                let cover = format!("/share/{token}/tracks/{}/cover?visit={pass}", song.id);
                object.insert("coverUrl".into(), Value::String(absolute(&cover)));
            }
        }
        tracks.push(value);
    }
    axum::Json(serde_json::json!({
        "id": share.id,
        "description": share.description,
        "expiresAt": share.expires_at,
        "visitCount": share.visit_count,
        "tracks": tracks,
    }))
    .into_response()
}

/// What a share stream may be asked for, and nothing more (decision 5).
///
/// Unknown parameters are refused rather than ignored: a visitor sending
/// `bitrate=96` must learn it got nothing of the kind, not receive the
/// original and believe otherwise.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareStreamQuery {
    /// The pass the opening handed out for this track.
    visit: Option<String>,
    /// Absent or `raw` for the original, `mp3` for the fixed MP3.
    format: Option<String>,
}

async fn stream(
    State(state): State<AppState>,
    Path((token, track_id)): Path<(String, Uuid)>,
    Query(query): Query<ShareStreamQuery>,
    headers: HeaderMap,
) -> Response {
    let (format, bitrate) = match query.format.as_deref() {
        None | Some("raw") => (OutputFormat::Raw, None),
        Some("mp3") => (OutputFormat::Mp3, Some(VISIT_MP3_BITRATE)),
        Some(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    // A missing, forged, expired or foreign pass answers like a track that is
    // not in the share, so probing learns nothing about which it was.
    let Some((share_id, pass_track)) = query
        .visit
        .as_deref()
        .and_then(|pass| stream_ticket::verify(&state.secret_box, pass, now_ms()))
        .filter(|(kind, ..)| *kind == TicketKind::ShareVisit)
        .map(|(_, share_id, track)| (share_id, track))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if pass_track != track_id {
        return StatusCode::NOT_FOUND.into_response();
    }
    let owner = match state
        .services
        .shared_track_owner(&token, share_id, track_id)
        .await
    {
        Ok(owner) => owner,
        Err(ServiceError::NotFound) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(error = %error, "public share stream lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let track = match state.db.stream_track_for_user(owner, track_id).await {
        Ok(Some(track)) => track,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(error = %error, "public share media lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let query = StreamQuery {
        format,
        bitrate,
        offset_ms: 0,
    };
    match state.media.serve(owner, track, query, range).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

/// What a share cover may be asked for: the pass, and nothing else.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareCoverQuery {
    visit: Option<String>,
}

/// A shared track's cover, for the share page.
///
/// **A route of the share's own** rather than the artwork route: that one
/// needs a session and answers for any cover the account may read, and a
/// visitor has no session. This one answers exactly one cover — that of a
/// track the share holds — and is authorised the way the stream is, by the
/// pass the counted opening handed out for that track, with the share read
/// again on every request. A deleted or expired share stops answering at
/// once; a capped one keeps the covers of the visit that was counted, as it
/// keeps its music. Nothing is written, nothing is counted.
///
/// The track's own cover, or its album's when it has none, read through the
/// owner's libraries: a track the owner can no longer read answers 404.
async fn cover(
    State(state): State<AppState>,
    Path((token, track_id)): Path<(String, Uuid)>,
    Query(query): Query<ShareCoverQuery>,
    headers: HeaderMap,
) -> Response {
    let Some((share_id, pass_track)) = query
        .visit
        .as_deref()
        .and_then(|pass| stream_ticket::verify(&state.secret_box, pass, now_ms()))
        .filter(|(kind, ..)| *kind == TicketKind::ShareVisit)
        .map(|(_, share_id, track)| (share_id, track))
    else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if pass_track != track_id {
        return StatusCode::NOT_FOUND.into_response();
    }
    let owner = match state
        .services
        .shared_track_owner(&token, share_id, track_id)
        .await
    {
        Ok(owner) => owner,
        Err(ServiceError::NotFound) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(error = %error, "public share cover lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let (hash, format) = match state.services.track_cover_for_user(owner, track_id).await {
        Ok(Some(found)) => found,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(error) => {
            tracing::error!(error = %error, "public share cover lookup failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // `private`: two owners whose libraries hold the same cover share its
    // hash, and the address carries a bearer pass. `no-cache` with the hash
    // as validator, so a second look within the visit is a 304.
    let etag = format!("\"{hash}\"");
    let cached = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').map(str::trim).any(|tag| tag == etag));
    let validators = [
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static("private, no-cache"),
        ),
        (
            header::ETAG,
            HeaderValue::from_str(&etag).expect("a hex hash is a header value"),
        ),
    ];
    if cached {
        return (StatusCode::NOT_MODIFIED, validators).into_response();
    }
    match crate::media::read_artwork(&state.artwork_dir, &hash, &format).await {
        Some((mime, bytes)) => (
            StatusCode::OK,
            validators,
            [(header::CONTENT_TYPE, HeaderValue::from_static(mime))],
            bytes,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accept(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn only_an_explicit_html_request_is_handed_the_page() {
        for page in [
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            "TEXT/HTML",
            "application/json;q=0.5, text/html",
        ] {
            assert!(wants_page(&accept(page)), "{page}");
        }
        for document in [
            "*/*",
            "application/json",
            "text/*",
            "text/html;q=0",
            "text/htmlx",
        ] {
            assert!(!wants_page(&accept(document)), "{document}");
        }
        assert!(!wants_page(&HeaderMap::new()));
    }
}
