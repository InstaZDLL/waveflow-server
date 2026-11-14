//! A track's music video, found by the desktop's rule over what the walk saw
//! (RFC-029 decisions 2 and 3), and the pass that records it.
//!
//! The rule is `waveflow_core::scanner::clips::find_clip`: a video named like
//! the track, case ignored, beside it, else in a `Clips` folder beside it,
//! else the track's own file when it holds a moving picture; several
//! candidates in `VIDEO_EXTENSIONS` order, each confirmed by its content, and
//! no neighbour offered to a track that is itself inside a `Clips` folder.
//! `find_clip` lists a folder for every track it is asked about, so the scan
//! matches names in memory over what the walk already listed and reads the
//! disk only to confirm a candidate. This copy is held to `find_clip` by a
//! test over a fixture tree, which is what fails first if the core's rule
//! changes.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
};

use uuid::Uuid;
use waveflow_core::scanner::clips::{has_video_track, is_clips_dir_name, VIDEO_EXTENSIONS};

use super::discovery::Walked;
use super::extract::stamp;
use super::ScanManager;
use crate::storage::catalog::{TrackClip, TrackClipFacts};

#[cfg(test)]
mod tests;

/// The containers whose content `has_video_track` can confirm. The core
/// keeps its own list private; an `.mp4` or a `.mov` may hold audio alone.
const ISO_BMFF_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov"];

/// What every WebM and Matroska file starts with. That is all the core checks
/// for these two, so a WebM holding audio alone passes, here as on the
/// desktop (RFC-029 decision 2).
const EBML_MAGIC: [u8; 4] = [0x1a, 0x45, 0xdf, 0xa3];

/// A catalogue name split into its folder (`""` at the root) and file name.
fn split(name: &str) -> (&str, &str) {
    name.rsplit_once('/').unwrap_or(("", name))
}

fn extension(file: &str) -> Option<String> {
    Path::new(file)
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
}

/// The name a track and its clip share, as the core compares it.
fn stem(file: &str) -> Option<String> {
    Path::new(file)
        .file_stem()
        .and_then(OsStr::to_str)
        .map(str::to_lowercase)
}

fn rank(name: &str) -> usize {
    let extension = extension(split(name).1).unwrap_or_default();
    VIDEO_EXTENSIONS
        .iter()
        .position(|known| *known == extension)
        .unwrap_or(VIDEO_EXTENSIONS.len())
}

/// One video a track may have, in the order the rule asks them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Candidate<'a> {
    pub(super) path: &'a Path,
    pub(super) name: &'a str,
    pub(super) origin: &'static str,
}

/// The videos one walk saw, indexed by folder and name.
pub(super) struct Videos<'a> {
    list: &'a [(PathBuf, String)],
    /// `(folder, stem)` → indices into `list`, in `VIDEO_EXTENSIONS` order.
    named: HashMap<(&'a str, String), Vec<usize>>,
    /// A folder → its `Clips` folder, the first by name when the disk holds
    /// two spellings of it.
    clips_dirs: HashMap<&'a str, &'a str>,
}

impl<'a> Videos<'a> {
    pub(super) fn new(list: &'a [(PathBuf, String)]) -> Self {
        let mut named: HashMap<(&str, String), Vec<usize>> = HashMap::new();
        let mut clips_dirs: HashMap<&str, &str> = HashMap::new();
        for (index, (_, name)) in list.iter().enumerate() {
            let (folder, file) = split(name);
            if let Some(stem) = stem(file) {
                named.entry((folder, stem)).or_default().push(index);
            }
            let (parent, last) = split(folder);
            if !folder.is_empty() && is_clips_dir_name(OsStr::new(last)) {
                let known = clips_dirs.entry(parent).or_insert(folder);
                *known = (*known).min(folder);
            }
        }
        for indices in named.values_mut() {
            indices.sort_by(|&left, &right| {
                (rank(&list[left].1), &list[left].1).cmp(&(rank(&list[right].1), &list[right].1))
            });
        }
        Self {
            list,
            named,
            clips_dirs,
        }
    }

    /// Every video the rule would try for `track`, in the order it tries them:
    /// beside it, then in its `Clips` folder, then the track's own file. `path`
    /// is the track's file, read only for the last.
    pub(super) fn candidates<'t>(&'t self, track: &'t str, path: &'t Path) -> Vec<Candidate<'t>> {
        let (folder, file) = split(track);
        let Some(stem) = stem(file) else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let in_clips_dir = !folder.is_empty() && is_clips_dir_name(OsStr::new(split(folder).1));
        if !in_clips_dir {
            let mut named = |folder: &'t str, origin: &'static str| {
                for &index in self
                    .named
                    .get(&(folder, stem.clone()))
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                {
                    let (path, name) = &self.list[index];
                    if name != track {
                        found.push(Candidate { path, name, origin });
                    }
                }
            };
            named(folder, "sibling");
            if let Some(clips) = self.clips_dirs.get(folder) {
                named(clips, "clips_dir");
            }
        }
        if extension(file).is_some_and(|own| ISO_BMFF_EXTENSIONS.contains(&own.as_str())) {
            found.push(Candidate {
                path,
                name: track,
                origin: "itself",
            });
        }
        found
    }
}

/// Whether a file holds what its container promises: a moving picture for
/// the ISO-BMFF ones, the EBML header for the others. A file too short to
/// hold the header holds no video; a file that cannot be read is an error,
/// which is not the same answer.
fn holds_video(path: &Path, format: &str) -> std::io::Result<bool> {
    let mut file = std::fs::File::open(path)?;
    let answer = if ISO_BMFF_EXTENSIONS.contains(&format) {
        has_video_track(&mut file)
    } else {
        let mut head = [0u8; 4];
        file.read_exact(&mut head).map(|()| head == EBML_MAGIC)
    };
    match answer {
        // Truncated: what the file holds was read, and it is not a video.
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
        answer => answer,
    }
}

/// The link a candidate makes once its content is confirmed. A candidate equal
/// to the link already recorded, path, size and time alike, is taken as
/// confirmed without opening it, unless the scan reads everything.
///
/// A candidate the pass cannot read is passed over, as the core passes it
/// over, unless it is the clip already recorded: then the recorded link
/// stands, because a read that failed says nothing about what the file holds,
/// and a link dropped on an I/O error would come back at the next scan.
pub(super) fn confirm(
    candidate: Candidate<'_>,
    recorded: Option<&TrackClip>,
    full: bool,
) -> Option<TrackClip> {
    let format = extension(split(candidate.name).1)?;
    if !VIDEO_EXTENSIONS.contains(&format.as_str()) {
        return None;
    }
    let unreadable = |error: std::io::Error| {
        tracing::warn!(path = %candidate.name, %error, "a music video could not be read");
        recorded
            .filter(|recorded| recorded.relative_path == candidate.name)
            .cloned()
    };
    let metadata = match std::fs::metadata(candidate.path) {
        Ok(metadata) => metadata,
        Err(error) => return unreadable(error),
    };
    if !metadata.is_file() {
        return None;
    }
    // From the one `stat` above, so a second one cannot fail apart from it,
    // and stamped as a track is.
    let (byte_size, modified_at) = stamp(&metadata);
    let link = TrackClip {
        relative_path: candidate.name.to_owned(),
        format,
        origin: candidate.origin,
        byte_size,
        modified_at,
    };
    let unchanged = !full
        && recorded.is_some_and(|recorded| {
            recorded.relative_path == link.relative_path
                && recorded.byte_size == link.byte_size
                && recorded.modified_at == link.modified_at
        });
    if unchanged {
        return Some(link);
    }
    match holds_video(candidate.path, &link.format) {
        Ok(video) => video.then_some(link),
        Err(error) => unreadable(error),
    }
}

/// What one walk saw that the link pass needs.
pub(super) struct WalkedClips {
    /// The audio files the walk kept, as `Discovery::paths`.
    pub(super) tracks: Vec<(PathBuf, String)>,
    pub(super) videos: Vec<(PathBuf, String)>,
    pub(super) unreadable: Vec<String>,
    pub(super) incomplete: bool,
    pub(super) scope: Option<Vec<String>>,
    pub(super) full: bool,
}

/// The links that change: the link each walked track should have now, against
/// the one it has.
///
/// A link is removed only where the walk could see that its clip went (RFC-018
/// decision 3, as for a track): the clip file was walked and no longer
/// qualifies, or its place was walked and it was not there. A clip under a
/// folder the walk could not read, or a walk that could not say what it
/// missed, keeps the link. An excluded clip was not walked, so its link goes,
/// as an excluded track does.
pub(super) fn changes(
    walked: &WalkedClips,
    facts: &HashMap<String, TrackClipFacts>,
) -> Vec<(Uuid, Option<TrackClip>)> {
    let videos = Videos::new(&walked.videos);
    let seen: HashSet<String> = walked.videos.iter().map(|(_, name)| name.clone()).collect();
    let saw = Walked {
        discovered: &seen,
        unreadable: &walked.unreadable,
        incomplete: walked.incomplete,
        scope: walked.scope.as_deref(),
    };
    let mut changes = Vec::new();
    for (path, name) in &walked.tracks {
        // A track that failed to read, or that is not available, has no row
        // to link from.
        let Some(fact) = facts.get(name) else {
            continue;
        };
        let found = videos
            .candidates(name, path)
            .into_iter()
            .find_map(|candidate| confirm(candidate, fact.clip.as_ref(), walked.full));
        match (found, &fact.clip) {
            (Some(found), Some(recorded)) if found == *recorded => {}
            (Some(found), _) => changes.push((fact.track_id, Some(found))),
            (None, Some(recorded))
                if seen.contains(&recorded.relative_path)
                    || saw.saw_missing(&recorded.relative_path) =>
            {
                changes.push((fact.track_id, None));
            }
            (None, _) => {}
        }
    }
    changes
}

impl ScanManager {
    /// Records the clip of every track the walk saw (RFC-029 decision 3),
    /// after availability is settled so a track that went is not linked.
    pub(super) async fn link_clips(
        &self,
        library_id: Uuid,
        walked: WalkedClips,
    ) -> anyhow::Result<()> {
        let facts = self.db.track_clip_facts(library_id).await?;
        let changes = tokio::task::spawn_blocking(move || changes(&walked, &facts)).await?;
        if !changes.is_empty() {
            tracing::debug!(%library_id, count = changes.len(), "music video links changed");
            self.db.apply_track_clips(&changes).await?;
        }
        Ok(())
    }
}
