//! Server-authoritative filesystem scanner with SSE-ready progress events.

use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::UNIX_EPOCH,
};

use dashmap::DashMap;
use futures_util::{stream, StreamExt};
use lofty::{
    file::TaggedFileExt,
    prelude::{Accessor, AudioFile, ItemKey},
};
use serde::Serialize;
use tokio::sync::{broadcast, Mutex};
use utoipa::ToSchema;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{
    catalog::{ApplyOutcome, ArtworkInput, CatalogApply, CatalogTrackInput, LibraryRecord},
    database::Database,
    lyrics::{self, LyricsInput},
};

const MAX_LYRICS_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScanProgress {
    pub scan_id: Uuid,
    pub library_id: Uuid,
    pub status: String,
    pub total: usize,
    pub processed: usize,
    pub added: i64,
    pub updated: i64,
    pub moved: i64,
    pub skipped: i64,
    pub unavailable: i64,
    pub errors: i64,
    pub current_path: Option<String>,
    pub message: Option<String>,
}

#[derive(Clone)]
pub struct ScanManager {
    db: Database,
    artwork_dir: PathBuf,
    parallelism: usize,
    events: Arc<DashMap<Uuid, broadcast::Sender<ScanProgress>>>,
    library_locks: Arc<DashMap<Uuid, Arc<Mutex<()>>>>,
}

impl ScanManager {
    pub fn new(db: Database, artwork_dir: PathBuf, parallelism: usize) -> Self {
        Self {
            db,
            artwork_dir,
            parallelism: parallelism.max(1),
            events: Arc::new(DashMap::new()),
            library_locks: Arc::new(DashMap::new()),
        }
    }

    pub async fn trigger(
        &self,
        library: LibraryRecord,
        requested_by: Option<Uuid>,
        trigger: &'static str,
    ) -> anyhow::Result<Uuid> {
        let scan_id = self
            .db
            .create_scan_job(library.id, requested_by, trigger)
            .await?;
        self.spawn(scan_id, library);
        Ok(scan_id)
    }

    /// Runs a job that already exists.
    ///
    /// Split out of [`Self::trigger`] so a caller that creates the job under
    /// its own tenancy guard does not have to create a second one to get it
    /// running.
    pub fn spawn(&self, scan_id: Uuid, library: LibraryRecord) {
        let library_id = library.id;
        let (sender, _) = broadcast::channel(128);
        self.events.insert(scan_id, sender);
        let manager = self.clone();
        tokio::spawn(async move {
            if let Err(error) = manager.run(scan_id, library).await {
                tracing::error!(%scan_id, error = %error, "library scan failed");
                let message = error.to_string();
                let _ = manager.db.fail_scan_job(scan_id, &message).await;
                manager.publish(ScanProgress {
                    scan_id,
                    library_id,
                    status: "failed".into(),
                    total: 0,
                    processed: 0,
                    added: 0,
                    updated: 0,
                    moved: 0,
                    skipped: 0,
                    unavailable: 0,
                    errors: 1,
                    current_path: None,
                    message: Some(message),
                });
            }
        });
    }

    pub fn subscribe(&self, scan_id: Uuid) -> Option<broadcast::Receiver<ScanProgress>> {
        self.events.get(&scan_id).map(|sender| sender.subscribe())
    }

    pub fn spawn_background(&self, interval: Option<std::time::Duration>) {
        let manager = self.clone();
        tokio::spawn(async move {
            manager.trigger_all("boot").await;
            let Some(interval) = interval else { return };
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                manager.trigger_all("scheduled").await;
            }
        });
    }

    async fn trigger_all(&self, trigger: &'static str) {
        match self.db.all_libraries().await {
            Ok(libraries) => {
                for library in libraries {
                    if let Err(error) = self.trigger(library, None, trigger).await {
                        tracing::warn!(error = %error, "could not queue background scan");
                    }
                }
            }
            Err(error) => tracing::warn!(error = %error, "could not list libraries for scanner"),
        }
    }

    async fn run(&self, scan_id: Uuid, library: LibraryRecord) -> anyhow::Result<()> {
        let lock = self
            .library_locks
            .entry(library.id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _library_guard = lock.lock().await;
        tokio::fs::create_dir_all(&self.artwork_dir).await?;
        let root = tokio::fs::canonicalize(&library.root_path).await?;
        let paths = tokio::task::spawn_blocking({
            let root = root.clone();
            move || discover_audio(&root)
        })
        .await??;
        let discovered: HashSet<String> = paths
            .iter()
            .filter_map(|path| relative_path(&root, path).ok())
            .collect();
        self.db.start_scan_job(scan_id, paths.len() as i64).await?;

        let root_for_tasks = root.clone();
        let artwork_dir = self.artwork_dir.clone();
        let mut extracted = stream::iter(paths.into_iter().map(|path| {
            let root = root_for_tasks.clone();
            let artwork = artwork_dir.clone();
            async move {
                let relative = relative_path(&root, &path);
                let result =
                    tokio::task::spawn_blocking(move || extract_file(&path, &artwork)).await;
                (relative, result)
            }
        }))
        .buffer_unordered(self.parallelism)
        .collect::<Vec<_>>()
        .await;
        extracted.sort_by(|left, right| scan_result_path(left).cmp(scan_result_path(right)));

        let mut progress = ScanProgress {
            scan_id,
            library_id: library.id,
            status: "running".into(),
            total: discovered.len(),
            processed: 0,
            added: 0,
            updated: 0,
            moved: 0,
            skipped: 0,
            unavailable: 0,
            errors: 0,
            current_path: None,
            message: None,
        };

        for batch in extracted.chunks(25) {
            let mut applies = Vec::new();
            let mut seen = Vec::new();
            let mut errors = Vec::new();
            let mut claimed_relocations = HashSet::new();
            for (relative, task_result) in batch {
                progress.processed += 1;
                match (relative, task_result) {
                    (Ok(relative), Ok(Ok(input))) => {
                        let mut input = input.clone();
                        progress.current_path = Some(relative.clone());
                        input.relative_path = relative.clone();
                        let existing = self.db.existing_track_by_path(library.id, relative).await?;
                        if let Some(existing) = existing.as_ref() {
                            if existing.file_size == input.file_size
                                && existing.modified_at == input.modified_at
                                && existing.quick_hash == input.quick_hash
                                && existing.full_hash == input.full_hash
                                && existing.lyrics_hash.as_deref() == Some(&input.lyrics_hash)
                                && existing.available
                            {
                                seen.push(existing.id);
                                progress.skipped += 1;
                                continue;
                            }
                        }
                        let mut moved = false;
                        let existing_id = if let Some(existing) = existing {
                            Some(existing.id)
                        } else {
                            let candidates = self
                                .db
                                .relocation_candidates(library.id, &input.full_hash)
                                .await?;
                            let missing = candidates
                                .into_iter()
                                .filter(|candidate| {
                                    !discovered.contains(&candidate.relative_path)
                                        && !claimed_relocations.contains(&candidate.id)
                                })
                                .collect::<Vec<_>>();
                            if missing.len() == 1 {
                                moved = true;
                                claimed_relocations.insert(missing[0].id);
                                Some(missing[0].id)
                            } else {
                                None
                            }
                        };
                        applies.push(CatalogApply {
                            input,
                            existing_id,
                            moved,
                        });
                    }
                    (relative, result) => {
                        progress.errors += 1;
                        let path = relative
                            .as_ref()
                            .cloned()
                            .unwrap_or_else(|_| "<invalid-relative-path>".into());
                        let message = match result {
                            Ok(Err(error)) => error.clone(),
                            Err(error) => format!("extract worker failed: {error}"),
                            Ok(Ok(_)) => "relative path failed".into(),
                        };
                        progress.current_path = Some(path.clone());
                        errors.push((path, message));
                    }
                }
            }
            self.db.mark_tracks_seen(&seen, scan_id).await?;
            for outcome in self
                .db
                .apply_catalog_tracks(library.id, scan_id, &applies)
                .await?
            {
                match outcome {
                    ApplyOutcome::Added => progress.added += 1,
                    ApplyOutcome::Updated => progress.updated += 1,
                    ApplyOutcome::Moved => progress.moved += 1,
                }
            }
            self.db.add_scan_errors(scan_id, &errors).await?;
            self.tick(&progress).await?;
        }

        progress.unavailable = self.db.mark_unseen_unavailable(library.id, scan_id).await?;
        self.db
            .finish_scan_job(scan_id, progress.unavailable)
            .await?;
        progress.status = "completed".into();
        progress.current_path = None;
        self.publish(progress);
        Ok(())
    }

    async fn tick(&self, progress: &ScanProgress) -> Result<(), sqlx::Error> {
        if progress.processed == progress.total || progress.processed.is_multiple_of(25) {
            self.db
                .update_scan_job(
                    scan_id(progress),
                    progress.processed as i64,
                    progress.added,
                    progress.updated,
                    progress.moved,
                    progress.skipped,
                    progress.errors,
                    progress.current_path.as_deref(),
                )
                .await?;
            self.publish(progress.clone());
        }
        Ok(())
    }

    fn publish(&self, progress: ScanProgress) {
        if let Some(sender) = self.events.get(&progress.scan_id) {
            let _ = sender.send(progress);
        }
    }
}

fn scan_result_path(
    result: &(
        anyhow::Result<String>,
        Result<Result<CatalogTrackInput, String>, tokio::task::JoinError>,
    ),
) -> &str {
    result.0.as_deref().unwrap_or("<invalid-relative-path>")
}

fn scan_id(progress: &ScanProgress) -> Uuid {
    progress.scan_id
}

fn discover_audio(root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        if entry.file_type().is_symlink() || !entry.file_type().is_file() {
            continue;
        }
        let supported = entry
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .is_some_and(|ext| {
                waveflow_core::scanner::AUDIO_EXTENSIONS
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(ext))
            });
        if supported {
            paths.push(entry.into_path());
        }
    }
    paths.sort();
    Ok(paths)
}

fn relative_path(root: &Path, path: &Path) -> anyhow::Result<String> {
    let canonical = fs::canonicalize(path)?;
    let relative = canonical
        .strip_prefix(root)
        .map_err(|_| anyhow::anyhow!("path escaped library root"))?;
    Ok(relative.to_string_lossy().replace('\\', "/"))
}

fn extract_file(path: &Path, artwork_dir: &Path) -> Result<CatalogTrackInput, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("metadata: {error}"))?;
    let size = metadata.len() as i64;
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0);
    let quick_hash =
        waveflow_core::scanner::hash_file(path).map_err(|error| format!("quick hash: {error}"))?;
    let full_hash = waveflow_core::scanner::hash_file_full(path)
        .map_err(|error| format!("full hash: {error}"))?;
    let relative_path = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("unknown")
        .to_owned();

    if matches!(
        path.extension()
            .and_then(|v| v.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("dsf" | "dff")
    ) {
        return extract_dsd(
            path,
            artwork_dir,
            relative_path,
            size,
            modified_at,
            quick_hash,
            full_hash,
        );
    }
    let tagged = lofty::read_from_path(path).map_err(|error| format!("lofty: {error}"))?;
    let props = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let title = tag
        .and_then(|tag| tag.title().map(|value| value.into_owned()))
        .unwrap_or_else(|| {
            path.file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("Unknown")
                .to_owned()
        });
    let cover = tag
        .and_then(|tag| waveflow_core::scanner::extract_cover(tag, artwork_dir))
        .or_else(|| waveflow_core::scanner::extract_folder_cover(path, artwork_dir));
    let artwork = cover.map(|cover| artwork_input(artwork_dir, cover));
    let lyrics = extract_lyrics(path, tag);
    let lyrics_hash = lyrics_hash(&lyrics);
    Ok(CatalogTrackInput {
        relative_path,
        file_size: size,
        modified_at,
        quick_hash,
        full_hash,
        title,
        artist: tag.and_then(|t| t.artist().map(|v| v.into_owned())),
        album: tag.and_then(|t| t.album().map(|v| v.into_owned())),
        album_artist: tag.and_then(waveflow_core::scanner::extract_album_artist),
        is_compilation: tag.is_some_and(waveflow_core::scanner::extract_compilation_flag),
        genre: tag.and_then(|t| t.genre().map(|v| v.into_owned())),
        year: tag.and_then(|t| t.date().map(|v| v.year as i64)),
        track_number: tag.and_then(|t| t.track().map(i64::from)),
        disc_number: tag.and_then(|t| t.disk().map(i64::from)),
        duration_ms: props.duration().as_millis() as i64,
        bitrate: props.audio_bitrate().map(i64::from),
        sample_rate: props.sample_rate().map(i64::from),
        channels: props.channels().map(i64::from),
        bit_depth: props.bit_depth().map(i64::from).filter(|v| *v > 0),
        codec: waveflow_core::scanner::file_type_label(tagged.file_type()),
        musical_key: tag.and_then(waveflow_core::scanner::extract_musical_key),
        tag_rating: tag
            .and_then(waveflow_core::scanner::extract_rating)
            .map(i64::from),
        artwork,
        lyrics_hash,
        lyrics,
    })
}

fn artwork_input(dir: &Path, cover: waveflow_core::scanner::ExtractedCover) -> ArtworkInput {
    let bytes = fs::metadata(dir.join(format!("{}.{}", cover.hash, cover.format)))
        .map(|v| v.len() as i64)
        .unwrap_or(0);
    ArtworkInput {
        hash: cover.hash,
        format: cover.format,
        source: cover.source.into(),
        byte_size: bytes,
    }
}

fn extract_dsd(
    path: &Path,
    artwork_dir: &Path,
    relative_path: String,
    size: i64,
    modified_at: i64,
    quick_hash: String,
    full_hash: String,
) -> Result<CatalogTrackInput, String> {
    use waveflow_core::audio_format::dsd::{
        metadata::read_metadata,
        parser::{parse_dff, parse_dsf, DsdContainer},
    };
    let ext = path
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let layout = if ext == "dsf" {
        parse_dsf(&mut file).map_err(|e| e.to_string())?
    } else {
        parse_dff(&mut file).map_err(|e| e.to_string())?
    };
    let meta = read_metadata(&mut file, layout.container).unwrap_or_default();
    let codec = layout
        .dsd_rate_multiple()
        .map(|v| format!("DSD{v}"))
        .or_else(|| {
            Some(
                match layout.container {
                    DsdContainer::Dsf => "DSF",
                    DsdContainer::Dff => "DFF",
                }
                .into(),
            )
        });
    let artwork = waveflow_core::scanner::extract_folder_cover(path, artwork_dir)
        .map(|v| artwork_input(artwork_dir, v));
    let lyrics = extract_lyrics(path, None);
    let lyrics_hash = lyrics_hash(&lyrics);
    Ok(CatalogTrackInput {
        relative_path,
        file_size: size,
        modified_at,
        quick_hash,
        full_hash,
        title: meta.title.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|v| v.to_str())
                .unwrap_or("Unknown")
                .into()
        }),
        artist: meta.artist,
        album: meta.album,
        album_artist: None,
        is_compilation: false,
        genre: meta.genre,
        year: meta.year,
        track_number: meta.track_number,
        disc_number: meta.disc_number,
        duration_ms: layout.duration_ms() as i64,
        bitrate: None,
        sample_rate: Some(layout.sample_rate_hz as i64),
        channels: Some(layout.channels.count() as i64),
        bit_depth: Some(1),
        codec,
        musical_key: None,
        tag_rating: None,
        artwork,
        lyrics_hash,
        lyrics,
    })
}

fn extract_lyrics(path: &Path, tag: Option<&lofty::tag::Tag>) -> Vec<LyricsInput> {
    let mut found = Vec::new();
    if let Some(tag) = tag {
        for key in [ItemKey::UnsyncLyrics, ItemKey::Lyrics] {
            if let Some(content) = tag.get_string(key) {
                push_lyrics(&mut found, "embedded", content);
            }
        }
    }
    for (extension, source) in [("lrc", "sidecar_lrc"), ("txt", "sidecar_text")] {
        let sidecar = path.with_extension(extension);
        if let Some(content) = read_sidecar(&sidecar) {
            push_lyrics(&mut found, source, &content);
        }
    }
    found
}

fn push_lyrics(found: &mut Vec<LyricsInput>, source: &'static str, content: &str) {
    let content = lyrics::normalize_content(content);
    if content.trim().is_empty()
        || found
            .iter()
            .any(|existing| existing.source == source && existing.content == content)
    {
        return;
    }
    let synced = lyrics::has_lrc_timestamps(&content);
    found.push(LyricsInput {
        source,
        lang: "xxx".into(),
        synced,
        content,
    });
}

fn read_sidecar(path: &Path) -> Option<String> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the reparse point itself so descriptor metadata can reject a
        // symlink instead of validating one path and reading its later target.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_LYRICS_BYTES
    {
        return None;
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_LYRICS_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_LYRICS_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn lyrics_hash(entries: &[LyricsInput]) -> String {
    let mut hasher = blake3::Hasher::new();
    for entry in entries {
        hasher.update(entry.source.as_bytes());
        hasher.update(&[0]);
        hasher.update(entry.lang.as_bytes());
        hasher.update(&[u8::from(entry.synced)]);
        hasher.update(entry.content.as_bytes());
        hasher.update(&[0xff]);
    }
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::{read_sidecar, MAX_LYRICS_BYTES};

    #[test]
    fn sidecar_reader_rejects_invalid_files_and_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let valid = temp.path().join("valid.lrc");
        std::fs::write(&valid, "[00:01]safe").unwrap();
        assert_eq!(read_sidecar(&valid).as_deref(), Some("[00:01]safe"));

        let oversized = temp.path().join("oversized.lrc");
        std::fs::write(&oversized, vec![b'x'; MAX_LYRICS_BYTES as usize + 1]).unwrap();
        assert!(read_sidecar(&oversized).is_none());

        let invalid_utf8 = temp.path().join("invalid.lrc");
        std::fs::write(&invalid_utf8, [0xff]).unwrap();
        assert!(read_sidecar(&invalid_utf8).is_none());
        assert!(read_sidecar(temp.path()).is_none());

        let link = temp.path().join("linked.lrc");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&valid, &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&valid, &link).is_ok();
        if linked {
            assert!(read_sidecar(&link).is_none());
        }
    }
}
