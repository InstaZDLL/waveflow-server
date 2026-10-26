//! Walking a library: ignore files, built-in exclusions, links, scopes, and
//! what a walk can say it saw missing.

use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use uuid::Uuid;
use walkdir::WalkDir;

use super::extract::sidecar_options;

#[cfg(test)]
mod tests;

/// The ignore file a folder may hold (RFC-021 decision 1).
pub const IGNORE_FILE: &str = ".waveflowignore";

/// Beyond this an ignore file is refused rather than read: a list of
/// exclusions has no business being larger, and the walk reads one per folder.
const MAX_IGNORE_BYTES: u64 = 64 * 1024;

/// Names the walk never enters, whatever an ignore file says (RFC-021
/// decision 1), beside every name beginning with a dot. Compared without
/// regard to case: `$RECYCLE.BIN` is spelled several ways by the systems that
/// write it.
const BUILT_IN_EXCLUSIONS: [&str; 6] = [
    "@eaDir",
    "#recycle",
    "#snapshot",
    "$RECYCLE.BIN",
    "System Volume Information",
    "lost+found",
];

/// How many skipped links one scan names in the log, one line each, before it
/// only counts the rest.
const LOGGED_LINKS: usize = 20;

/// Whether an entry below the root is left out by name. The root and the
/// folders above it are never asked: a library may well live under a dot
/// folder, and a temporary directory always does.
fn built_in_exclusion(name: &std::ffi::OsStr, depth: usize, is_dir: bool) -> bool {
    let name = name.to_string_lossy();
    if name.starts_with('.') {
        // Received files land here (RFC-008), and the scan is what keeps them
        // in the catalogue after the upload that applied them.
        return !(depth == 1 && is_dir && name == crate::services::uploads::MANAGED_DIR);
    }
    BUILT_IN_EXCLUSIONS
        .iter()
        .any(|excluded| excluded.eq_ignore_ascii_case(&name))
}

/// What one folder's ignore file says.
enum Rules {
    /// No file, or a root file with nothing in it.
    None,
    Matcher(ignore::gitignore::Gitignore),
    /// An empty file below the root: the folder and everything under it.
    ExcludeAll,
    /// A file the walk cannot use, and why. Its folder counts as unreadable.
    Unusable(&'static str),
}

/// Reads `dir`'s ignore file, if it has one. Never through a link, and never
/// in a way that can block, as the lyrics sidecars are read.
fn load_rules(dir: &Path, at_root: bool) -> Rules {
    let file = dir.join(IGNORE_FILE);
    let metadata = match fs::symlink_metadata(&file) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Rules::None,
        Err(_) => return Rules::Unusable("its .waveflowignore could not be read"),
    };
    if !metadata.is_file() {
        return Rules::Unusable("its .waveflowignore is not a regular file");
    }
    if metadata.len() > MAX_IGNORE_BYTES {
        return Rules::Unusable("its .waveflowignore is larger than 64 KiB");
    }
    let Some(text) = sidecar_options().open(&file).ok().and_then(|opened| {
        let mut bytes = Vec::new();
        opened
            .take(MAX_IGNORE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() as u64 <= MAX_IGNORE_BYTES)
            .then(|| String::from_utf8_lossy(&bytes).into_owned())
    }) else {
        return Rules::Unusable("its .waveflowignore could not be read");
    };
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        // At the root an empty file excludes nothing: one stray file must not
        // mark a whole library missing.
        return if at_root {
            Rules::None
        } else {
            Rules::ExcludeAll
        };
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(dir);
    for line in text.lines() {
        if builder.add_line(None, line).is_err() {
            return Rules::Unusable("its .waveflowignore holds a pattern that cannot be read");
        }
    }
    match builder.build() {
        Ok(matcher) => Rules::Matcher(matcher),
        Err(_) => Rules::Unusable("its .waveflowignore holds a pattern that cannot be read"),
    }
}

/// Why the walk left a symbolic link behind (RFC-021 decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkippedLink {
    /// The library does not follow links.
    NotFollowed,
    /// Its target lies outside the library root.
    Outside,
    /// Its target does not exist, or cannot be read.
    Broken,
    /// It leads back to one of its own ancestors.
    Loop,
}

impl SkippedLink {
    fn reason(self) -> &'static str {
        match self {
            Self::NotFollowed => "the library does not follow links",
            Self::Outside => "its target is outside the library",
            Self::Broken => "its target cannot be read",
            Self::Loop => "it leads back to a folder above it",
        }
    }
}

/// A folder the walk could not read, relative to the root, and why.
pub(super) struct Unreadable {
    pub(super) folder: String,
    pub(super) reason: &'static str,
}

/// What the walk found: the audio files, in catalogue path order, each with
/// the path it is read from and the path the catalogue knows it by, and the
/// folders it could not read, relative to the root.
pub(super) struct Discovery {
    pub(super) paths: Vec<(PathBuf, String)>,
    pub(super) unreadable: Vec<Unreadable>,
    /// An error named no path inside the root, so the walk cannot say which
    /// tracks it missed: none may be marked missing.
    pub(super) incomplete: bool,
    /// Entries left out by an exclusion, a folder counting once whatever it
    /// holds (RFC-021 decision 2). They count as not discovered.
    pub(super) excluded: usize,
    /// Symbolic links the walk did not follow (RFC-021 decision 3).
    pub(super) skipped_links: usize,
}

/// Where one walk may go: the library root, whether it follows links, and —
/// when it does — the folders outside the root a link may reach into and the
/// other libraries' roots no link may reach into (RFC-021, links outside the
/// root). Every path in it is canonical.
pub(super) struct Reach<'a> {
    pub(super) root: &'a Path,
    pub(super) follow_links: bool,
    pub(super) link_roots: &'a [PathBuf],
    pub(super) libraries: &'a [PathBuf],
}

impl Reach<'_> {
    /// Whether a followed link may lead to `target`, a canonical path.
    fn admits_target(&self, target: &Path) -> bool {
        target.starts_with(self.root)
            || crate::library::link_roots::admits(self.link_roots, self.libraries, target)
    }

    /// What the walk reached at `path`: the canonical path to read it from,
    /// and the path relative to the root the catalogue names it by.
    ///
    /// Without links, the path as walked. With links, a target inside the root
    /// is named by its canonical path, so one file is one track however many
    /// links name it (decision 3); a target in an allowed folder outside it has
    /// no path inside the root, so it is named by the path the walk reached it
    /// through — links included, which is why the stream check accepts a link
    /// that resolves into an allowed folder. Anything else names nothing.
    fn catalogue_name(&self, path: &Path) -> Option<(PathBuf, String)> {
        if !self.follow_links {
            return Some((path.to_path_buf(), relative_to(self.root, path)));
        }
        let canonical = fs::canonicalize(path).ok()?;
        if canonical.starts_with(self.root) {
            let name = relative_to(self.root, &canonical);
            return Some((canonical, name));
        }
        (crate::library::link_roots::admits(self.link_roots, self.libraries, &canonical)
            && path.starts_with(self.root))
        .then(|| (canonical, relative_to(self.root, path)))
    }
}

/// What the walk accumulates while `walkdir` decides, entry by entry, what to
/// enter. Shared between the filter and the loop, hence the cell.
#[derive(Default)]
struct WalkState {
    /// Each folder's ignore rules, by the path the walk sees it under.
    rules: std::collections::HashMap<PathBuf, ignore::gitignore::Gitignore>,
    unreadable: Vec<Unreadable>,
    excluded: usize,
    skipped_links: usize,
    /// A folder could not be read and could not be named as the catalogue
    /// names its tracks: no track may be marked missing.
    incomplete: bool,
}

impl WalkState {
    /// Records a folder the walk could not read, or could not use the rules
    /// of, under the name its tracks carry in the catalogue (RFC-018
    /// decision 3, RFC-021 decision 3).
    ///
    /// With links followed, a track is stored under its canonical path, and a
    /// folder reached through a link has another name: spared under the
    /// link's, its tracks would be marked missing anyway. So the folder is
    /// named as the catalogue names what it holds; one that names nothing
    /// leaves the walk incomplete.
    fn unreadable(&mut self, reach: &Reach<'_>, path: &Path, reason: &'static str) {
        match reach.catalogue_name(path).map(|(_, name)| name) {
            Some(folder) => {
                tracing::warn!(
                    path = %folder,
                    reason,
                    "a folder in the library could not be used; its tracks are left as they are"
                );
                self.unreadable.push(Unreadable { folder, reason });
            }
            None => {
                tracing::warn!(
                    path = %relative_to(reach.root, path),
                    reason,
                    "a folder reached through a link could not be used, nor resolved; nothing is marked missing this scan"
                );
                self.incomplete = true;
                self.unreadable.push(Unreadable {
                    folder: "<unknown>".to_owned(),
                    reason,
                });
            }
        }
    }

    /// Whether an ignore file of a folder above `path` leaves it out. The
    /// deepest file that has an opinion decides, as in git.
    fn ignored(&self, root: &Path, path: &Path, is_dir: bool) -> bool {
        let mut folder = path.parent();
        while let Some(current) = folder {
            if let Some(rules) = self.rules.get(current) {
                let matched = rules.matched(path, is_dir);
                if matched.is_ignore() {
                    return true;
                }
                if matched.is_whitelist() {
                    return false;
                }
            }
            if current == root {
                break;
            }
            folder = current.parent();
        }
        false
    }

    fn skip_link(&mut self, path: &str, why: SkippedLink) {
        self.skipped_links += 1;
        if self.skipped_links <= LOGGED_LINKS {
            tracing::info!(path = %path, reason = why.reason(), "a symbolic link in the library was skipped");
        }
    }
}

/// A path relative to the root, as the catalogue and the log spell it.
fn relative_to(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| "<outside the library>".to_owned())
}

/// The walk's decision on one entry below where it started: enter it, or
/// leave it. `base` is how deep below the root the walk started, so the rules
/// that depend on depth read the depth below the root.
fn admit(
    entry: &walkdir::DirEntry,
    reach: &Reach<'_>,
    base: usize,
    walk: &std::cell::RefCell<WalkState>,
) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let root = reach.root;
    let mut walk = walk.borrow_mut();
    let path = entry.path();
    // Followed, `file_type` is the target's; not followed, it is the link's.
    let is_dir = entry.file_type().is_dir();
    let is_link = if reach.follow_links {
        entry.path_is_symlink()
    } else {
        entry.file_type().is_symlink()
    };
    // The rules themselves: read with their folder, never counted as left out.
    if entry.file_name() == IGNORE_FILE {
        return false;
    }
    // By the name the walk sees, a link's own included (decision 1).
    if built_in_exclusion(entry.file_name(), base + entry.depth(), is_dir)
        || walk.ignored(root, path, is_dir)
    {
        walk.excluded += 1;
        tracing::debug!(path = %relative_to(root, path), "left out of the scan");
        return false;
    }
    if is_link {
        let skipped = if !reach.follow_links {
            Some(SkippedLink::NotFollowed)
        } else {
            match fs::canonicalize(path) {
                Ok(target) if reach.admits_target(&target) => None,
                Ok(_) => Some(SkippedLink::Outside),
                Err(_) => Some(SkippedLink::Broken),
            }
        };
        if let Some(why) = skipped {
            walk.skip_link(&relative_to(root, path), why);
            return false;
        }
    } else if is_dir && reach.follow_links && !reach.link_roots.is_empty() {
        // A folder that is no link itself, reached through one into an
        // allowed folder, may be another library's root: the link that led
        // here stops at it, and is counted as skipped there.
        match fs::canonicalize(path) {
            Ok(target) if reach.admits_target(&target) => {}
            _ => {
                walk.skip_link(&relative_to(root, path), SkippedLink::Outside);
                return false;
            }
        }
    }
    if is_dir {
        match load_rules(path, false) {
            Rules::None => {}
            Rules::Matcher(rules) => {
                walk.rules.insert(path.to_path_buf(), rules);
            }
            Rules::ExcludeAll => {
                walk.excluded += 1;
                tracing::debug!(path = %relative_to(root, path), "left out of the scan by an empty ignore file");
                return false;
            }
            Rules::Unusable(reason) => {
                walk.unreadable(reach, path, reason);
                return false;
            }
        }
    }
    true
}

/// How many folders a scoped scan walks at most. A watcher that saw changes in
/// more than this is told to walk the whole library instead: a list that long
/// says the changes were everywhere, and walking it folder by folder would
/// cost more than the walk it was meant to save.
pub(crate) const MAX_SCOPE_FOLDERS: usize = 64;

/// A scope as a scoped walk may use it: folders relative to the root, spelled
/// with `/`, none of them nested in another, at most [`MAX_SCOPE_FOLDERS`].
///
/// `None` — the whole library — for anything that is not plainly such a list:
/// an empty folder (the root itself), a `..`, an absolute path, too many
/// folders. Walking more than asked is always safe; walking less is not.
pub(crate) fn normalised_scope(scope: &[String]) -> Option<Vec<String>> {
    let mut folders = Vec::with_capacity(scope.len());
    for folder in scope {
        // A trailing separator is a spelling; a leading one is an absolute
        // path, which no folder of a library is.
        if folder.starts_with('/') {
            return None;
        }
        let folder = folder.trim_end_matches('/');
        if folder.is_empty()
            || folder
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == ".." || part.contains('\\'))
            || Path::new(folder).is_absolute()
        {
            return None;
        }
        folders.push(folder.to_owned());
    }
    folders.sort();
    folders.dedup();
    // A folder is dropped when any of its ancestors is listed too. Sorting
    // alone does not put a folder's descendants right after it — `-`, ` ` and
    // `(` sort before `/`, so "Rock - Live" falls between "Rock" and
    // "Rock/Album" — hence every ancestor is asked, not the last one kept.
    let listed: HashSet<&str> = folders.iter().map(String::as_str).collect();
    let kept: Vec<String> = folders
        .iter()
        .filter(|folder| {
            !folder
                .match_indices('/')
                .any(|(at, _)| listed.contains(&folder[..at]))
        })
        .cloned()
        .collect();
    (!kept.is_empty() && kept.len() <= MAX_SCOPE_FOLDERS).then_some(kept)
}

/// Where a scoped walk of `folder` starts, and how deep below the root that
/// is — or `None` when there is nothing to walk there.
///
/// Every folder on the way down is judged as the whole walk would have judged
/// it, so a scoped walk reads exactly what a full one would read below it: the
/// built-in exclusions and every ignore file above it, which are loaded as it
/// goes. A folder on the way that is excluded is left out, and what the
/// catalogue holds under it counts as not discovered (RFC-021 decision 2); one
/// that cannot be read, or whose rules cannot be used, is unreadable and spares
/// its tracks (RFC-018 decision 3). A folder that is gone was walked, and
/// nothing is there. A scoped walk reads no link, so a link on the way leads
/// nowhere, as it would in a walk that does not follow links.
fn enter_scope(
    reach: &Reach<'_>,
    walk: &std::cell::RefCell<WalkState>,
    folder: &str,
) -> Option<(PathBuf, usize)> {
    let root = reach.root;
    let mut current = root.to_path_buf();
    let parts: Vec<&str> = folder.split('/').collect();
    for (index, name) in parts.iter().enumerate() {
        let depth = index + 1;
        let next = current.join(name);
        let metadata = match fs::symlink_metadata(&next) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => {
                walk.borrow_mut()
                    .unreadable(reach, &next, "folder could not be read");
                return None;
            }
        };
        let mut state = walk.borrow_mut();
        if metadata.file_type().is_symlink() {
            state.skip_link(&relative_to(root, &next), SkippedLink::NotFollowed);
            return None;
        }
        if !metadata.is_dir() {
            // A file where a folder was named: the folder above it, walked
            // whole, holds it.
            return Some((current, depth - 1));
        }
        if built_in_exclusion(std::ffi::OsStr::new(name), depth, true)
            || state.ignored(root, &next, true)
        {
            state.excluded += 1;
            return None;
        }
        match load_rules(&next, false) {
            Rules::None => {}
            Rules::Matcher(rules) => {
                state.rules.insert(next.clone(), rules);
            }
            Rules::ExcludeAll => {
                state.excluded += 1;
                return None;
            }
            Rules::Unusable(reason) => {
                state.unreadable(reach, &next, reason);
                return None;
            }
        }
        current = next;
    }
    Some((current, parts.len()))
}

/// Walks the library (RFC-018 decision 3, RFC-021 decisions 1 to 3), or only
/// the folders of `scope` when the watcher could name them.
///
/// An entry the walk cannot read is logged by its path relative to the root,
/// never the absolute one, and the walk goes on; only the root itself failing
/// fails the scan, since then nothing can be told apart — and that includes an
/// ignore file at the root the walk cannot use.
///
/// With `follow_links`, a file reached through a link is returned under the
/// name [`Reach::catalogue_name`] gives it, and once only, however many links
/// name it.
pub(super) fn discover_audio(
    reach: &Reach<'_>,
    scope: Option<&[String]>,
) -> anyhow::Result<Discovery> {
    let root = reach.root;
    let walk = std::cell::RefCell::new(WalkState::default());
    match load_rules(root, true) {
        Rules::Matcher(rules) => {
            walk.borrow_mut().rules.insert(root.to_path_buf(), rules);
        }
        Rules::Unusable(reason) => {
            anyhow::bail!("the .waveflowignore at the library root cannot be used: {reason}")
        }
        Rules::None | Rules::ExcludeAll => {}
    }
    let starts: Vec<(PathBuf, usize)> = match scope {
        None => vec![(root.to_path_buf(), 0)],
        Some(folders) => folders
            .iter()
            .filter_map(|folder| enter_scope(reach, &walk, folder))
            .collect(),
    };
    let mut paths = Vec::new();
    let mut incomplete = false;
    for (start, base) in starts {
        let entries = WalkDir::new(&start)
            .follow_links(reach.follow_links)
            .into_iter()
            .filter_entry(|entry| admit(entry, reach, base, &walk));
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.depth() == 0 && base == 0 => return Err(error.into()),
                Err(error) => {
                    let mut walk = walk.borrow_mut();
                    // A link the walk was told to follow and could not: a loop,
                    // or a target it cannot read. A link, not a folder, so
                    // nothing under a folder is spared for it.
                    let link = if error.loop_ancestor().is_some() {
                        Some(SkippedLink::Loop)
                    } else if reach.follow_links
                        && error.path().is_some_and(|path| {
                            fs::symlink_metadata(path)
                                .is_ok_and(|meta| meta.file_type().is_symlink())
                        })
                    {
                        Some(SkippedLink::Broken)
                    } else {
                        None
                    };
                    let named = error.path().filter(|path| path.starts_with(root));
                    match (link, named) {
                        (Some(why), Some(path)) => walk.skip_link(&relative_to(root, path), why),
                        (_, None) => {
                            tracing::warn!("the library walk failed somewhere it could not name");
                            incomplete = true;
                            walk.unreadable.push(Unreadable {
                                folder: "<unknown>".to_owned(),
                                reason: "folder could not be read",
                            });
                        }
                        (None, Some(path)) => {
                            walk.unreadable(reach, path, "folder could not be read");
                        }
                    }
                    continue;
                }
            };
            // A link is only ever yielded here when it is followed, and then
            // its type is its target's.
            if !entry.file_type().is_file() {
                continue;
            }
            let Some((path, name)) = reach.catalogue_name(entry.path()) else {
                continue;
            };
            let supported = path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| {
                    waveflow_core::scanner::AUDIO_EXTENSIONS
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(ext))
                });
            if supported {
                paths.push((path, name));
            }
        }
    }
    paths.sort_by(|left, right| left.1.cmp(&right.1));
    // Followed links can name one file twice, or name a file the walk also
    // found directly: the first name in catalogue order keeps it.
    let mut read = HashSet::new();
    paths.retain(|(path, _)| read.insert(path.clone()));
    let walk = walk.into_inner();
    if walk.skipped_links > LOGGED_LINKS {
        tracing::info!(
            count = walk.skipped_links - LOGGED_LINKS,
            "further symbolic links in the library were skipped"
        );
    }
    Ok(Discovery {
        paths,
        unreadable: walk.unreadable,
        incomplete: incomplete || walk.incomplete,
        excluded: walk.excluded,
        skipped_links: walk.skipped_links,
    })
}

/// What one walk saw, as relocation needs it: the files it found, what it
/// could not look at, and — for a scoped walk — where it looked at all.
pub(super) struct Walked<'a> {
    pub(super) discovered: &'a HashSet<String>,
    pub(super) unreadable: &'a [String],
    pub(super) incomplete: bool,
    /// The folders a scoped walk covered, `None` for the whole library.
    pub(super) scope: Option<&'a [String]>,
}

/// Whether `path` is `folder` or lies under it, compared as text.
fn under(path: &str, folder: &str) -> bool {
    path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

impl Walked<'_> {
    /// Whether the walk can say this track's file is gone: it looked where the
    /// file was, did not find it, and that place is not under a folder it
    /// could not read. A scoped walk did not look outside its folders, so a
    /// track there is never one it saw missing — which is what keeps a scoped
    /// scan from moving the identity of a track it never looked for onto a
    /// file it found.
    fn saw_missing(&self, relative_path: &str) -> bool {
        !self.incomplete
            && self
                .scope
                .is_none_or(|folders| folders.iter().any(|folder| under(relative_path, folder)))
            && !self.discovered.contains(relative_path)
            && !self
                .unreadable
                .iter()
                .any(|folder| under(relative_path, folder))
    }
}

/// The candidates a scan may still claim.
///
/// A track whose file this walk found is not missing, whatever a hash or a hint
/// says about it, and one already claimed by an earlier file in this batch is
/// spoken for. Nor is one under a folder the walk could not read, or any at all
/// when the walk could not say what it missed (RFC-018 decision 3): its file
/// may be right where it was. What survives is what genuinely disappeared — and
/// a relocation is only recorded when exactly one thing did, because moving a
/// favourite onto the wrong row is worse than not moving it at all.
pub(super) fn unclaimed(
    candidates: Vec<crate::storage::catalog::ExistingTrack>,
    walked: &Walked<'_>,
    claimed: &HashSet<Uuid>,
) -> Vec<crate::storage::catalog::ExistingTrack> {
    candidates
        .into_iter()
        .filter(|candidate| {
            walked.saw_missing(&candidate.relative_path) && !claimed.contains(&candidate.id)
        })
        .collect()
}
