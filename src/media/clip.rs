//! A track's music video: the routes that serve and ticket it (RFC-029
//! decisions 5 and 6).
//!
//! The file is the operator's, found beside the track by the scan, and served
//! as it is: whole or by range, never transcoded, remuxed or rewritten. There
//! is no route that places or removes one; the disk is the authority.

use axum::{
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap},
    response::Response,
};
use uuid::Uuid;

use super::stream::{secure_track_path, serve_file, StreamTicketResponse};
use super::{bearer_principal, MediaError, MediaService};
use crate::{auth::stream_ticket::TicketKind, services::ClipSource, AppState};

/// What the recorded format is served as. The scan records nothing else, and
/// whether a given player can open the container is the player's to say.
fn mime_for_clip(format: &str) -> &'static str {
    match format {
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        // `mp4` and `m4v`.
        _ => "video/mp4",
    }
}

impl MediaService {
    /// Serves a clip once something has authorised it, resolving the file at
    /// this request through the library's root, link roots and link policy
    /// exactly as the audio beside it is. A clip deleted since the scan answers
    /// 404, not a 500 and not stale bytes. Caching is the audio's: the URL
    /// resolves the link of the moment, which the next scan can change.
    pub async fn serve_clip(
        &self,
        clip: &ClipSource,
        range: Option<&str>,
    ) -> Result<Response, MediaError> {
        let path = secure_track_path(
            &clip.library_root,
            &clip.relative_path,
            &self.inner.link_roots,
            &clip.link_policy,
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                track_id = %clip.track_id,
                library_id = %clip.library_id,
                error = %error,
                "music video path rejected"
            );
            MediaError::NotFound
        })?;
        serve_file(&path, range, mime_for_clip(&clip.format)).await
    }
}

/// The clip a track has, for `user_id`, or the 404 every refusal shares.
async fn authorised_clip(
    state: &AppState,
    user_id: Uuid,
    track_id: Uuid,
) -> Result<ClipSource, MediaError> {
    state
        .services
        .clip_for_track(user_id, track_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "music video lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)
}

fn range(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
}

#[utoipa::path(
    get,
    path = "/api/v2/tracks/{track_id}/clip",
    tag = "media",
    params(("track_id" = Uuid, Path)),
    responses(
        (status = 200, description = "The music video this track has now"),
        (status = 206),
        (status = 401),
        (status = 404),
        (status = 416)
    )
)]
pub async fn clip_for_track(
    State(state): State<AppState>,
    AxumPath(track_id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    let user = bearer_principal(&state, &headers).await?;
    // A track that does not exist, one belonging to somebody else, one with
    // no clip and one whose clip is gone from the disk all answer 404.
    let clip = authorised_clip(&state, user.id, track_id).await?;
    state.media.serve_clip(&clip, range(&headers)).await
}

#[utoipa::path(
    post,
    path = "/api/v2/tracks/{track_id}/clip-ticket",
    tag = "media",
    params(("track_id" = Uuid, Path)),
    responses((status = 200, body = StreamTicketResponse), (status = 401), (status = 404))
)]
pub async fn create_clip_ticket(
    State(state): State<AppState>,
    AxumPath(track_id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<axum::Json<StreamTicketResponse>, MediaError> {
    let user = bearer_principal(&state, &headers).await?;
    // Minting proves access now; redeeming re-proves it.
    authorised_clip(&state, user.id, track_id).await?;
    // The audio's lifetime, for the audio's reason: the element reuses the URL
    // for every range request while seeking, and a clip is seeked into more
    // than a canvas.
    let expires_at = i64::try_from(state.stream_ticket_ttl.as_millis())
        .ok()
        .and_then(|ttl| crate::auth::authentication::now_ms().checked_add(ttl))
        .ok_or_else(|| {
            tracing::error!("stream ticket TTL does not fit in an epoch timestamp");
            MediaError::Internal
        })?;
    let ticket = crate::auth::stream_ticket::mint(
        &state.secret_box,
        TicketKind::Clip,
        user.id,
        track_id,
        expires_at,
    )
    .map_err(|error| {
        tracing::error!(error = %error, "music video ticket minting failed");
        MediaError::Internal
    })?;
    // Relative, as the audio's and the canvas's: the client prepends the base
    // URL it authenticated against.
    Ok(axum::Json(StreamTicketResponse {
        url: format!("/api/v2/clip-stream/{ticket}"),
        expires_at,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v2/clip-stream/{ticket}",
    tag = "media",
    params(("ticket" = String, Path)),
    responses(
        (status = 200, description = "What <video src> plays"),
        (status = 206),
        (status = 404),
        (status = 416)
    )
)]
pub async fn clip_with_ticket(
    State(state): State<AppState>,
    AxumPath(ticket): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    // This route serves a clip, and it says so: an audio or a canvas ticket
    // opens nothing here, as a clip ticket opens nothing there.
    let (_, user_id, track_id) = crate::auth::stream_ticket::verify(
        &state.secret_box,
        &ticket,
        crate::auth::authentication::now_ms(),
    )
    .filter(|(kind, ..)| *kind == TicketKind::Clip)
    .ok_or(MediaError::NotFound)?;
    // Re-checked on every redemption, so a removed member or a disabled
    // account loses the clip at once, not at the ticket's expiry.
    let clip = authorised_clip(&state, user_id, track_id).await?;
    state.media.serve_clip(&clip, range(&headers)).await
}
