//! Canvas loops: the routes that serve, place, remove and ticket them.

use axum::{
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use uuid::Uuid;

use super::artwork::if_none_match_hits;
use super::stream::{serve_file, StreamTicketResponse};
use super::{bearer_principal, MediaError};
use crate::AppState;

/// A canvas addressed by its hash. The bytes behind a hash never change, so
/// this URL can never answer differently and a client should stop asking.
const CANVAS_CANONICAL_CACHE: &str = "private, max-age=31536000, immutable";
/// A canvas addressed by a track: the link is what it resolves, and a member
/// can replace it at any moment. Stated apart from the artwork constants that
/// happen to hold the same policy — a canvas is not a cover, and one changing
/// must not drag the other.
const CANVAS_ALIAS_CACHE: &str = "private, no-cache";

fn mime_for_canvas(format: &str) -> &'static str {
    match format {
        "webm" => "video/webm",
        // The store only ever holds what the probe accepted, and that is one of
        // exactly two containers.
        _ => "video/mp4",
    }
}

/// Serves the blob behind a canvas, once something has authorised it.
///
/// Shared by the three ways in — the hash, the track alias, and a ticket — so
/// the caching rule and the 304 live in one place and cannot drift apart per
/// route. Which `cache_control` applies is the caller's decision, because only
/// the caller knows how the URL was addressed.
async fn serve_canvas(
    state: &AppState,
    blob: &crate::services::CanvasBlob,
    headers: &HeaderMap,
    cache_control: &'static str,
) -> Result<Response, MediaError> {
    // The validator is the hash however the URL was addressed, which is what
    // makes revalidating an alias a 304 rather than a second transfer of bytes
    // the client already holds.
    let etag = format!("\"{}\"", blob.hash);
    if if_none_match_hits(headers, &etag) {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [
                (header::CACHE_CONTROL, cache_control.to_owned()),
                (header::ETAG, etag),
            ],
        )
            .into_response());
    }
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    let path = state.services.canvas_file(blob);
    let mut response = serve_file(&path, range, mime_for_canvas(&blob.format)).await?;
    let served = response.headers_mut();
    served.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(cache_control).map_err(|_| MediaError::Internal)?,
    );
    served.insert(
        header::ETAG,
        HeaderValue::from_str(&etag).map_err(|_| MediaError::Internal)?,
    );
    Ok(response)
}

#[utoipa::path(
    get,
    path = "/api/v2/canvas/{canvas_hash}",
    tag = "media",
    params(("canvas_hash" = String, Path)),
    responses(
        (status = 200, description = "The loop, immutable under this URL"),
        (status = 206),
        (status = 304),
        (status = 401),
        (status = 404)
    )
)]
pub async fn canvas_by_hash(
    State(state): State<AppState>,
    AxumPath(canvas_hash): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    let user = bearer_principal(&state, &headers).await?;
    // Knowing a hash establishes nothing. Two accounts that are strangers to
    // each other can hold the same loop, so this resolves through membership
    // the way `artwork_for_user` does, and a hash nobody reachable references
    // answers exactly like one that does not exist.
    let blob = state
        .services
        .canvas_for_user(user.id, &canvas_hash)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "canvas lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    serve_canvas(&state, &blob, &headers, CANVAS_CANONICAL_CACHE).await
}

#[utoipa::path(
    get,
    path = "/api/v2/tracks/{track_id}/canvas",
    tag = "media",
    params(("track_id" = Uuid, Path)),
    responses(
        (status = 200, description = "The loop this track carries now"),
        (status = 206),
        (status = 304),
        (status = 401),
        (status = 404)
    )
)]
pub async fn canvas_for_track(
    State(state): State<AppState>,
    AxumPath(track_id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    let user = bearer_principal(&state, &headers).await?;
    // A track id is guessed no worse than a hash, so the alias is no more an
    // authorisation than the fingerprint is: membership in the track's library,
    // re-read on this request. A track that does not exist, one belonging to
    // somebody else, and one with no canvas all answer 404.
    let blob = state
        .services
        .canvas_for_track(user.id, track_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "canvas alias lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    serve_canvas(&state, &blob, &headers, CANVAS_ALIAS_CACHE).await
}

#[utoipa::path(
    put,
    path = "/api/v2/tracks/{track_id}/canvas",
    tag = "media",
    params(("track_id" = Uuid, Path)),
    request_body(content = Vec<u8>, description = "An mp4 or webm loop"),
    responses(
        (status = 200, body = CanvasResponse),
        (status = 401),
        (status = 404),
        (status = 409, description = "The library's canvas quota is full"),
        (status = 413),
        (status = 422, description = "Not a short loop this server will take")
    )
)]
pub async fn put_canvas(
    State(state): State<AppState>,
    AxumPath(track_id): AxumPath<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::Json<CanvasResponse>, MediaError> {
    let user = bearer_principal(&state, &headers).await?;
    let device = crate::api::origin_device(&state, &headers, user.id)
        .await
        .map_err(|_| MediaError::InvalidRequest)?;
    let blob = state
        .services
        .place_canvas(user.id, track_id, &body, device)
        .await
        .map_err(canvas_error)?;
    Ok(axum::Json(CanvasResponse {
        url: format!("/api/v2/canvas/{}", blob.hash),
        hash: blob.hash,
        format: blob.format,
        byte_size: blob.byte_size,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/v2/tracks/{track_id}/canvas",
    tag = "media",
    params(("track_id" = Uuid, Path)),
    responses((status = 204), (status = 401), (status = 404))
)]
pub async fn delete_canvas(
    State(state): State<AppState>,
    AxumPath(track_id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, MediaError> {
    let user = bearer_principal(&state, &headers).await?;
    let device = crate::api::origin_device(&state, &headers, user.id)
        .await
        .map_err(|_| MediaError::InvalidRequest)?;
    state
        .services
        .remove_canvas(user.id, track_id, device)
        .await
        .map_err(canvas_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/api/v2/tracks/{track_id}/canvas-ticket",
    tag = "media",
    params(("track_id" = Uuid, Path)),
    responses((status = 200, body = StreamTicketResponse), (status = 401), (status = 404))
)]
pub async fn create_canvas_ticket(
    State(state): State<AppState>,
    AxumPath(track_id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<axum::Json<StreamTicketResponse>, MediaError> {
    let user = bearer_principal(&state, &headers).await?;
    // Minting proves access now; redeeming re-proves it. A member removed
    // between the two loses the canvas immediately rather than at expiry.
    state
        .services
        .canvas_for_track(user.id, track_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "canvas ticket authorization lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    let expires_at = i64::try_from(state.stream_ticket_ttl.as_millis())
        .ok()
        .and_then(|ttl| crate::auth::authentication::now_ms().checked_add(ttl))
        .ok_or_else(|| {
            tracing::error!("stream ticket TTL does not fit in an epoch timestamp");
            MediaError::Internal
        })?;
    let ticket = crate::auth::stream_ticket::mint(
        &state.secret_box,
        crate::auth::stream_ticket::TicketKind::Canvas,
        user.id,
        track_id,
        expires_at,
    )
    .map_err(|error| {
        tracing::error!(error = %error, "canvas ticket minting failed");
        MediaError::Internal
    })?;
    // Relative, for the same reason the audio ticket is: it is consumed by the
    // client that just authenticated, not sent out of the application.
    Ok(axum::Json(StreamTicketResponse {
        url: format!("/api/v2/canvas-stream/{ticket}"),
        expires_at,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v2/canvas-stream/{ticket}",
    tag = "media",
    params(("ticket" = String, Path)),
    responses(
        (status = 200, description = "What <video src> plays"),
        (status = 206),
        (status = 304),
        (status = 404)
    )
)]
pub async fn canvas_with_ticket(
    State(state): State<AppState>,
    AxumPath(ticket): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    // This route serves a canvas, and it says so. Without the assertion an
    // audio ticket would open a canvas and the reverse — which is the whole
    // reason the kind is inside the sealed payload.
    let (_, user_id, track_id) = crate::auth::stream_ticket::verify(
        &state.secret_box,
        &ticket,
        crate::auth::authentication::now_ms(),
    )
    .filter(|(kind, ..)| *kind == crate::auth::stream_ticket::TicketKind::Canvas)
    .ok_or(MediaError::NotFound)?;
    // Re-checked on every redemption, so a ticket cannot outlive the membership
    // that justified it.
    let blob = state
        .services
        .canvas_for_track(user_id, track_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "ticket canvas authorization lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    // Addressed by a ticket that names a track, so this resolves the link of
    // the moment exactly as the alias does, and caches like it.
    serve_canvas(&state, &blob, &headers, CANVAS_ALIAS_CACHE).await
}

/// Maps a canvas refusal onto the status that says it.
///
/// `Forbidden` becomes 404 like everywhere else in this API: a caller who may
/// not place a canvas learns nothing about whether the library would have taken
/// one. `Invalid` is 422 rather than 400 — the request was well formed, and
/// what it carried is not a short loop this server will keep.
fn canvas_error(error: crate::services::ServiceError) -> MediaError {
    match error {
        crate::services::ServiceError::NotFound | crate::services::ServiceError::Forbidden => {
            MediaError::NotFound
        }
        crate::services::ServiceError::Invalid => MediaError::InvalidRequest,
        crate::services::ServiceError::Conflict => MediaError::Conflict,
        error => {
            tracing::error!(error = %error, "canvas mutation failed");
            MediaError::Internal
        }
    }
}

/// What a placed canvas answers with.
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct CanvasResponse {
    /// Where the bytes are, addressed by content. Immutable under this URL.
    pub url: String,
    pub hash: String,
    /// `mp4` or `webm`, decided by reading the bytes rather than by trusting
    /// what the request called them.
    pub format: String,
    pub byte_size: i64,
}
