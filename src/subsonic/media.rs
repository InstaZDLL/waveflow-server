//! Streaming, downloading and cover art.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) async fn media_response(
    state: &AppState,
    principal: &Principal,
    params: &Params,
    download: bool,
    range: Option<&str>,
) -> Result<Response, ProtocolError> {
    let id = params.uuid("id")?;
    let track = state
        .db
        .stream_track_for_user(principal.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(not_found)?;
    let requested_bitrate = params
        .first("maxBitRate")
        .map(|value| value.parse::<u32>())
        .transpose()
        .map_err(|_| invalid("Invalid bitrate"))?
        .filter(|bitrate| *bitrate > 0);
    // A format this server cannot produce — `flac`, `wav`, `alac`, a typo — is
    // read as no format at all rather than refused (RFC-022 decision 6): the
    // original when the cap admits it, MP3 at the cap when it does not. The
    // `Content-Type` names what was sent, which is how every client learns it.
    // A refusal plays nothing, and it was the one answer no other server gave.
    let named =
        params
            .first("format")
            .and_then(|format| match format.to_ascii_lowercase().as_str() {
                "raw" => Some(OutputFormat::Raw),
                "mp3" => Some(OutputFormat::Mp3),
                "opus" | "ogg" => Some(OutputFormat::Opus),
                "aac" | "m4a" | "adts" => Some(OutputFormat::Aac),
                _ => None,
            });
    let format = if download {
        OutputFormat::Raw
    } else if let Some(format) = named {
        format
    } else if requested_bitrate.is_some_and(|limit| {
        track
            .bitrate
            .and_then(|bitrate| u32::try_from(bitrate).ok())
            .is_none_or(|source| source > limit)
    }) {
        // Legacy Subsonic clients such as DSub always send maxBitRate, even
        // when it matches the source. Downsample only when the cap is lower;
        // otherwise preserve direct playback just like Navidrome.
        OutputFormat::Mp3
    } else {
        OutputFormat::Raw
    };
    let query = StreamQuery {
        format,
        bitrate: (format != OutputFormat::Raw)
            .then_some(requested_bitrate)
            .flatten(),
        offset_ms: params
            .first("timeOffset")
            .map(|value| {
                value
                    .parse::<u64>()
                    .ok()
                    .and_then(|seconds| seconds.checked_mul(1000))
                    .ok_or(())
            })
            .transpose()
            .map_err(|_| invalid("Invalid time offset"))?
            .unwrap_or(0),
    };
    // ReplayGain on the encode (RFC-022, what was still open): `replayGain=`
    // when the request names it — a parameter of this server's, which every
    // other server ignores — the operator's default otherwise. Only a
    // transcode carries it; the original is never re-encoded to apply it.
    let replay_gain = match params.first("replayGain") {
        Some(mode) => crate::replay_gain::ReplayGainMode::parse(mode)
            .ok_or_else(|| invalid("Invalid replayGain"))?,
        None => state.media.subsonic_replay_gain(),
    };
    match state
        .media
        .serve_with_replay_gain(principal.id, track, query, range, replay_gain)
        .await
    {
        Ok(mut response) => {
            if download {
                response.headers_mut().insert(
                    header::CONTENT_DISPOSITION,
                    "attachment".parse().expect("static header value"),
                );
            }
            Ok(response)
        }
        Err(MediaError::NotFound | MediaError::Unauthorized) => Err(not_found()),
        Err(MediaError::InvalidRequest) => Err(invalid("Invalid media parameters")),
        Err(error @ (MediaError::RangeNotSatisfiable(_) | MediaError::Busy)) => {
            Ok(error.into_response())
        }
        // `Conflict` belongs to the canvas routes, which have no Subsonic
        // surface — the facade is frozen for v2.0-beta and no validated client
        // asks for a loop. `serve` cannot produce it, so this arm exists to
        // keep the match exhaustive rather than to describe a reachable state,
        // and it says "internal" because reaching it would mean the media
        // service returned something it has no way to return.
        Err(MediaError::Internal | MediaError::Conflict) => Err(internal("media service failed")),
    }
}

pub(super) async fn cover_art_response(
    state: &AppState,
    principal: &Principal,
    params: &Params,
) -> Result<Response, ProtocolError> {
    let id = params.first("id").ok_or_else(missing)?;
    let (hash, format) = state
        .services
        .artwork_for_user(principal.id, id)
        .await
        .map_err(internal)?
        .ok_or_else(not_found)?;
    let (mime, bytes) = crate::media::read_artwork(&state.artwork_dir, &hash, &format)
        .await
        .ok_or_else(not_found)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}
