//! Tenant-authorized native playback, FFmpeg transcoding and content cache.

mod artwork;
mod canvas;
mod stream;
#[cfg(test)]
pub(crate) mod tests;
mod transcode;

pub use artwork::{__path_artwork, artwork, artwork_mime, read_artwork};
pub use canvas::{
    __path_canvas_by_hash, __path_canvas_for_track, __path_canvas_with_ticket,
    __path_create_canvas_ticket, __path_delete_canvas, __path_put_canvas, canvas_by_hash,
    canvas_for_track, canvas_with_ticket, create_canvas_ticket, delete_canvas, put_canvas,
    CanvasResponse,
};
pub use stream::{
    __path_create_stream_ticket, __path_stream_track, __path_stream_with_ticket,
    create_stream_ticket, stream_track, stream_with_ticket, DownloadQuery, StreamTicketResponse,
};
pub(crate) use stream::{attachment, secure_track_path};

use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use axum::handler::Handler;
use axum::{
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use dashmap::{DashMap, DashSet};
use serde::Deserialize;
use tokio::{
    process::Command,
    sync::{Mutex, Semaphore},
};
use uuid::Uuid;

use crate::{config::Config, replay_gain::ReplayGainMode, AppState};

/// Audio is tenant-scoped and reached through a bearer token or a stream
/// ticket, so the same URL means different things to different callers. No
/// shared cache may retain a copy.
const MEDIA_CACHE_CONTROL: &str = "private, no-store";

/// The protocols FFmpeg and ffprobe may open on behalf of an input
/// (RFC-023 decision 2): local files, and nothing else.
///
/// Every input this server hands either tool is a path it chose — a library
/// file, a staged upload, a cached encode — so `file` is all any of them
/// needs. FFmpeg's own default is wider, and it is a default nothing here
/// pinned: a top-level `http://` or `concat:` input is opened as asked, and a
/// file opened through `file` may still name `crypto` and `data` resources from
/// inside it. Named here, the list is the same on every build the server runs
/// against, and a playlist-shaped file cannot reach the network whatever the
/// installed FFmpeg would have allowed.
///
/// The whitelist is an input option, so it governs the input it precedes and
/// every resource opened from inside it. Outputs are named by this server
/// (`pipe:1`, a staging file) and are not affected.
pub(crate) const INPUT_PROTOCOLS: &str = "file";

/// Names `path` as the input of an FFmpeg or ffprobe command, under
/// [`INPUT_PROTOCOLS`].
///
/// The one way an input is given to either tool, so no call site can forget
/// the whitelist: `-protocol_whitelist file -i <path>`. ffprobe takes `-i` as
/// FFmpeg does.
pub(crate) fn file_input<'a>(command: &'a mut Command, path: &Path) -> &'a mut Command {
    command
        .arg("-protocol_whitelist")
        .arg(INPUT_PROTOCOLS)
        .arg("-i")
        .arg(path)
}

#[derive(Clone)]
pub struct MediaService {
    inner: Arc<MediaInner>,
}

struct MediaInner {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
    cache_dir: PathBuf,
    cache_max_bytes: u64,
    global: Arc<Semaphore>,
    global_limit: usize,
    per_user_limit: usize,
    users: DashMap<Uuid, Arc<Semaphore>>,
    cache_locks: DashMap<String, Arc<Mutex<()>>>,
    cache_access: DashMap<PathBuf, SystemTime>,
    /// Cache keys a fill behind a seek is already working on, so a burst of
    /// seeks on one track starts one encode rather than one per seek.
    pending_fills: DashSet<String>,
    active_transcodes: AtomicUsize,
    transcoding_available: bool,
    /// SHA-256 of the whole `ffmpeg -version` output (RFC-016 decision 5): a
    /// quality measurement is current only under the FFmpeg that made it.
    ffmpeg_digest: String,
    /// The folders outside every library a stored path may lead into through
    /// a link (`WAVEFLOW_LINK_ROOTS`).
    link_roots: crate::link_roots::LinkRoots,
    /// What a Subsonic transcode applies of ReplayGain unless the request
    /// says otherwise (`WAVEFLOW_SUBSONIC_REPLAYGAIN`).
    subsonic_replay_gain: ReplayGainMode,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Raw,
    Mp3,
    Opus,
    /// AAC-LC in an ADTS stream (RFC-022 decision 5): self-describing frames
    /// that stream from the first byte down a pipe and seek by byte range once
    /// cached. A regular MP4 cannot be written to a pipe, and a fragmented one
    /// carries no index to seek by.
    Aac,
}

impl OutputFormat {
    /// Every format a transcode can produce, as the native API spells them.
    /// `GET /api/v2/transcode/status` lists them so a client can offer AAC
    /// without inferring it from a version number.
    pub const TRANSCODED: [Self; 3] = [Self::Mp3, Self::Opus, Self::Aac];

    pub fn name(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
            Self::Aac => "aac",
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub format: OutputFormat,
    pub bitrate: Option<u32>,
    #[serde(default, alias = "offsetMs")]
    pub offset_ms: u64,
}

#[derive(Debug)]
pub enum MediaError {
    Unauthorized,
    NotFound,
    InvalidRequest,
    /// The range could not be honoured. The length is `None` when the
    /// representation does not have one yet — a transcode still being produced
    /// knows how many bytes it has written, never how many it will.
    RangeNotSatisfiable(Option<u64>),
    /// A quota the caller cannot spend past. Distinct from a refusal of the
    /// bytes themselves: nothing is wrong with what was sent, there is only no
    /// room for it.
    Conflict,
    Busy,
    Internal,
}

impl MediaService {
    pub async fn initialize(config: &Config) -> anyhow::Result<Self> {
        let ffmpeg_version = check_tool(&config.ffmpeg_path, "ffmpeg").await?;
        check_tool(&config.ffprobe_path, "ffprobe").await?;
        tokio::fs::create_dir_all(&config.transcode_cache_dir)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "cannot create transcode cache {}: {error}",
                    config.transcode_cache_dir.display()
                )
            })?;
        Ok(Self {
            inner: Arc::new(MediaInner {
                ffmpeg: config.ffmpeg_path.clone(),
                ffprobe: config.ffprobe_path.clone(),
                cache_dir: config.transcode_cache_dir.clone(),
                cache_max_bytes: config.transcode_cache_max_bytes,
                global: Arc::new(Semaphore::new(config.transcode_global_limit)),
                global_limit: config.transcode_global_limit,
                per_user_limit: config.transcode_per_user_limit,
                users: DashMap::new(),
                cache_locks: DashMap::new(),
                cache_access: DashMap::new(),
                pending_fills: DashSet::new(),
                link_roots: crate::link_roots::LinkRoots::new(config.link_roots.clone()),
                subsonic_replay_gain: config.subsonic_replay_gain,
                active_transcodes: AtomicUsize::new(0),
                transcoding_available: true,
                ffmpeg_digest: crate::quality::decoder_digest(&ffmpeg_version),
            }),
        })
    }

    pub fn ffprobe_path(&self) -> &Path {
        &self.inner.ffprobe
    }

    /// Which FFmpeg this process runs, as `track_quality.decoder` records it.
    pub fn ffmpeg_digest(&self) -> &str {
        &self.inner.ffmpeg_digest
    }

    pub fn active_transcodes(&self) -> usize {
        self.inner.active_transcodes.load(Ordering::Relaxed)
    }

    /// The two ceilings a `429` on the stream route is enforcing: the whole
    /// server's, then one account's.
    ///
    /// Reported so a client can size its own concurrency before asking. Without
    /// them the only way to find the limit is to be refused by it, which costs
    /// a request and says nothing about how far over the line the caller was.
    pub fn transcode_limits(&self) -> (usize, usize) {
        (self.inner.global_limit, self.inner.per_user_limit)
    }

    /// What a Subsonic transcode applies of ReplayGain when the request does
    /// not say.
    pub fn subsonic_replay_gain(&self) -> ReplayGainMode {
        self.inner.subsonic_replay_gain
    }

    /// The service is only constructed after both FFmpeg tools pass startup
    /// detection, so an initialized instance is the capability signal.
    pub fn transcoding_available(&self) -> bool {
        self.inner.transcoding_available
    }
}

impl IntoResponse for MediaError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::InvalidRequest => StatusCode::UNPROCESSABLE_ENTITY.into_response(),
            Self::Conflict => StatusCode::CONFLICT.into_response(),
            Self::Busy => {
                (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "1")]).into_response()
            }
            Self::RangeNotSatisfiable(size) => {
                // Two refusals wear this status, and they do not mean the same
                // thing.
                //
                // A complete file knows its length, so it can say which range
                // would have been satisfiable — and it accepts ranges, which
                // every other answer `serve_file` gives for that same resource
                // already says. Only the range asked for was wrong.
                //
                // A transcode still being produced knows how many bytes it has
                // written, never how many it will, and takes no range until it
                // is whole. `bytes */0` would not say "length unknown": it
                // would say the resource is empty, which a client is entitled
                // to believe and never ask about again. The unsatisfied-range
                // form has nowhere to put an unknown length, so the header is
                // left out instead of filled with a falsehood.
                let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
                let headers = response.headers_mut();
                match size {
                    Some(size) => {
                        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
                        if let Ok(value) = HeaderValue::from_str(&format!("bytes */{size}")) {
                            headers.insert(header::CONTENT_RANGE, value);
                        }
                    }
                    None => {
                        headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("none"));
                    }
                }
                response
            }
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

/// How long `ffmpeg -version` and `ffprobe -version` get at start-up.
///
/// The check refuses to boot on a binary that is missing or hangs; it is not
/// timing a healthy one. Five seconds was overrun on the Windows CI runner,
/// where a parallel test run starts several cold FFmpeg processes at once
/// (#281). Thirty still catches a hung binary, and a healthy boot answers long
/// before it, so the only thing the larger budget changes is how long a broken
/// installation takes to say so.
const TOOL_CHECK_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `<tool> -version` and answers what it printed.
///
/// The output is kept rather than discarded because FFmpeg's identifies the
/// decoder a quality measurement was made through (RFC-016 decision 5).
async fn check_tool(path: &Path, name: &str) -> anyhow::Result<Vec<u8>> {
    let output = tokio::time::timeout(
        TOOL_CHECK_TIMEOUT,
        Command::new(path)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("{name} check timed out: {}", path.display()))?
    .map_err(|error| anyhow::anyhow!("{name} is required at {}: {error}", path.display()))?;
    if !output.status.success() {
        anyhow::bail!(
            "{name} check failed at {} with {}",
            path.display(),
            output.status
        );
    }
    Ok(output.stdout)
}

pub fn router(state: AppState) -> Router {
    // Read before the state moves into the router: the ceiling follows the
    // configured canvas size, so the two cannot disagree.
    let canvas_body_limit = state.canvas_body_limit;
    Router::new()
        .route("/api/v2/artwork/{artwork_id}", get(artwork))
        .route("/api/v2/canvas/{canvas_hash}", get(canvas_by_hash))
        .route(
            "/api/v2/tracks/{track_id}/canvas",
            get(canvas_for_track)
                .put(put_canvas.layer(axum::extract::DefaultBodyLimit::max(canvas_body_limit)))
                .delete(delete_canvas),
        )
        .route(
            "/api/v2/tracks/{track_id}/canvas-ticket",
            axum::routing::post(create_canvas_ticket),
        )
        // Outside the bearer surface, like the audio one and for the same
        // reason: <video src> cannot send an Authorization header.
        .route("/api/v2/canvas-stream/{ticket}", get(canvas_with_ticket))
        .route("/api/v2/tracks/{track_id}/stream", get(stream_track))
        .route(
            "/api/v2/tracks/{track_id}/stream-ticket",
            axum::routing::post(create_stream_ticket),
        )
        // Deliberately outside the bearer-authenticated surface: the sealed
        // ticket in the path is the credential, because <audio src> cannot send
        // an Authorization header.
        .route("/api/v2/stream/{ticket}", get(stream_with_ticket))
        .with_state(state)
}

/// Resolves a Bearer caller for the media routes through the same door as
/// every other route.
///
/// These three used to extract the header and call the authenticator
/// themselves, which meant they were outside the one place scopes are
/// checked. They are reads, so today the check grants what it already
/// granted; the point is that a mutation added here cannot quietly skip it,
/// which is exactly how the scope list came to be stored and never read.
///
/// `/api/v2/stream/{ticket}` is deliberately not routed through this: its
/// credential is the sealed ticket in the path, because `<audio src>` cannot
/// send an Authorization header.
async fn bearer_principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<crate::authentication::AuthUser, MediaError> {
    crate::api::authenticated(state, headers, crate::api::Access::Read)
        .await
        .map_err(|_| MediaError::Unauthorized)
}
