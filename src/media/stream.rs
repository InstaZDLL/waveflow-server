//! Playing a track's audio: the stream routes and their tickets, the path
//! check, downloads, and the byte ranges a file is served by.

use std::{
    ffi::OsStr,
    io,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
};
use serde::Deserialize;
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt},
    sync::Mutex,
};
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use super::transcode::{cache_key, extension, fill_deadline, mime_for_format, seek_cache_key};
use super::{
    bearer_principal, MediaError, MediaService, OutputFormat, StreamQuery, MEDIA_CACHE_CONTROL,
};
use crate::{media::replay_gain::ReplayGainMode, storage::catalog::StreamTrack, AppState};

const MAX_RANGE_BYTES: u64 = 8 * 1024 * 1024;

/// `?download=1` on a stream ticket (RFC-012 decision 5).
///
/// Its own extractor rather than a field on [`StreamQuery`], which the
/// Subsonic façade and the bearer route share and which has no business
/// learning a header choice only the ticket route makes.
#[derive(Debug, Default, Deserialize)]
pub struct DownloadQuery {
    download: Option<String>,
}

impl DownloadQuery {
    fn asked(&self) -> bool {
        matches!(self.download.as_deref(), Some("1" | "true"))
    }
}

impl MediaService {
    pub async fn serve(
        &self,
        user_id: Uuid,
        track: StreamTrack,
        query: StreamQuery,
        range: Option<&str>,
    ) -> Result<Response, MediaError> {
        self.serve_with_replay_gain(user_id, track, query, range, ReplayGainMode::Off)
            .await
    }

    /// [`Self::serve`], a transcode levelled by the track's ReplayGain under
    /// `mode` (RFC-022, what was still open). The original is served as it is
    /// whatever the mode says: the gain rides on an encode that happens anyway,
    /// and never causes one. A gain is part of the cache key, so a levelled
    /// encode is never handed to a request that did not ask for it; no gain —
    /// the mode off, or a track with no tags — keeps the key it always had.
    pub async fn serve_with_replay_gain(
        &self,
        user_id: Uuid,
        track: StreamTrack,
        query: StreamQuery,
        range: Option<&str>,
        mode: ReplayGainMode,
    ) -> Result<Response, MediaError> {
        let gain = crate::media::replay_gain::gain(track.replay_gain, mode);
        if !track.available {
            return Err(MediaError::NotFound);
        }
        let source = secure_track_path(
            &track.library_root,
            &track.relative_path,
            &self.inner.link_roots,
            &track.link_policy,
        )
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
        let key = cache_key(&track.full_hash, query.format, bitrate, gain);
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

        // A seek a client probed is kept under a key of its own (see
        // `transcode_into_cache`), and once there it is a file like any other:
        // every range of it is answered, not only the ones from the first byte.
        let seek = (query.offset_ms > 0).then(|| {
            let key = seek_cache_key(&key, query.offset_ms);
            let path = self
                .inner
                .cache_dir
                .join(format!("{key}.{}", extension(query.format)));
            (key, path)
        });
        if let Some((_, seek_path)) = &seek {
            if tokio::fs::try_exists(seek_path).await.unwrap_or(false) {
                self.inner
                    .cache_access
                    .insert(seek_path.clone(), SystemTime::now());
                return serve_file(seek_path, range, mime_for_format(query.format)).await;
            }
        }

        // A byte range has no stable meaning until a transcode is complete, so
        // seeking a live transcode goes through offset_ms instead. A range
        // that starts at the first byte is not a seek though: it is how every
        // player opens a resource. Refusing it made a client fail on the first
        // play of a track and succeed on the second, once the cache existed —
        // Feishin with `bytes=0-`, then Juliet with the `bytes=0-1` probe iOS
        // sends before anything else.
        if range.is_some_and(|value| !starts_at_the_first_byte(value)) {
            return Err(MediaError::RangeNotSatisfiable(None));
        }

        if let Some((seek_key, seek_path)) = seek {
            // A bounded range from the first byte asks how long the seek is,
            // which a live transcode cannot say. Anything else — no range, or
            // `bytes=0-` from a browser — is a listener who wants sound now.
            let response = if range.is_some_and(asks_for_a_bounded_start) {
                let encoded = self
                    .transcode_into_cache(
                        user_id,
                        source.clone(),
                        query.format,
                        bitrate,
                        gain,
                        query.offset_ms,
                        seek_key,
                        seek_path,
                        fill_deadline(track.duration_ms),
                    )
                    .await?;
                serve_opened(encoded, range, mime_for_format(query.format)).await?
            } else {
                self.stream_transcode(
                    user_id,
                    source.clone(),
                    query.format,
                    bitrate,
                    gain,
                    query.offset_ms,
                    None,
                )
                .await?
            };
            // After the live stream has its slots, never before: the listener
            // waiting on this request comes first, and a fill that took the
            // last slot would be the reason they were answered 429.
            if !tokio::fs::try_exists(&cache_path).await.unwrap_or(false) {
                self.fill_cache_behind_a_seek(
                    source,
                    query.format,
                    bitrate,
                    gain,
                    key,
                    cache_path,
                    fill_deadline(track.duration_ms),
                );
            }
            return Ok(response);
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
            // Whoever held the lock finished the transcode, so the range that
            // had no meaning a moment ago has one now, and the second caller
            // gets the same partial answer it would have got from the cache.
            return serve_file(&cache_path, range, mime_for_format(query.format)).await;
        }

        self.stream_transcode(
            user_id,
            source,
            query.format,
            bitrate,
            gain,
            0,
            Some((cache_path, guard)),
        )
        .await
    }
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct StreamTicketResponse {
    /// Path to play from, already containing the ticket.
    ///
    /// **Always relative to this server**, never absolute — unlike a share URL,
    /// which is prefixed with `WAVEFLOW_PUBLIC_URL` because it is meant to be
    /// sent outside the application. Clients may safely reject an absolute or
    /// protocol-relative value here, and are encouraged to.
    pub url: String,
    pub expires_at: i64,
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
    let user = bearer_principal(&state, &headers).await?;
    // Minting proves access now; redeeming re-proves it, so a ticket cannot
    // outlive the membership that justified it.
    state
        // A track this account reads, or a stored podcast episode (RFC-025
        // decision 11): the one lookup every stream route makes.
        .services
        .stream_source_for_user(user.id, track_id)
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
        .and_then(|ttl| crate::auth::authentication::now_ms().checked_add(ttl))
        .ok_or_else(|| {
            tracing::error!("stream ticket TTL does not fit in an epoch timestamp");
            MediaError::Internal
        })?;
    let ticket = crate::auth::stream_ticket::mint(
        &state.secret_box,
        crate::auth::stream_ticket::TicketKind::Audio,
        user.id,
        track_id,
        expires_at,
    )
    .map_err(|error| {
        tracing::error!(error = %error, "stream ticket minting failed");
        MediaError::Internal
    })?;
    // Deliberately relative, and deliberately unlike `createShare`, which does
    // prefix WAVEFLOW_PUBLIC_URL. A share link is made to leave the
    // application; a ticket is consumed by the client that just authenticated.
    // Handing it an absolute URL would let the server redirect playback to a
    // host the user never authenticated against — which is why native clients
    // reject absolute and protocol-relative values. Do not "harmonise" the two.
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
        ("format" = Option<String>, Query, description = "raw, mp3, opus or aac (ADTS)"),
        ("bitrate" = Option<u32>, Query),
        ("offset_ms" = Option<u64>, Query),
        ("download" = Option<String>, Query, description = "1: the original, as an attachment named after the file (RFC-012)")
    ),
    responses(
        (status = 200, description = "Original or live transcoded audio"),
        (status = 206, description = "Original or completed cached transcode range"),
        (status = 404),
        (status = 416),
        (status = 422, description = "A download asked for together with a transcode or an offset"),
        (status = 429)
    )
)]
pub async fn stream_with_ticket(
    State(state): State<AppState>,
    AxumPath(ticket): AxumPath<String>,
    Query(query): Query<StreamQuery>,
    Query(download): Query<DownloadQuery>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    // An invalid, forged or expired ticket is indistinguishable from an unknown
    // track, so probing tells an attacker nothing. A ticket of another kind
    // joins them: this route serves audio, and it says so rather than trusting
    // that only audio tickets reach it.
    let (_, user_id, track_id) = crate::auth::stream_ticket::verify(
        &state.secret_box,
        &ticket,
        crate::auth::authentication::now_ms(),
    )
    .filter(|(kind, ..)| *kind == crate::auth::stream_ticket::TicketKind::Audio)
    .ok_or(MediaError::NotFound)?;
    let track = state
        .services
        .stream_source_for_user(user_id, track_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "ticket stream authorization lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;
    let range = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok());
    if !download.asked() {
        return state.media.serve(user_id, track, query, range).await;
    }
    // A download is the file. A transcode, a bit rate or an offset would make
    // it something else under the file's own name, so asking for one with it
    // is refused rather than quietly ignored.
    if query.format != OutputFormat::Raw || query.bitrate.is_some() || query.offset_ms != 0 {
        return Err(MediaError::InvalidRequest);
    }
    let name = download_name(&state, user_id, &track).await;
    let mut response = state.media.serve(user_id, track, query, range).await?;
    response
        .headers_mut()
        .insert(header::CONTENT_DISPOSITION, attachment(&name));
    Ok(response)
}

/// The name a downloaded file is saved under.
///
/// A scanned track's own basename, which is what its owner called it. A track
/// received through RFC-008 is named by its hash on disk, so it is saved as
/// `Artist - Title.ext` from its tags instead — the hash is truthful and of no
/// use to anyone who opens their downloads folder.
async fn download_name(state: &AppState, user_id: Uuid, track: &StreamTrack) -> String {
    let path = Path::new(&track.relative_path);
    let basename = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("track")
        .to_owned();
    let received = track
        .relative_path
        .split('/')
        .next()
        .is_some_and(|first| first == crate::services::uploads::MANAGED_DIR);
    if !received {
        return basename;
    }
    let extension = path.extension().and_then(OsStr::to_str).unwrap_or("");
    let song = state
        .services
        .songs_by_ids(user_id, &[track.id])
        .await
        .ok()
        .and_then(|songs| songs.into_iter().next());
    let Some(song) = song else {
        return basename;
    };
    let stem = match song
        .artist
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty())
    {
        Some(artist) => format!("{artist} - {}", song.title.trim()),
        None => song.title.trim().to_owned(),
    };
    if stem.is_empty() {
        return basename;
    }
    if extension.is_empty() {
        stem
    } else {
        format!("{stem}.{extension}")
    }
}

/// `attachment` with the name in both forms RFC 6266 gives it.
///
/// `filename` is an ASCII fallback, every character a browser could misread
/// in a quoted string — a quote, a backslash, a control, anything outside
/// ASCII — replaced by `_`, and a separator too, so no browser builds a path
/// from it. `filename*` carries the real name, percent-encoded UTF-8, and
/// every current browser prefers it.
pub(crate) fn attachment(name: &str) -> HeaderValue {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let fallback: String = cleaned
        .chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' => c,
            _ => '_',
        })
        .collect();
    let encoded = percent_encoding::utf8_percent_encode(&cleaned, ATTR_CHAR).to_string();
    HeaderValue::from_str(&format!(
        "attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}"
    ))
    .unwrap_or_else(|_| HeaderValue::from_static("attachment"))
}

/// Everything but RFC 5987's `attr-char` is percent-encoded in `filename*`.
const ATTR_CHAR: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'#')
    .remove(b'$')
    .remove(b'&')
    .remove(b'+')
    .remove(b'-')
    .remove(b'.')
    .remove(b'^')
    .remove(b'_')
    .remove(b'`')
    .remove(b'|')
    .remove(b'~');

#[utoipa::path(
    get,
    path = "/api/v2/tracks/{track_id}/stream",
    tag = "media",
    params(
        ("track_id" = Uuid, Path),
        ("format" = Option<String>, Query, description = "raw, mp3, opus or aac (ADTS)"),
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
    let user = bearer_principal(&state, &headers).await?;
    let track = state
        // A track this account reads, or a stored podcast episode (RFC-025
        // decision 11): the one lookup every stream route makes.
        .services
        .stream_source_for_user(user.id, track_id)
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

///
/// **A link on the way** is refused, with one exception: a library following
/// links stores a file in an allowed folder outside its root under the path
/// the walk reached it through (RFC-021, links outside the root). Such a path
/// is accepted only while that library still follows links, and only when it
/// resolves, now, into one of `link_roots` and into no other library's root —
/// re-read on every request, so a link retargeted since the scan, or a links
/// setting turned off since, leads nowhere. Another library's file is never
/// reached through a link, whoever asks: the caller's access was checked on
/// this track's library only.
pub(crate) async fn secure_track_path(
    root: &Path,
    relative: &str,
    link_roots: &crate::library::link_roots::LinkRoots,
    policy: &crate::library::link_roots::LinkPolicy,
) -> io::Result<PathBuf> {
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
    let mut through_link = false;
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
            if !policy.follow_links || link_roots.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "symlink media path rejected",
                ));
            }
            through_link = true;
        }
    }
    let canonical_file = tokio::fs::canonicalize(&cursor).await?;
    let reachable = if through_link {
        // Off the async worker: resolving a root is a blocking call, and a
        // hung network mount would otherwise hold the worker with it.
        let (link_roots, policy, target) =
            (link_roots.clone(), policy.clone(), canonical_file.clone());
        tokio::task::spawn_blocking(move || {
            crate::library::link_roots::admits(
                &link_roots.resolved(),
                &policy.other_libraries_resolved(),
                &target,
            )
        })
        .await
        .map_err(io::Error::other)?
    } else {
        canonical_file.starts_with(&canonical_root)
    };
    if !reachable || !tokio::fs::metadata(&canonical_file).await?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "media path escapes library",
        ));
    }
    Ok(canonical_file)
}

pub(super) async fn serve_file(
    path: &Path,
    range: Option<&str>,
    mime: &'static str,
) -> Result<Response, MediaError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|_| MediaError::NotFound)?;
    serve_opened(file, range, mime).await
}

/// [`serve_file`] from a file already open: one the pruning pass may have
/// unlinked since, which an open handle still reads.
pub(super) async fn serve_opened(
    file: tokio::fs::File,
    range: Option<&str>,
    mime: &'static str,
) -> Result<Response, MediaError> {
    let size = file
        .metadata()
        .await
        .map_err(|_| MediaError::NotFound)?
        .len();
    match range {
        Some(range) => {
            let (start, end) =
                parse_range(range, size).ok_or(MediaError::RangeNotSatisfiable(Some(size)))?;
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

/// Whether a `Range` header starts at the first byte, which is how a player
/// opens a resource rather than how it seeks into one. Browsers send `bytes=0-`;
/// iOS AVFoundation first probes with `bytes=0-1` to learn the size and type.
/// Both mean "begin here", and neither can be refused without breaking the
/// first play of every track.
///
/// The answer to such a request is the whole stream rather than the bytes
/// asked for, which HTTP allows: a server may always ignore `Range` and answer
/// 200. Before the transcode exists there is no total length to put in a
/// `Content-Range`, so a partial answer is not available to give — except to a
/// seek, which is encoded whole first when [`asks_for_a_bounded_start`].
pub(super) fn starts_at_the_first_byte(value: &str) -> bool {
    value.strip_prefix("bytes=").is_some_and(|spec| {
        // A multipart range is several ranges, and this answers with one body.
        let Some((start, end)) = spec.trim().split_once('-') else {
            return false;
        };
        start.trim().parse::<u64>() == Ok(0)
            // A malformed end - a second hyphen, a word - is not a range this
            // understands, so it is refused rather than read as an open.
            && (end.trim().is_empty() || end.trim().parse::<u64>().is_ok())
    })
}

/// Whether a `Range` header asks for a bounded span from the first byte —
/// `bytes=0-1` above all, which asks for two bytes and the length of the whole.
/// `bytes=0-` asks for everything and is satisfied by a live stream.
pub(super) fn asks_for_a_bounded_start(value: &str) -> bool {
    starts_at_the_first_byte(value) && value.trim_end().ends_with(|c: char| c.is_ascii_digit())
}

pub(super) fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    if size == 0 {
        return None;
    }
    let value = value.strip_prefix("bytes=")?;
    if value.contains(',') {
        return None;
    }
    let (start, end) = value.trim().split_once('-')?;
    // Trimmed on both bounds, like `starts_at_the_first_byte`: a spelling the
    // cold path accepts must not be refused once the cache exists.
    let (start, end) = (start.trim(), end.trim());
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
        OutputFormat::Aac => 192,
        OutputFormat::Raw => return Err(MediaError::InvalidRequest),
    });
    if !(32..=512).contains(&bitrate) {
        return Err(MediaError::InvalidRequest);
    }
    Ok(bitrate)
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
