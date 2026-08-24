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
    let format = if download {
        OutputFormat::Raw
    } else if let Some(format) = params.first("format") {
        match format {
            "raw" => OutputFormat::Raw,
            "mp3" => OutputFormat::Mp3,
            "opus" | "ogg" => OutputFormat::Opus,
            _ => return Err(invalid("Unsupported format")),
        }
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
    match state.media.serve(principal.id, track, query, range).await {
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
        Err(MediaError::Internal) => Err(internal("media service failed")),
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
