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
        // Read here rather than taken as an argument: a scan that died halfway
        // left the request standing, and the run picking up after it has to
        // honour a request it never saw made.
        let full = self.db.full_scan_requested(library.id).await?;
        if full {
            tracing::info!(library_id = %library.id, "scanning every file, skipping nothing");
        }
        self.db
            .start_scan_job(scan_id, paths.len() as i64, full)
            .await?;

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
                            if !full
                                && existing.file_size == input.file_size
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
        // After availability is settled, not before: the album and artist
        // identifiers are a majority vote over the tracks that are still there,
        // so a file that vanished must stop voting first.
        self.db.consolidate_catalog_derivations(library.id).await?;
        self.db.consolidate_sort_names(library.id).await?;
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
    let raw = raw_credits(tag);
    let extended = extended_tags(tag);
    Ok(CatalogTrackInput {
        relative_path,
        file_size: size,
        modified_at,
        quick_hash,
        full_hash,
        title,
        artist: tag.and_then(|t| t.artist().map(|v| v.into_owned())),
        artists: raw.artists,
        album_artists: raw.album_artists,
        roles: raw.roles,
        performer_pairs: raw.performer_pairs,
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
        musicbrainz_recording_id: extended.musicbrainz_recording_id,
        musicbrainz_release_id: extended.musicbrainz_release_id,
        musicbrainz_artist_id: extended.musicbrainz_artist_id,
        replay_gain_track_gain: extended.replay_gain_track_gain,
        replay_gain_track_peak: extended.replay_gain_track_peak,
        replay_gain_album_gain: extended.replay_gain_album_gain,
        replay_gain_album_peak: extended.replay_gain_album_peak,
        bpm: extended.bpm,
        sort_title: extended.sort_title,
        sort_album: extended.sort_album,
        sort_album_artist: extended.sort_album_artist,
        sort_artist: extended.sort_artist,
        comment: extended.comment,
        isrc: extended.isrc,
        moods: extended.moods,
        explicit_status: extended.explicit_status,
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
    let extended = extended_tags(None);
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
        artist: meta.artist.clone(),
        // DSD has no tag reader beyond its own metadata block, so the only
        // credit it can name is the artist tag. Every other role list stays
        // empty rather than being invented.
        artists: Vec::new(),
        album_artists: Vec::new(),
        roles: Vec::new(),
        performer_pairs: Vec::new(),
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
        musicbrainz_recording_id: extended.musicbrainz_recording_id,
        musicbrainz_release_id: extended.musicbrainz_release_id,
        musicbrainz_artist_id: extended.musicbrainz_artist_id,
        replay_gain_track_gain: extended.replay_gain_track_gain,
        replay_gain_track_peak: extended.replay_gain_track_peak,
        replay_gain_album_gain: extended.replay_gain_album_gain,
        replay_gain_album_peak: extended.replay_gain_album_peak,
        bpm: extended.bpm,
        sort_title: extended.sort_title,
        sort_album: extended.sort_album,
        sort_album_artist: extended.sort_album_artist,
        sort_artist: extended.sort_artist,
        comment: extended.comment,
        isrc: extended.isrc,
        moods: extended.moods,
        explicit_status: extended.explicit_status,
        artwork,
        lyrics_hash,
        lyrics,
    })
}

/// The OpenSubsonic media tags the scanner reads beyond the Subsonic 1.16 set.
///
/// Grouped rather than inlined because both entry points build a
/// [`CatalogTrackInput`], and the DSD path has no tag reader at all: it has to
/// produce the same shape with every field empty rather than a different one.
#[derive(Debug, Default)]
struct ExtendedTags {
    musicbrainz_recording_id: Option<String>,
    musicbrainz_release_id: Option<String>,
    musicbrainz_artist_id: Option<String>,
    replay_gain_track_gain: Option<f64>,
    replay_gain_track_peak: Option<f64>,
    replay_gain_album_gain: Option<f64>,
    replay_gain_album_peak: Option<f64>,
    bpm: Option<i64>,
    sort_title: Option<String>,
    sort_album: Option<String>,
    sort_album_artist: Option<String>,
    sort_artist: Option<String>,
    comment: Option<String>,
    isrc: Option<String>,
    moods: Option<String>,
    explicit_status: Option<String>,
}

/// Reads every credit a file names, in tag order.
///
/// The role vocabulary and the tag each role is read from follow the
/// reference. `WRITER` folds onto `COMPOSER` as it does there: the two are one
/// credit on the overwhelming majority of files that carry either, and telling
/// them apart would invent a distinction the taggers do not make.
///
/// `PERFORMER` is the one role whose credit carries a second value. ID3 stores
/// it as a musician-credits frame lofty exposes only through its own tag type,
/// so what reaches here is the generic spelling — `Name (instrument)` — which
/// [`crate::tags::split_performer`] takes apart.
fn raw_credits(tag: Option<&lofty::tag::Tag>) -> crate::tags::RawCredits {
    use crate::tags::Role;
    let Some(tag) = tag else {
        return crate::tags::RawCredits::default();
    };
    let values = |key: ItemKey| -> Vec<String> {
        tag.get_strings(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect()
    };
    let single = |key: ItemKey| -> Option<String> {
        tag.get_string(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };

    let mut roles: Vec<(Role, Vec<String>)> = Vec::new();
    let mut composers = values(ItemKey::Composer);
    composers.extend(values(ItemKey::Writer));
    if !composers.is_empty() {
        roles.push((Role::Composer, composers));
    }
    for (role, key) in [
        (Role::Lyricist, ItemKey::Lyricist),
        (Role::Conductor, ItemKey::Conductor),
        (Role::Arranger, ItemKey::Arranger),
        (Role::Producer, ItemKey::Producer),
        (Role::Director, ItemKey::Director),
        (Role::Engineer, ItemKey::Engineer),
        (Role::Mixer, ItemKey::MixEngineer),
        (Role::Remixer, ItemKey::Remixer),
        (Role::DjMixer, ItemKey::MixDj),
    ] {
        let found = values(key);
        if !found.is_empty() {
            roles.push((role, found));
        }
    }

    let performer_pairs = values(ItemKey::Performer)
        .into_iter()
        .map(|value| crate::tags::split_performer(&value))
        .collect();

    crate::tags::RawCredits {
        artist: values(ItemKey::TrackArtist),
        artists: values(ItemKey::TrackArtists),
        album_artist: values(ItemKey::AlbumArtist),
        album_artists: values(ItemKey::AlbumArtists),
        sort_artist: single(ItemKey::TrackArtistSortOrder),
        sort_album_artist: single(ItemKey::AlbumArtistSortOrder),
        is_compilation: waveflow_core::scanner::extract_compilation_flag(tag),
        roles,
        performer_pairs,
    }
}
fn extended_tags(tag: Option<&lofty::tag::Tag>) -> ExtendedTags {
    let Some(tag) = tag else {
        return ExtendedTags::default();
    };
    let text = |key: ItemKey| {
        tag.get_string(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    // ReplayGain tags carry their unit: `-7.32 dB`. Reading only the first
    // token keeps the suffix from turning a valid measurement into none. A
    // non-finite value is discarded rather than stored: it would travel all
    // the way to a player as a volume adjustment.
    let gain = |key: ItemKey| {
        tag.get_string(key)
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
    };
    ExtendedTags {
        musicbrainz_recording_id: text(ItemKey::MusicBrainzRecordingId),
        musicbrainz_release_id: text(ItemKey::MusicBrainzReleaseId),
        musicbrainz_artist_id: text(ItemKey::MusicBrainzArtistId),
        replay_gain_track_gain: gain(ItemKey::ReplayGainTrackGain),
        replay_gain_track_peak: gain(ItemKey::ReplayGainTrackPeak),
        replay_gain_album_gain: gain(ItemKey::ReplayGainAlbumGain),
        replay_gain_album_peak: gain(ItemKey::ReplayGainAlbumPeak),
        // Both spellings occur in the wild, and a fractional value is written
        // often enough that parsing the whole string would drop it entirely.
        bpm: text(ItemKey::IntegerBpm)
            .or_else(|| text(ItemKey::Bpm))
            .and_then(|value| {
                value
                    .split(['.', ','])
                    .next()
                    .and_then(|whole| whole.parse::<i64>().ok())
            })
            .filter(|value| *value > 0),
        sort_title: text(ItemKey::TrackTitleSortOrder),
        sort_album: text(ItemKey::AlbumTitleSortOrder),
        sort_album_artist: text(ItemKey::AlbumArtistSortOrder),
        sort_artist: text(ItemKey::TrackArtistSortOrder),
        comment: text(ItemKey::Comment),
        isrc: text(ItemKey::Isrc),
        moods: text(ItemKey::Mood),
        // Normalised to the two words the specification defines. The tag
        // spells the same thing differently per format — MP4 writes the
        // iTunes rating as 1 or 2, others write words — and a client only
        // ever compares against `explicit` and `clean`. A tag that says
        // "no advisory" is not a claim that the work is clean, so it maps to
        // no value rather than to the second word.
        explicit_status: text(ItemKey::ParentalAdvisory).and_then(|value| {
            match value.to_ascii_lowercase().as_str() {
                "1" | "true" | "explicit" => Some("explicit".to_owned()),
                "2" | "clean" | "edited" => Some("clean".to_owned()),
                _ => None,
            }
        }),
    }
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
    use super::{extended_tags, read_sidecar, MAX_LYRICS_BYTES};
    use lofty::{
        prelude::ItemKey,
        tag::{Tag, TagType},
    };

    #[test]
    fn extended_tags_survive_the_shapes_real_files_use() {
        // MP4 is the tag type whose lofty mapping covers every key this reads.
        // The others cover a subset each — ID3v2 carries the recording MBID in
        // a UFID frame rather than a text one, and Vorbis comments spell BPM
        // only as `Bpm` — which is what the fallbacks in `extended_tags` are
        // for. This test is about the parsing, so it uses the tag type that
        // exercises all of it at once.
        let mut tag = Tag::new(TagType::Mp4Ilst);
        // ReplayGain is written with its unit, and a leading + is legal.
        assert!(tag.insert_text(ItemKey::ReplayGainTrackGain, "-7.32 dB".into()));
        assert!(tag.insert_text(ItemKey::ReplayGainAlbumGain, "+1.50 dB".into()));
        assert!(tag.insert_text(ItemKey::ReplayGainTrackPeak, "0.988525".into()));
        // BPM is often fractional even in the integer key.
        assert!(tag.insert_text(ItemKey::IntegerBpm, "128.5".into()));
        assert!(tag.insert_text(ItemKey::MusicBrainzRecordingId, "  rec-1  ".into()));
        assert!(tag.insert_text(ItemKey::Comment, "   ".into()));
        assert!(tag.insert_text(ItemKey::Isrc, "FRZ039800212".into()));
        assert!(tag.insert_text(ItemKey::Mood, "Melancholic; Warm".into()));
        assert!(tag.insert_text(ItemKey::ParentalAdvisory, "2".into()));
        // The three sort keys, which the catalogue derives album and artist
        // sort names from. A joined credit carries a joined sort tag, and the
        // pairing happens where the names are split — what this pins is that
        // all three arrive at all, under the tag type that spells them.
        assert!(tag.insert_text(ItemKey::AlbumTitleSortOrder, "Night Sessions, The".into()));
        assert!(tag.insert_text(ItemKey::AlbumArtistSortOrder, "Nocturnes, The".into()));
        assert!(tag.insert_text(
            ItemKey::TrackArtistSortOrder,
            "Nocturnes, The; Reed, Ada".into()
        ));
        let extended = extended_tags(Some(&tag));
        assert_eq!(extended.replay_gain_track_gain, Some(-7.32));
        assert_eq!(extended.replay_gain_album_gain, Some(1.5));
        assert_eq!(extended.replay_gain_track_peak, Some(0.988_525));
        assert_eq!(extended.replay_gain_album_peak, None);
        assert_eq!(extended.bpm, Some(128));
        // Trimmed, and a tag holding only whitespace is no value at all
        // rather than an empty one the presence rule would then report as
        // meaningful.
        assert_eq!(extended.musicbrainz_recording_id.as_deref(), Some("rec-1"));
        assert_eq!(extended.comment, None);
        assert_eq!(extended.isrc.as_deref(), Some("FRZ039800212"));
        assert_eq!(extended.moods.as_deref(), Some("Melancholic; Warm"));
        // The per-format spelling is normalised to the two words the
        // specification defines; a client compares against nothing else.
        assert_eq!(extended.explicit_status.as_deref(), Some("clean"));
        assert_eq!(extended.sort_album.as_deref(), Some("Night Sessions, The"));
        assert_eq!(
            extended.sort_album_artist.as_deref(),
            Some("Nocturnes, The")
        );
        assert_eq!(
            extended.sort_artist.as_deref(),
            Some("Nocturnes, The; Reed, Ada")
        );

        // Unparseable measurements are dropped rather than stored: a NaN gain
        // would reach a player as a volume adjustment.
        let mut broken = Tag::new(TagType::Mp4Ilst);
        assert!(broken.insert_text(ItemKey::ReplayGainTrackGain, "loud".into()));
        assert!(broken.insert_text(ItemKey::ReplayGainAlbumPeak, "NaN".into()));
        assert!(broken.insert_text(ItemKey::IntegerBpm, "0".into()));
        let broken = extended_tags(Some(&broken));
        assert_eq!(broken.replay_gain_track_gain, None);
        assert_eq!(broken.replay_gain_album_peak, None);
        assert_eq!(broken.bpm, None);
        // A tag saying "no advisory" is not a claim that the work is clean.
        let mut neutral = Tag::new(TagType::Mp4Ilst);
        assert!(neutral.insert_text(ItemKey::ParentalAdvisory, "0".into()));
        assert_eq!(extended_tags(Some(&neutral)).explicit_status, None);

        // The DSD path has no tag reader and must produce the same shape.
        let absent = extended_tags(None);
        assert_eq!(absent.musicbrainz_recording_id, None);
        assert_eq!(absent.bpm, None);
    }

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
