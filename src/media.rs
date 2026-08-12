//! Tenant-authorized native playback, FFmpeg transcoding and content cache.

use std::{
    collections::HashMap,
    ffi::OsStr,
    io,
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use bytes::Bytes;
use dashmap::DashMap;
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, Mutex, Semaphore},
};
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::{catalog::StreamTrack, config::Config, AppState};

const STREAM_CHUNK_SIZE: usize = 64 * 1024;
const MAX_RANGE_BYTES: u64 = 8 * 1024 * 1024;
const TRANSCODE_PROFILE_VERSION: &str = "waveflow-ffmpeg-v1";
/// Audio is tenant-scoped and reached through a bearer token or a stream
/// ticket, so the same URL means different things to different callers. No
/// shared cache may retain a copy.
const MEDIA_CACHE_CONTROL: &str = "private, no-store";

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
    per_user_limit: usize,
    users: DashMap<Uuid, Arc<Semaphore>>,
    cache_locks: DashMap<String, Arc<Mutex<()>>>,
    cache_access: DashMap<PathBuf, SystemTime>,
    active_transcodes: AtomicUsize,
    transcoding_available: bool,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Raw,
    Mp3,
    Opus,
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
    RangeNotSatisfiable(u64),
    Busy,
    Internal,
}

impl MediaService {
    pub async fn initialize(config: &Config) -> anyhow::Result<Self> {
        check_tool(&config.ffmpeg_path, "ffmpeg").await?;
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
                per_user_limit: config.transcode_per_user_limit,
                users: DashMap::new(),
                cache_locks: DashMap::new(),
                cache_access: DashMap::new(),
                active_transcodes: AtomicUsize::new(0),
                transcoding_available: true,
            }),
        })
    }

    pub fn ffprobe_path(&self) -> &Path {
        &self.inner.ffprobe
    }

    pub fn active_transcodes(&self) -> usize {
        self.inner.active_transcodes.load(Ordering::Relaxed)
    }

    /// The service is only constructed after both FFmpeg tools pass startup
    /// detection, so an initialized instance is the capability signal.
    pub fn transcoding_available(&self) -> bool {
        self.inner.transcoding_available
    }

    pub async fn serve(
        &self,
        user_id: Uuid,
        track: StreamTrack,
        query: StreamQuery,
        range: Option<&str>,
    ) -> Result<Response, MediaError> {
        if !track.available {
            return Err(MediaError::NotFound);
        }
        let source = secure_track_path(&track.library_root, &track.relative_path)
            .await
            .map_err(|error| {
                tracing::warn!(track_id = %track.id, library_id = %track.library_id, error = %error, "media path rejected");
                MediaError::NotFound
            })?;

        if query.format == OutputFormat::Raw {
            if query.offset_ms != 0 || query.bitrate.is_some() {
                return Err(MediaError::InvalidRequest);
            }
            return serve_file(&source, range, mime_for_path(&source)).await;
        }

        let bitrate = validate_bitrate(query.format, query.bitrate)?;
        if query.offset_ms > track.duration_ms.max(0) as u64 {
            return Err(MediaError::InvalidRequest);
        }
        let key = cache_key(&track.full_hash, query.format, bitrate);
        let cache_path = self
            .inner
            .cache_dir
            .join(format!("{key}.{}", extension(query.format)));

        if query.offset_ms == 0 && tokio::fs::try_exists(&cache_path).await.unwrap_or(false) {
            self.inner
                .cache_access
                .insert(cache_path.clone(), SystemTime::now());
            return serve_file(&cache_path, range, mime_for_format(query.format)).await;
        }

        // A byte range has no stable meaning until a transcode is complete.
        // Clients seek a live transcode with offset_ms instead.
        if range.is_some() {
            return Err(MediaError::RangeNotSatisfiable(0));
        }

        if query.offset_ms > 0 {
            return self
                .stream_transcode(
                    user_id,
                    source,
                    query.format,
                    bitrate,
                    query.offset_ms,
                    None,
                )
                .await;
        }

        let lock = self
            .inner
            .cache_locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let guard = lock.lock_owned().await;
        if tokio::fs::try_exists(&cache_path).await.unwrap_or(false) {
            drop(guard);
            self.inner
                .cache_access
                .insert(cache_path.clone(), SystemTime::now());
            return serve_file(&cache_path, None, mime_for_format(query.format)).await;
        }

        self.stream_transcode(
            user_id,
            source,
            query.format,
            bitrate,
            0,
            Some((cache_path, guard)),
        )
        .await
    }

    async fn stream_transcode(
        &self,
        user_id: Uuid,
        source: PathBuf,
        format: OutputFormat,
        bitrate: u32,
        offset_ms: u64,
        cache: Option<(PathBuf, tokio::sync::OwnedMutexGuard<()>)>,
    ) -> Result<Response, MediaError> {
        let global = Arc::clone(&self.inner.global)
            .try_acquire_owned()
            .map_err(|_| MediaError::Busy)?;
        let user_semaphore = self
            .inner
            .users
            .entry(user_id)
            .or_insert_with(|| Arc::new(Semaphore::new(self.inner.per_user_limit)))
            .clone();
        let user = user_semaphore
            .try_acquire_owned()
            .map_err(|_| MediaError::Busy)?;

        let mut command = Command::new(&self.inner.ffmpeg);
        command.arg("-hide_banner").arg("-loglevel").arg("error");
        if offset_ms > 0 {
            command
                .arg("-ss")
                .arg(format!("{:.3}", offset_ms as f64 / 1000.0));
        }
        command
            .arg("-i")
            .arg(&source)
            .args(["-vn", "-map_metadata", "-1"]);
        let bitrate_arg = format!("{bitrate}k");
        match format {
            OutputFormat::Mp3 => {
                command.args(["-c:a", "libmp3lame", "-b:a", &bitrate_arg, "-f", "mp3"]);
            }
            OutputFormat::Opus => {
                command.args([
                    "-c:a",
                    "libopus",
                    "-b:a",
                    &bitrate_arg,
                    "-vbr",
                    "on",
                    "-f",
                    "ogg",
                ]);
            }
            OutputFormat::Raw => return Err(MediaError::InvalidRequest),
        }
        command
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            tracing::error!(error = %error, "failed to spawn ffmpeg");
            MediaError::Internal
        })?;
        let mut stdout = child.stdout.take().ok_or(MediaError::Internal)?;
        let (sender, receiver) = mpsc::channel::<Result<Bytes, io::Error>>(8);
        let inner = Arc::clone(&self.inner);
        inner.active_transcodes.fetch_add(1, Ordering::Relaxed);

        tokio::spawn(async move {
            let _global = global;
            let _user = user;
            let _active = ActiveGuard(&inner.active_transcodes);
            let (cache_path, _cache_guard) = match cache {
                Some((path, guard)) => (Some(path), Some(guard)),
                None => (None, None),
            };
            let temp_path = cache_path.as_ref().map(|path| {
                path.with_extension(format!(
                    "{}.part-{}",
                    path.extension().and_then(OsStr::to_str).unwrap_or("cache"),
                    Uuid::new_v4()
                ))
            });
            let mut cache_file = match temp_path.as_ref() {
                Some(path) => match tokio::fs::File::create(path).await {
                    Ok(file) => Some(file),
                    Err(error) => {
                        tracing::warn!(error = %error, path = %path.display(), "transcode cache write disabled for request");
                        None
                    }
                },
                None => None,
            };
            let mut buffer = vec![0u8; STREAM_CHUNK_SIZE];
            let mut consumer_present = true;
            loop {
                // Watch for the consumer leaving alongside the read instead of
                // only noticing on the next send. FFmpeg can sit on a chunk for
                // a long time under load, and until it yields one there is
                // nothing to fail on, so an abandoned transcode would hold a
                // process and a concurrency slot for as long as the encode took.
                // Both branches are cancel-safe, and `biased` checks departure
                // first so a consumer that left during the read is seen at once.
                let read = tokio::select! {
                    biased;
                    _ = sender.closed() => {
                        consumer_present = false;
                        break;
                    }
                    result = stdout.read(&mut buffer) => result,
                };
                match read {
                    Ok(0) => break,
                    Ok(read) => {
                        if let Some(file) = cache_file.as_mut() {
                            if let Err(error) = file.write_all(&buffer[..read]).await {
                                tracing::warn!(error = %error, "transcode cache write failed");
                                cache_file = None;
                            }
                        }
                        if sender
                            .send(Ok(Bytes::copy_from_slice(&buffer[..read])))
                            .await
                            .is_err()
                        {
                            consumer_present = false;
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error)).await;
                        consumer_present = false;
                        break;
                    }
                }
            }

            if !consumer_present {
                let _ = child.kill().await;
                if let Some(path) = temp_path.as_ref() {
                    let _ = tokio::fs::remove_file(path).await;
                }
                return;
            }
            let success = child
                .wait()
                .await
                .map(|status| status.success())
                .unwrap_or(false);
            if success {
                if let Some(mut file) = cache_file {
                    if file.flush().await.is_ok() {
                        drop(file);
                        if let (Some(temp), Some(final_path)) =
                            (temp_path.as_ref(), cache_path.as_ref())
                        {
                            if tokio::fs::rename(temp, final_path).await.is_ok() {
                                inner
                                    .cache_access
                                    .insert(final_path.clone(), SystemTime::now());
                                prune_cache(&inner).await;
                            }
                        }
                    }
                } else if let Some(path) = temp_path.as_ref() {
                    let _ = tokio::fs::remove_file(path).await;
                }
            } else {
                tracing::warn!(path = %source.display(), "ffmpeg transcode failed");
                if let Some(path) = temp_path.as_ref() {
                    let _ = tokio::fs::remove_file(path).await;
                }
            }
        });

        let body = Body::from_stream(ReceiverStream::new(receiver));
        Ok((
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, mime_for_format(format)),
                (header::CACHE_CONTROL, MEDIA_CACHE_CONTROL),
                (header::ACCEPT_RANGES, "none"),
            ],
            body,
        )
            .into_response())
    }
}

struct ActiveGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Content type for a stored artwork, or `None` for a format we never wrote.
///
/// Shared by the native route and the Subsonic facade: two copies would drift
/// on the day a format is added, and the client that got
/// `application/octet-stream` would simply show no cover.
pub fn artwork_mime(format: &str) -> Option<&'static str> {
    match format {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Reads an artwork file from the store.
///
/// `hash` and `format` come from the database, never from the request, so the
/// joined path cannot escape `artwork_dir` — the caller resolves the request's
/// id through `artwork_for_user`, which also enforces tenancy.
pub async fn read_artwork(
    artwork_dir: &std::path::Path,
    hash: &str,
    format: &str,
) -> Option<(&'static str, Vec<u8>)> {
    let mime = artwork_mime(format)?;
    let bytes = tokio::fs::read(artwork_dir.join(format!("{hash}.{format}")))
        .await
        .ok()?;
    Some((mime, bytes))
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/v2/artwork/{artwork_id}", get(artwork))
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

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct StreamTicketResponse {
    /// Path to play from, already containing the ticket.
    pub url: String,
    pub expires_at: i64,
}

#[utoipa::path(
    get,
    path = "/api/v2/artwork/{artwork_id}",
    tag = "media",
    params((
        "artwork_id" = String,
        Path,
        description = "An `artwork_hash` from any catalogue payload, or the id of a track, \
                       album or artist that carries one."
    )),
    responses(
        (status = 200, description = "Cover image", content_type = "image/jpeg"),
        (status = 401),
        (status = 404)
    )
)]
/// Serves a cover behind the same bearer as the rest of the native API.
///
/// Catalogue payloads carry `artwork_hash` but nothing resolved it: only the
/// Subsonic facade served images, behind a separate credential. A native client
/// therefore showed a remote catalogue with no covers at all.
///
/// Access is re-checked per request through `artwork_for_user`, so a hash
/// guessed from another tenant's library answers 404 like anything else.
pub async fn artwork(
    State(state): State<AppState>,
    AxumPath(artwork_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or(MediaError::Unauthorized)?;
    let user = state
        .auth
        .authenticate(token)
        .await
        .map_err(|_| MediaError::Unauthorized)?;
    let (hash, format) = state
        .services
        .artwork_for_user(user.id, &artwork_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "artwork lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    let (mime, bytes) = read_artwork(&state.artwork_dir, &hash, &format)
        .await
        .ok_or(MediaError::NotFound)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime),
            // Immutable in practice: the hash *is* the content. Kept private so
            // a shared cache never serves one tenant's cover to another.
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        bytes,
    )
        .into_response())
}

#[utoipa::path(
    post,
    path = "/api/v2/tracks/{track_id}/stream-ticket",
    tag = "media",
    params(("track_id" = Uuid, Path)),
    responses(
        (status = 200, body = StreamTicketResponse),
        (status = 401),
        (status = 404)
    )
)]
pub async fn create_stream_ticket(
    State(state): State<AppState>,
    AxumPath(track_id): AxumPath<Uuid>,
    headers: HeaderMap,
) -> Result<axum::Json<StreamTicketResponse>, MediaError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or(MediaError::Unauthorized)?;
    let user = state
        .auth
        .authenticate(token)
        .await
        .map_err(|_| MediaError::Unauthorized)?;
    // Minting proves access now; redeeming re-proves it, so a ticket cannot
    // outlive the membership that justified it.
    state
        .db
        .stream_track_for_user(user.id, track_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "stream ticket authorization lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    // Saturating on overflow would mint a ticket that never expires; a TTL that
    // cannot be represented is a misconfiguration, not something to round off.
    let expires_at = i64::try_from(state.stream_ticket_ttl.as_millis())
        .ok()
        .and_then(|ttl| crate::authentication::now_ms().checked_add(ttl))
        .ok_or_else(|| {
            tracing::error!("stream ticket TTL does not fit in an epoch timestamp");
            MediaError::Internal
        })?;
    let ticket = crate::stream_ticket::mint(&state.secret_box, user.id, track_id, expires_at)
        .map_err(|error| {
            tracing::error!(error = %error, "stream ticket minting failed");
            MediaError::Internal
        })?;
    Ok(axum::Json(StreamTicketResponse {
        url: format!("/api/v2/stream/{ticket}"),
        expires_at,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v2/stream/{ticket}",
    tag = "media",
    params(
        ("ticket" = String, Path),
        ("format" = Option<String>, Query, description = "raw, mp3 or opus"),
        ("bitrate" = Option<u32>, Query),
        ("offset_ms" = Option<u64>, Query)
    ),
    responses(
        (status = 200, description = "Original or live transcoded audio"),
        (status = 206, description = "Original or completed cached transcode range"),
        (status = 404),
        (status = 416),
        (status = 429)
    )
)]
pub async fn stream_with_ticket(
    State(state): State<AppState>,
    AxumPath(ticket): AxumPath<String>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    // An invalid, forged or expired ticket is indistinguishable from an unknown
    // track, so probing tells an attacker nothing.
    let (user_id, track_id) =
        crate::stream_ticket::verify(&state.secret_box, &ticket, crate::authentication::now_ms())
            .ok_or(MediaError::NotFound)?;
    let track = state
        .db
        .stream_track_for_user(user_id, track_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "ticket stream authorization lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    state.media.serve(user_id, track, query, range).await
}

#[utoipa::path(
    get,
    path = "/api/v2/tracks/{track_id}/stream",
    tag = "media",
    params(
        ("track_id" = Uuid, Path),
        ("format" = Option<String>, Query, description = "raw, mp3 or opus"),
        ("bitrate" = Option<u32>, Query),
        ("offset_ms" = Option<u64>, Query)
    ),
    responses(
        (status = 200, description = "Original or live transcoded audio"),
        (status = 206, description = "Original or completed cached transcode range"),
        (status = 401),
        (status = 404),
        (status = 416),
        (status = 429)
    )
)]
pub async fn stream_track(
    State(state): State<AppState>,
    AxumPath(track_id): AxumPath<Uuid>,
    Query(query): Query<StreamQuery>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or(MediaError::Unauthorized)?;
    let user = state
        .auth
        .authenticate(token)
        .await
        .map_err(|_| MediaError::Unauthorized)?;
    let track = state
        .db
        .stream_track_for_user(user.id, track_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "stream authorization lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    state.media.serve(user.id, track, query, range).await
}

impl IntoResponse for MediaError {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthorized => StatusCode::UNAUTHORIZED.into_response(),
            Self::NotFound => StatusCode::NOT_FOUND.into_response(),
            Self::InvalidRequest => StatusCode::UNPROCESSABLE_ENTITY.into_response(),
            Self::Busy => {
                (StatusCode::TOO_MANY_REQUESTS, [(header::RETRY_AFTER, "1")]).into_response()
            }
            Self::RangeNotSatisfiable(size) => {
                let content_range = format!("bytes */{size}");
                (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    [
                        (header::CONTENT_RANGE, content_range.as_str()),
                        (header::ACCEPT_RANGES, "none"),
                    ],
                )
                    .into_response()
            }
            Self::Internal => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
}

async fn check_tool(path: &Path, name: &str) -> anyhow::Result<()> {
    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Command::new(path)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .status(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("{name} check timed out: {}", path.display()))?
    .map_err(|error| anyhow::anyhow!("{name} is required at {}: {error}", path.display()))?;
    if !output.success() {
        anyhow::bail!("{name} check failed at {} with {output}", path.display());
    }
    Ok(())
}

async fn secure_track_path(root: &Path, relative: &str) -> io::Result<PathBuf> {
    let canonical_root = tokio::fs::canonicalize(root).await?;
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "invalid relative media path",
        ));
    }

    let mut cursor = canonical_root.clone();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            unreachable!("components validated above")
        };
        cursor.push(component);
        if tokio::fs::symlink_metadata(&cursor)
            .await?
            .file_type()
            .is_symlink()
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "symlink media path rejected",
            ));
        }
    }
    let canonical_file = tokio::fs::canonicalize(&cursor).await?;
    if !canonical_file.starts_with(&canonical_root)
        || !tokio::fs::metadata(&canonical_file).await?.is_file()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "media path escapes library",
        ));
    }
    Ok(canonical_file)
}

async fn serve_file(
    path: &Path,
    range: Option<&str>,
    mime: &'static str,
) -> Result<Response, MediaError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| MediaError::NotFound)?;
    let size = file
        .metadata()
        .await
        .map_err(|_| MediaError::NotFound)?
        .len();
    match range {
        Some(range) => {
            let (start, end) =
                parse_range(range, size).ok_or(MediaError::RangeNotSatisfiable(size))?;
            serve_partial(file, start, end, size, mime).await
        }
        None => {
            let mut response = Response::new(Body::from_stream(ReaderStream::new(file)));
            *response.status_mut() = StatusCode::OK;
            response
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
            response.headers_mut().insert(
                header::CONTENT_LENGTH,
                HeaderValue::from_str(&size.to_string()).map_err(|_| MediaError::Internal)?,
            );
            response
                .headers_mut()
                .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static(MEDIA_CACHE_CONTROL),
            );
            Ok(response)
        }
    }
}

async fn serve_partial(
    mut file: tokio::fs::File,
    start: u64,
    end: u64,
    size: u64,
    mime: &'static str,
) -> Result<Response, MediaError> {
    file.seek(std::io::SeekFrom::Start(start))
        .await
        .map_err(|_| MediaError::Internal)?;
    let length = end - start + 1;
    let stream = ReaderStream::new(file.take(length));
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::PARTIAL_CONTENT;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).map_err(|_| MediaError::Internal)?,
    );
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(MEDIA_CACHE_CONTROL),
    );
    headers.insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes {start}-{end}/{size}"))
            .map_err(|_| MediaError::Internal)?,
    );
    Ok(response)
}

fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?.min(size).min(MAX_RANGE_BYTES);
        if suffix == 0 {
            return None;
        }
        return Some((size - suffix, size - 1));
    }
    let start = start.parse::<u64>().ok()?;
    if start >= size {
        return None;
    }
    let requested_end = if end.is_empty() {
        size - 1
    } else {
        end.parse::<u64>().ok()?.min(size - 1)
    };
    if requested_end < start {
        return None;
    }
    Some((start, requested_end.min(start + MAX_RANGE_BYTES - 1)))
}

fn validate_bitrate(format: OutputFormat, bitrate: Option<u32>) -> Result<u32, MediaError> {
    let bitrate = bitrate.unwrap_or(match format {
        OutputFormat::Mp3 => 192,
        OutputFormat::Opus => 128,
        OutputFormat::Raw => return Err(MediaError::InvalidRequest),
    });
    if !(32..=512).contains(&bitrate) {
        return Err(MediaError::InvalidRequest);
    }
    Ok(bitrate)
}

fn cache_key(source_hash: &str, format: OutputFormat, bitrate: u32) -> String {
    blake3::hash(
        format!("{source_hash}:{format:?}:{bitrate}:{TRANSCODE_PROFILE_VERSION}").as_bytes(),
    )
    .to_hex()
    .to_string()
}

fn extension(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Raw => "bin",
        OutputFormat::Mp3 => "mp3",
        OutputFormat::Opus => "opus",
    }
}

fn mime_for_format(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Raw => "application/octet-stream",
        OutputFormat::Mp3 => "audio/mpeg",
        OutputFormat::Opus => "audio/ogg",
    }
}

fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "m4a" | "mp4" => "audio/mp4",
        "aac" => "audio/aac",
        "dsf" | "dff" => "audio/dsd",
        _ => "application/octet-stream",
    }
}

async fn prune_cache(inner: &MediaInner) {
    let Ok(mut directory) = tokio::fs::read_dir(&inner.cache_dir).await else {
        return;
    };
    let mut files = Vec::new();
    let mut total = 0u64;
    while let Ok(Some(entry)) = directory.next_entry().await {
        let path = entry.path();
        if path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.contains(".part-"))
        {
            continue;
        }
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        total = total.saturating_add(metadata.len());
        let accessed = inner
            .cache_access
            .get(&path)
            .map(|value| *value)
            .or_else(|| metadata.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        files.push((accessed, path, metadata.len()));
    }
    files.sort_by_key(|(accessed, _, _)| *accessed);
    let mut removed = HashMap::new();
    for (_, path, size) in files {
        if total <= inner.cache_max_bytes {
            break;
        }
        if tokio::fs::remove_file(&path).await.is_ok() {
            total = total.saturating_sub(size);
            inner.cache_access.remove(&path);
            removed.insert(path, size);
        }
    }
    if !removed.is_empty() {
        tracing::debug!(files = removed.len(), "transcode cache pruned");
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, sync::Arc, time::SystemTime};

    use dashmap::DashMap;
    use tokio::sync::Semaphore;

    use super::{parse_range, prune_cache, secure_track_path, MediaInner};

    #[test]
    fn ranges_are_bounded_and_validated() {
        assert_eq!(parse_range("bytes=2-5", 10), Some((2, 5)));
        assert_eq!(parse_range("bytes=8-", 10), Some((8, 9)));
        assert_eq!(parse_range("bytes=-3", 10), Some((7, 9)));
        assert_eq!(parse_range("bytes=10-", 10), None);
        assert_eq!(parse_range("bytes=5-2", 10), None);
        assert_eq!(parse_range("bytes=0-1,4-5", 10), None);
    }

    #[tokio::test]
    async fn media_paths_reject_parent_components_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("library");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("safe.wav"), b"audio").unwrap();
        assert!(secure_track_path(&root, "safe.wav").await.is_ok());
        assert!(secure_track_path(&root, "../safe.wav").await.is_err());

        let link = root.join("linked.wav");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(root.join("safe.wav"), &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(root.join("safe.wav"), &link).is_ok();
        if linked {
            assert!(secure_track_path(&root, "linked.wav").await.is_err());
        }
    }

    #[tokio::test]
    async fn cache_pruning_removes_the_least_recently_used_file() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old.mp3");
        let recent = temp.path().join("recent.mp3");
        std::fs::write(&old, b"1234").unwrap();
        std::fs::write(&recent, b"5678").unwrap();
        let access = DashMap::new();
        access.insert(old.clone(), SystemTime::UNIX_EPOCH);
        access.insert(recent.clone(), SystemTime::now());
        let inner = MediaInner {
            ffmpeg: PathBuf::from("ffmpeg"),
            ffprobe: PathBuf::from("ffprobe"),
            cache_dir: temp.path().to_path_buf(),
            cache_max_bytes: 4,
            global: Arc::new(Semaphore::new(1)),
            per_user_limit: 1,
            users: DashMap::new(),
            cache_locks: DashMap::new(),
            cache_access: access,
            active_transcodes: std::sync::atomic::AtomicUsize::new(0),
            transcoding_available: true,
        };
        prune_cache(&inner).await;
        assert!(!old.exists());
        assert!(recent.exists());
    }
}
