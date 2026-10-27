//! The transcode cache: the FFmpeg command, the live transcode that fills the
//! cache from the first byte, the fill behind a seek, and the pruning pass.

use std::{
    collections::HashMap,
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};

use axum::{
    body::Body,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::{mpsc, Mutex, OwnedSemaphorePermit, Semaphore},
};
use tokio_stream::wrappers::ReceiverStream;
use uuid::Uuid;

use super::{file_input, MediaError, MediaInner, MediaService, OutputFormat, MEDIA_CACHE_CONTROL};
use crate::media::replay_gain::Gain;

const STREAM_CHUNK_SIZE: usize = 64 * 1024;
const TRANSCODE_PROFILE_VERSION: &str = "waveflow-ffmpeg-v1";

impl MediaService {
    /// The FFmpeg invocation for a transcode, up to where its output goes.
    ///
    /// Shared by the live stream, which sends it to a pipe, and the cache fill
    /// behind a seek, which sends it to a file. Two copies of these arguments
    /// would drift apart the day one of them changed, and the cache would start
    /// holding a different encode from the one its key names.
    pub(super) fn transcode_command(
        &self,
        source: &Path,
        format: OutputFormat,
        bitrate: u32,
        gain: Option<Gain>,
        offset_ms: u64,
    ) -> Result<Command, MediaError> {
        let mut command = Command::new(&self.inner.ffmpeg);
        command.arg("-hide_banner").arg("-loglevel").arg("error");
        if offset_ms > 0 {
            command
                .arg("-ss")
                .arg(format!("{:.3}", offset_ms as f64 / 1000.0));
        }
        file_input(&mut command, source).args(["-vn", "-map_metadata", "-1"]);
        if let Some(gain) = gain {
            command.arg("-af").arg(gain.filter());
        }
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
            OutputFormat::Aac => {
                // FFmpeg's own encoder, which every build has: `libfdk_aac` is
                // better at low rates and missing from most builds, since its
                // licence is not GPL-compatible.
                command.args(["-c:a", "aac", "-b:a", &bitrate_arg, "-f", "adts"]);
            }
            OutputFormat::Raw => return Err(MediaError::InvalidRequest),
        }
        Ok(command)
    }

    /// Transcodes the whole track into the cache, behind a seek into a live
    /// transcode of it. Issue #185.
    ///
    /// Seeking a live transcode goes through `offset_ms`, and asking for it is
    /// the very act that abandons the offset-0 stream: the only one allowed to
    /// fill the cache, and one whose departure kills its encoder and deletes its
    /// partial file. The stream replacing it is not the whole track and is not
    /// cached. Without this, a track seeked on its first play stayed a live
    /// transcode on every later play, without byte ranges, so the client seeked
    /// through `offset_ms` again and abandoned the stream again.
    ///
    /// **Below anything a listener is waiting on.** It starts only when it can
    /// take a global slot and still leave one free, so it never turns a live
    /// request into a 429, and it never counts against the account's own limit,
    /// which the seek itself is holding. Without that room it does nothing, and
    /// the next seek asks again.
    ///
    /// **Outside the cache lock.** The play a seek abandons still holds that
    /// lock for the moment it takes to notice its consumer is gone, and the seek
    /// lands inside that moment: waiting for the lock or giving up on it would
    /// both miss the one case this is for. A second writer of the same key costs
    /// an encode and nothing worse — each writes its own staging file, and the
    /// one that finds the cache already there discards its own.
    ///
    /// **Bounded by `deadline`.** A live stream ends when its listener leaves;
    /// a fill has no listener, so without a deadline an encode that never
    /// finished would hold its slot and its key until the server restarted.
    /// Past it, FFmpeg is killed and nothing it wrote is kept.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn fill_cache_behind_a_seek(
        &self,
        source: PathBuf,
        format: OutputFormat,
        bitrate: u32,
        gain: Option<Gain>,
        key: String,
        cache_path: PathBuf,
        deadline: Duration,
    ) {
        if !self.inner.pending_fills.insert(key.clone()) {
            return;
        }
        // Released on every way out from here on, the refusals included, so a
        // fill that could not start never stops the next seek from asking.
        let pending = PendingFill {
            inner: Arc::clone(&self.inner),
            key,
        };
        // Two permits in one step and one handed straight back: a slot, with
        // another still free after it. Checking for two and then taking one
        // would leave a gap in which a second fill takes the last.
        let Ok(mut global) = Arc::clone(&self.inner.global).try_acquire_many_owned(2) else {
            return;
        };
        drop(global.split(1));
        let Ok(command) = self.transcode_command(&source, format, bitrate, gain, 0) else {
            return;
        };
        let inner = Arc::clone(&self.inner);
        inner.active_transcodes.fetch_add(1, Ordering::Relaxed);

        tokio::spawn(async move {
            // Declared in this order so they drop in the reverse one: the key
            // and the slot are given back before the count reaches zero, so a
            // caller waiting on the count finds all three released.
            let _active = ActiveGuard(&inner.active_transcodes);
            let _global = global;
            let _pending = pending;
            encode_into_cache(&inner, command, &source, cache_path, deadline).await;
        });
    }

    /// Transcodes a seek into the cache before answering it, for a client that
    /// asked for a bounded range of it.
    ///
    /// A live transcode has no length, so a request for the first two bytes of
    /// a seek — the probe iOS sends before it plays anything, a seek included —
    /// could only be answered with the whole stream, and AVFoundation does not
    /// play a probe answered that way: Juliet asked three times and crashed (the
    /// 2026-10-27 campaign, Opus with `timeOffset`). Encoded whole first, under
    /// the seek's own key, the probe is answered with its two bytes and a total,
    /// and the ranges the player reads next come out of the same file.
    ///
    /// **A listener is waiting on it**, so it takes the listener's slots, as a
    /// live stream does, and is refused with 429 when there are none. **It
    /// outlives the request**: a client that gives up before the encode ends
    /// still finds it in the cache at its next probe. **It holds the key's lock
    /// to the end**, so a burst of probes makes one encode, and the probes
    /// behind the first are answered from its file.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn transcode_into_cache(
        &self,
        user_id: Uuid,
        source: PathBuf,
        format: OutputFormat,
        bitrate: u32,
        gain: Option<Gain>,
        offset_ms: u64,
        key: String,
        cache_path: PathBuf,
        deadline: Duration,
    ) -> Result<(), MediaError> {
        let lock = self
            .inner
            .cache_locks
            .entry(key)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let guard = lock.lock_owned().await;
        if tokio::fs::try_exists(&cache_path).await.unwrap_or(false) {
            return Ok(());
        }
        let (global, user) = self.listener_slots(user_id)?;
        let command = self.transcode_command(&source, format, bitrate, gain, offset_ms)?;
        let inner = Arc::clone(&self.inner);
        inner.active_transcodes.fetch_add(1, Ordering::Relaxed);
        let encode = tokio::spawn(async move {
            // The slots and the lock drop before the count, as in a fill.
            let _active = ActiveGuard(&inner.active_transcodes);
            let _held = (global, user, guard);
            encode_into_cache(&inner, command, &source, cache_path, deadline).await
        });
        match encode.await {
            Ok(true) => Ok(()),
            _ => Err(MediaError::Internal),
        }
    }

    /// A slot on the server and one of the account's own, for a transcode a
    /// listener is waiting on; `Busy` when either is spent.
    fn listener_slots(
        &self,
        user_id: Uuid,
    ) -> Result<(OwnedSemaphorePermit, OwnedSemaphorePermit), MediaError> {
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
        Ok((global, user))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn stream_transcode(
        &self,
        user_id: Uuid,
        source: PathBuf,
        format: OutputFormat,
        bitrate: u32,
        gain: Option<Gain>,
        offset_ms: u64,
        cache: Option<(PathBuf, tokio::sync::OwnedMutexGuard<()>)>,
    ) -> Result<Response, MediaError> {
        let (global, user) = self.listener_slots(user_id)?;

        let mut command = self.transcode_command(&source, format, bitrate, gain, offset_ms)?;
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
            let temp_path = cache_path.as_deref().map(staging_path);
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

/// Runs a transcode `command` into `cache_path` through a staging file, and
/// says whether the cache holds the encode afterwards.
///
/// Shared by the fill behind a seek and the encode of a seek a client probed,
/// which differ in who waits on them and never in how a file reaches the cache.
///
/// **Bounded by `deadline`.** Past it, FFmpeg is killed and nothing it wrote is
/// kept, so an encode that never finished cannot hold its slot and its key
/// until the server restarts.
async fn encode_into_cache(
    inner: &MediaInner,
    mut command: Command,
    source: &Path,
    cache_path: PathBuf,
    deadline: Duration,
) -> bool {
    let staging = staging_path(&cache_path);
    command
        .arg("-y")
        .arg(&staging)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::warn!(error = %error, "failed to spawn ffmpeg for a cache fill");
            return false;
        }
    };
    let finished = match tokio::time::timeout(deadline, child.wait()).await {
        Ok(status) => status.is_ok_and(|status| status.success()),
        // Killed and awaited before its staging file is removed: a file
        // a live process still holds cannot be deleted on Windows.
        Err(_) => {
            let _ = child.kill().await;
            false
        }
    };
    if !finished {
        tracing::warn!(
            path = %source.display(),
            "ffmpeg cache fill failed or ran past its deadline"
        );
        let _ = tokio::fs::remove_file(&staging).await;
        return false;
    }
    // A play from the start may have committed the same key meanwhile.
    // Its file is as good as this one, and replacing a file a reader
    // may be streaming from is not a write worth making.
    if tokio::fs::try_exists(&cache_path).await.unwrap_or(false) {
        let _ = tokio::fs::remove_file(&staging).await;
        return true;
    }
    if let Err(error) = tokio::fs::rename(&staging, &cache_path).await {
        tracing::warn!(error = %error, "transcode cache fill could not be committed");
        let _ = tokio::fs::remove_file(&staging).await;
        return false;
    }
    inner.cache_access.insert(cache_path, SystemTime::now());
    prune_cache(inner).await;
    true
}

/// Releases a key's claim on the cache fill however the fill ends, so one that
/// failed or never started does not stop the next seek from asking again.
struct PendingFill {
    inner: Arc<MediaInner>,
    key: String,
}

impl Drop for PendingFill {
    fn drop(&mut self) {
        self.inner.pending_fills.remove(&self.key);
    }
}

struct ActiveGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// How far past the track's own length a cache fill may run.
///
/// Encodes run many times faster than the music, so the track's length is
/// already a generous bound, and a fill slower than that has stopped being a
/// background job on a server with other work. The minute covers FFmpeg
/// starting on a busy machine, and a track whose length the scan did not learn.
const FILL_DEADLINE_MARGIN: Duration = Duration::from_secs(60);

/// The deadline a cache fill of a track this long runs under.
pub(super) fn fill_deadline(duration_ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(duration_ms).unwrap_or(0))
        .saturating_add(FILL_DEADLINE_MARGIN)
}

/// Where a transcode is written before it is whole, beside the name it will
/// take. The pruning pass skips `.part-`, and the suffix is unique, so two
/// writers of one key never share a file.
fn staging_path(cache_path: &Path) -> PathBuf {
    cache_path.with_extension(format!(
        "{}.part-{}",
        cache_path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("cache"),
        Uuid::new_v4()
    ))
}

pub(super) fn cache_key(
    source_hash: &str,
    format: OutputFormat,
    bitrate: u32,
    gain: Option<Gain>,
) -> String {
    // A gain is named only when there is one, so every key written before
    // ReplayGain existed is still the key of the same encode.
    let gain = gain
        .map(|gain| format!(":{}", gain.key()))
        .unwrap_or_default();
    blake3::hash(
        format!("{source_hash}:{format:?}:{bitrate}:{TRANSCODE_PROFILE_VERSION}{gain}").as_bytes(),
    )
    .to_hex()
    .to_string()
}

/// The key of a transcode that starts `offset_ms` into the track whose whole
/// encode is `key`. A seek has a cache entry of its own: no byte of the whole
/// track's file is where a given second begins.
pub(super) fn seek_cache_key(key: &str, offset_ms: u64) -> String {
    blake3::hash(format!("{key}@{offset_ms}").as_bytes())
        .to_hex()
        .to_string()
}

pub(super) fn extension(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Raw => "bin",
        OutputFormat::Mp3 => "mp3",
        OutputFormat::Opus => "opus",
        OutputFormat::Aac => "aac",
    }
}

pub(super) fn mime_for_format(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Raw => "application/octet-stream",
        OutputFormat::Mp3 => "audio/mpeg",
        OutputFormat::Opus => "audio/ogg",
        OutputFormat::Aac => "audio/aac",
    }
}

pub(super) async fn prune_cache(inner: &MediaInner) {
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
