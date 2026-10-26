//! Reading one file: the pre-check that skips it, its tags, credits, cover,
//! DSD metadata, and the lyrics embedded in it or kept beside it.

use std::{fs, io::Read, path::Path, time::UNIX_EPOCH};

use lofty::{
    file::TaggedFileExt,
    prelude::{Accessor, AudioFile, ItemKey},
};
use uuid::Uuid;

use crate::{
    library::lyrics::{self, LyricsInput},
    storage::catalog::{ArtworkInput, CatalogTrackInput},
};

#[cfg(test)]
mod tests;

const MAX_LYRICS_BYTES: u64 = 1024 * 1024;

/// What a scan does with one discovered file.
pub(super) enum Scanned {
    /// Unchanged since the last scan: skipped without being opened.
    Skipped(Uuid),
    /// Read, with the sidecar signature to record once it is applied.
    /// `None` when a sidecar could not be opened: nothing is recorded, so the
    /// file is read again until it can be.
    Read(Box<Result<CatalogTrackInput, String>>, Option<String>),
}

/// The pre-check, then the read (RFC-018 decision 1).
///
/// A file whose size, modification time and sidecar signature match what the
/// catalogue holds, on an available track that already has its relocation
/// hint, is skipped from a `stat` alone. Anything else, and every file on a
/// full scan, is read as before.
pub(super) fn scan_file(
    path: &Path,
    artwork_dir: &Path,
    known: Option<crate::storage::catalog::SkipFacts>,
    full: bool,
) -> Scanned {
    let signature = sidecar_signature(path);
    if let Some(known) = known.filter(|_| !full) {
        let unchanged = file_stamp(path).is_some_and(|(size, modified_at)| {
            size == known.file_size && modified_at == known.modified_at
        }) && known.available
            && known.has_pid
            && signature.is_some()
            && known.sidecar_signature == signature;
        if unchanged {
            return Scanned::Skipped(known.id);
        }
    }
    Scanned::Read(Box::new(extract_file(path, artwork_dir)), signature)
}

/// A file's size and modification time, as `extract_file` records them.
fn file_stamp(path: &Path) -> Option<(i64, i64)> {
    let metadata = fs::metadata(path).ok()?;
    let modified_at = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as i64)
        .unwrap_or(0);
    Some((metadata.len() as i64, modified_at))
}

/// The two lyrics sidecars a scan reads beside an audio file, by size and
/// modification time, absent ones as `-`:
/// `lrc:<size>:<mtime_ms>|txt:<size>:<mtime_ms>`. Adding, editing or deleting
/// either changes it; it is never empty.
///
/// `None` when a sidecar exists but cannot be opened. The read then goes ahead
/// without its lyrics, and recording the signature would skip the file for as
/// long as the sidecar's size and time stay put — a permission fixed later
/// moves neither. With no signature the file is read on every scan until the
/// sidecar can be.
///
/// Read without following links, and a sidecar that is not a regular file — a
/// link, a FIFO, a folder — counts as absent, since `read_sidecar` would not
/// read it either. Only a regular file is opened, and never in a way that can
/// block.
fn sidecar_signature(path: &Path) -> Option<String> {
    let mut parts = Vec::with_capacity(2);
    for extension in ["lrc", "txt"] {
        let sidecar = path.with_extension(extension);
        match fs::symlink_metadata(&sidecar) {
            Ok(metadata) if metadata.is_file() => {
                sidecar_options().open(&sidecar).ok()?;
                let modified_at = metadata
                    .modified()
                    .ok()
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_millis() as i64)
                    .unwrap_or(0);
                parts.push(format!("{extension}:{}:{modified_at}", metadata.len()));
            }
            _ => parts.push(format!("{extension}:-")),
        }
    }
    Some(parts.join("|"))
}

/// How a sidecar is opened, for reading it and for checking that it can be:
/// never through a link, and without blocking on a special file swapped in
/// after it was looked at.
pub(super) fn sidecar_options() -> fs::OpenOptions {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Open the reparse point itself so descriptor metadata can reject a
        // symlink instead of validating one path and reading its later target.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options
}

pub(crate) fn extract_file(path: &Path, artwork_dir: &Path) -> Result<CatalogTrackInput, String> {
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
        original_release_date: extended.original_release_date,
        release_date: extended.release_date,
        release_types: extended.release_types,
        record_labels: extended.record_labels,
        disc_subtitle: extended.disc_subtitle,
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
        source: cover.source.as_str().to_owned(),
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
        original_release_date: extended.original_release_date,
        release_date: extended.release_date,
        release_types: extended.release_types,
        record_labels: extended.record_labels,
        disc_subtitle: extended.disc_subtitle,
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
    original_release_date: Option<String>,
    release_date: Option<String>,
    release_types: Option<String>,
    record_labels: Option<String>,
    disc_subtitle: Option<String>,
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
/// [`crate::library::tags::split_performer`] takes apart.
fn raw_credits(tag: Option<&lofty::tag::Tag>) -> crate::library::tags::RawCredits {
    use crate::library::tags::Role;
    let Some(tag) = tag else {
        return crate::library::tags::RawCredits::default();
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
        .map(|value| crate::library::tags::split_performer(&value))
        .collect();

    crate::library::tags::RawCredits {
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
    // A tag written as several items rather than one `;`-joined string. Joined
    // here so the column holds one spelling, and split again on the way out by
    // the same helper that splits `moods`.
    let joined = |key: ItemKey| {
        let values = tag
            .get_strings(key)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| values.join("; "))
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
        original_release_date: text(ItemKey::OriginalReleaseDate),
        release_date: text(ItemKey::ReleaseDate).or_else(|| text(ItemKey::RecordingDate)),
        release_types: joined(ItemKey::MusicBrainzReleaseType),
        // `LABEL` is the tag Picard writes and the one the reference reads;
        // `PUBLISHER` is the older spelling the same value arrives under on
        // files tagged by anything else.
        record_labels: joined(ItemKey::Label).or_else(|| joined(ItemKey::Publisher)),
        disc_subtitle: text(ItemKey::SetSubtitle),
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
    let file = sidecar_options().open(path).ok()?;
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
    // Not `String::from_utf8`: lyrics written by a tool on a Chinese, Japanese
    // or Korean Windows are in that system's code page, and a strict UTF-8
    // read dropped them whole. The core decoder is the one the desktop reads
    // the same files with, so both answer the same text for them.
    Some(waveflow_core::text_encoding::decode(&bytes).into_owned())
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
