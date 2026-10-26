//! Watching library roots, so a change is scanned in seconds rather than at
//! the next interval (RFC-021 decision 6).
//!
//! One `notify` watcher observes every root whose library is watched (the
//! per-library `watch` switch), recursively. Its events are a hint that
//! something moved: they name a library and the folders the changes were in,
//! never what to read. The scan they trigger is the ordinary incremental one,
//! limited to those folders when the library reads no link (see
//! [`Settle::touch_folder`]); it works out by itself what changed there. The debouncing is this module's own, per library: a library is
//! scanned once its changes have been quiet for the configured delay, or once
//! thirty delays have passed since the first of them when they never are.
//!
//! The periodic scan stays the safety net. A watch that cannot be established —
//! a mount that does not support it, the inotify limit reached, a backend that
//! will not start — is logged once and the library is left to that net.

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use notify::{
    event::{MetadataKind, ModifyKind},
    Event, EventKind, RecursiveMode, Watcher,
};
use tokio::{sync::mpsc, time::Instant};
use uuid::Uuid;

use crate::{
    library::scanner::ScanManager, storage::catalog::LibraryRecord, storage::database::Database,
};

/// How often the watched set is re-read even when nothing in this process said
/// it changed: the CLI adds and removes libraries from another process.
const REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// How many delays a stream of changes that never settles waits before it is
/// scanned anyway.
const MAX_WAIT_DELAYS: u32 = 30;

/// Libraries with changes waiting, and when they arrived.
#[derive(Debug)]
pub struct Settle {
    delay: Duration,
    max_wait: Duration,
    pending: HashMap<Uuid, Pending>,
}

/// One library's waiting changes: when the first and the last arrived, and
/// the folders they were in — `None` once anything says the whole library
/// must be walked.
#[derive(Debug)]
struct Pending {
    first: Instant,
    last: Instant,
    folders: Option<BTreeSet<String>>,
}

impl Settle {
    pub fn new(delay: Duration) -> Self {
        Self {
            delay,
            // Saturating: `WAVEFLOW_WATCH_DELAY_SECS` has no ceiling, and an
            // overflow here would panic inside the spawned task, which nothing
            // watches — the watcher would be gone without a word.
            max_wait: delay.saturating_mul(MAX_WAIT_DELAYS),
            pending: HashMap::new(),
        }
    }

    /// Records a change in one library that only a walk of the whole library
    /// can answer: a queue overflow, an error, a change at its root, a
    /// library whose walk follows links.
    pub fn touch(&mut self, library_id: Uuid, now: Instant) {
        self.pending
            .entry(library_id)
            .and_modify(|pending| {
                pending.last = now;
                pending.folders = None;
            })
            .or_insert(Pending {
                first: now,
                last: now,
                folders: None,
            });
    }

    /// Records a change in one folder of a library, relative to its root.
    ///
    /// The scan this settles into walks those folders only, which is what
    /// RFC-018's rules allow a scan that did not look everywhere: only what it
    /// walked may be marked missing, and relocation claims nothing outside it.
    /// Past [`crate::library::scanner::MAX_SCOPE_FOLDERS`] folders the changes were
    /// everywhere, and the whole library is walked instead.
    pub fn touch_folder(&mut self, library_id: Uuid, folder: String, now: Instant) {
        let pending = self.pending.entry(library_id).or_insert(Pending {
            first: now,
            last: now,
            folders: Some(BTreeSet::new()),
        });
        pending.last = now;
        if let Some(folders) = pending.folders.as_mut() {
            folders.insert(folder);
            if folders.len() > crate::library::scanner::MAX_SCOPE_FOLDERS {
                pending.folders = None;
            }
        }
    }

    /// The libraries whose changes have settled and that no scan is running
    /// for, now no longer pending. One that has settled while a scan of it
    /// runs stays pending, and is answered here once that scan is over: the
    /// running scan may have walked past the change.
    pub fn due(&mut self, now: Instant, scanning: impl Fn(Uuid) -> bool) -> Vec<Uuid> {
        self.due_with_scope(now, scanning)
            .into_iter()
            .map(|(library_id, _)| library_id)
            .collect()
    }

    /// [`Self::due`], each library with the folders its changes were in, or
    /// `None` for the whole library.
    pub fn due_with_scope(
        &mut self,
        now: Instant,
        scanning: impl Fn(Uuid) -> bool,
    ) -> Vec<(Uuid, Option<Vec<String>>)> {
        let mut due = Vec::new();
        let (delay, max_wait) = (self.delay, self.max_wait);
        self.pending.retain(|library_id, pending| {
            let settled = now.duration_since(pending.last) >= delay
                || now.duration_since(pending.first) >= max_wait;
            if settled && !scanning(*library_id) {
                due.push((
                    *library_id,
                    pending
                        .folders
                        .take()
                        .map(|folders| folders.into_iter().collect()),
                ));
                false
            } else {
                true
            }
        });
        due.sort();
        due
    }

    pub fn is_pending(&self, library_id: Uuid) -> bool {
        self.pending.contains_key(&library_id)
    }
}

/// Whether an event can be a change to the music. Reads are not, and the
/// scanner reads every file it hashes: answering them would have the watcher
/// trigger itself for ever.
pub fn relevant(event: &Event) -> bool {
    !matches!(
        event.kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(MetadataKind::AccessTime))
    )
}

/// The library a changed path belongs to, the deepest root first. Nothing
/// under `.waveflow-managed`: an upload writes there, and applies what it
/// wrote itself.
pub fn library_of(libraries: &HashMap<Uuid, LibraryRecord>, path: &Path) -> Option<Uuid> {
    libraries
        .values()
        .filter_map(|library| {
            let relative = path.strip_prefix(&library.root_path).ok()?;
            let managed = matches!(
                relative.components().next(),
                Some(Component::Normal(first)) if first == crate::services::uploads::MANAGED_DIR
            );
            (!managed).then_some((library.root_path.components().count(), library.id))
        })
        .max()
        .map(|(_, id)| id)
}

/// The folder a change at `path` was in, relative to the root of the library
/// that holds it, as a scoped scan names folders — or `None` when only the
/// whole library can say what it was: a change at the root itself.
///
/// A folder that exists is its own scope; anything else — a file, or
/// something gone — is in its parent's.
pub fn folder_of(root: &Path, path: &Path) -> Option<String> {
    let folder = if std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
        path
    } else {
        path.parent()?
    };
    let relative = folder.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?.to_owned()),
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Starts watching every watched library, and keeps the watched set in step
/// with the catalogue. Answers at once; the watcher runs until the process
/// ends.
pub fn spawn(scanner: ScanManager, db: Database, delay: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (sender, mut events) = mpsc::unbounded_channel::<notify::Result<Event>>();
        let mut watcher = match notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        }) {
            Ok(watcher) => watcher,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "the file system watcher could not start; libraries are scanned on the interval only"
                );
                return;
            }
        };
        let mut watched: HashMap<Uuid, LibraryRecord> = HashMap::new();
        // Those whose walk reads no link, and so may be scanned folder by
        // folder: a folder reached through a link is named by its target in
        // the catalogue, and the path an event names would not match it.
        let mut scopable: HashSet<Uuid> = HashSet::new();
        let mut refused: HashSet<(Uuid, PathBuf)> = HashSet::new();
        let mut changes = scanner.library_changes();
        let mut refresh = tokio::time::interval(REFRESH_INTERVAL);
        let mut tick = tokio::time::interval(delay.min(Duration::from_secs(1)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut settle = Settle::new(delay);
        loop {
            tokio::select! {
                _ = refresh.tick() => {
                    reconcile(&db, &mut watcher, &mut watched, &mut scopable, &mut refused).await;
                }
                changed = changes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    reconcile(&db, &mut watcher, &mut watched, &mut scopable, &mut refused).await;
                }
                Some(event) = events.recv() => {
                    let now = Instant::now();
                    match event {
                        // The kernel dropped events: anything may have changed.
                        Ok(event) if event.need_rescan() => {
                            for library_id in watched.keys() {
                                settle.touch(*library_id, now);
                            }
                        }
                        Ok(event) if relevant(&event) => {
                            for path in &event.paths {
                                let Some(library_id) = library_of(&watched, path) else {
                                    continue;
                                };
                                let folder = scopable
                                    .contains(&library_id)
                                    .then(|| watched.get(&library_id))
                                    .flatten()
                                    .and_then(|library| folder_of(&library.root_path, path));
                                match folder {
                                    Some(folder) => settle.touch_folder(library_id, folder, now),
                                    None => settle.touch(library_id, now),
                                }
                            }
                        }
                        Ok(_) => {}
                        Err(error) => {
                            for path in &error.paths {
                                if let Some(library_id) = library_of(&watched, path) {
                                    settle.touch(library_id, now);
                                }
                            }
                            tracing::debug!(%error, "the file system watcher reported an error");
                        }
                    }
                }
                _ = tick.tick() => {
                    dispatch_due(&mut settle, &watched, &scanner, Instant::now()).await;
                }
            }
        }
    })
}

/// Queues a scan for every watched library whose changes have settled, and
/// that no scan is running for.
///
/// A library whose scan cannot be queued goes back to pending: it has left
/// `pending` to be dispatched, and dropping it there would leave the change to
/// the periodic scan — which `WAVEFLOW_SCAN_INTERVAL_SECS=0` turns off. One the
/// watcher no longer holds is dropped: it was deleted, and nothing is owed.
pub async fn dispatch_due(
    settle: &mut Settle,
    watched: &HashMap<Uuid, LibraryRecord>,
    scanner: &ScanManager,
    now: Instant,
) {
    for (library_id, scope) in settle.due_with_scope(now, |id| scanner.is_scanning(id)) {
        let Some(library) = watched.get(&library_id).cloned() else {
            continue;
        };
        let queued = match scope.as_deref() {
            Some(folders) => scanner.trigger_scoped(library, "watch", folders).await,
            None => scanner.trigger(library, None, "watch").await,
        };
        if let Err(error) = queued {
            tracing::warn!(%library_id, %error, "could not queue a scan for a watched change");
            match scope {
                Some(folders) => {
                    for folder in folders {
                        settle.touch_folder(library_id, folder, now);
                    }
                }
                None => settle.touch(library_id, now),
            }
        }
    }
}

/// The watched libraries a removed watch on `removed` may have taken down with
/// it.
///
/// On inotify a recursive watch is one descriptor per folder, shared by every
/// watch that covers that folder, and removing a recursive watch removes every
/// descriptor beneath it. A library nested under the removed root loses its
/// watch, and so does the library around it when the removed one was nested:
/// both are watched again.
pub fn sharing_watches_with(watched: &HashMap<Uuid, LibraryRecord>, removed: &Path) -> Vec<Uuid> {
    let mut affected: Vec<Uuid> = watched
        .values()
        .filter(|library| {
            library.root_path.starts_with(removed) || removed.starts_with(&library.root_path)
        })
        .map(|library| library.id)
        .collect();
    affected.sort();
    affected
}

/// What [`reconcile`] needs of a file system watcher: place a recursive watch
/// on a root, and take one back. A trait so the order of those calls — which
/// is what decides whether a library stays watched on inotify — can be tested
/// against a model of the kernel's sharing rather than against Linux alone.
pub trait Watches {
    fn watch(&mut self, root: &Path) -> Result<(), String>;
    /// Best effort: a watch that was never placed is nothing to take back.
    fn unwatch(&mut self, root: &Path);
}

impl Watches for notify::RecommendedWatcher {
    fn watch(&mut self, root: &Path) -> Result<(), String> {
        Watcher::watch(self, root, RecursiveMode::Recursive).map_err(|error| error.to_string())
    }

    fn unwatch(&mut self, root: &Path) {
        let _ = Watcher::unwatch(self, root);
    }
}

/// Watches what the catalogue holds and asks to be watched, and nothing else:
/// a library whose `watch` switch is off is taken out of the watched set as
/// a deleted one is, and left to the periodic scan.
async fn reconcile(
    db: &Database,
    watcher: &mut impl Watches,
    watched: &mut HashMap<Uuid, LibraryRecord>,
    scopable: &mut HashSet<Uuid>,
    refused: &mut HashSet<(Uuid, PathBuf)>,
) {
    let libraries = match db.watched_libraries().await {
        Ok(libraries) => libraries,
        Err(error) => {
            tracing::warn!(%error, "could not list the libraries to watch");
            return;
        }
    };
    *scopable = libraries
        .iter()
        .filter(|(_, follow_links)| !follow_links)
        .map(|(library, _)| library.id)
        .collect();
    let current: HashMap<Uuid, LibraryRecord> = libraries
        .into_iter()
        .map(|(library, _)| (library.id, library))
        .collect();
    let gone: Vec<Uuid> = watched
        .iter()
        .filter(|(id, library)| {
            current
                .get(id)
                .is_none_or(|now| now.root_path != library.root_path)
        })
        .map(|(id, _)| *id)
        .collect();
    // Every root whose watch was taken back, so the watches it shared can be
    // put back once the set is settled.
    let mut removed: Vec<PathBuf> = Vec::new();
    for library_id in gone {
        if let Some(library) = watched.remove(&library_id) {
            watcher.unwatch(&library.root_path);
            removed.push(library.root_path);
        }
    }
    refused.retain(|(id, root)| current.get(id).is_some_and(|now| now.root_path == *root));
    for (library_id, library) in current {
        if watched.contains_key(&library_id)
            || refused.contains(&(library_id, library.root_path.clone()))
        {
            continue;
        }
        match watcher.watch(&library.root_path) {
            Ok(()) => {
                tracing::debug!(%library_id, "watching the library for changes");
                watched.insert(library_id, library);
            }
            Err(error) => {
                // A recursive watch can fail halfway; take back what it set.
                watcher.unwatch(&library.root_path);
                tracing::warn!(
                    %library_id,
                    %error,
                    "the library cannot be watched; it is scanned on the interval only"
                );
                removed.push(library.root_path.clone());
                refused.insert((library_id, library.root_path));
            }
        }
    }
    rewatch(watcher, watched, refused, removed);
}

/// Places again what the removals took from libraries still watched.
///
/// A worklist rather than one pass, because the cleanup of a re-install that
/// fails is itself a removal: on inotify it strips the descriptors of an
/// enclosing library that was just re-installed. So every failed root goes
/// back on the list, and what it shared is watched again in turn.
///
/// It ends: a root only joins the list when its library leaves `watched`,
/// which happens once per library, and a successful re-install adds nothing.
/// Enclosing roots go first, the order in which a nested failure does the most
/// damage — so the worklist, not the order, is what keeps them watched.
pub fn rewatch(
    watcher: &mut impl Watches,
    watched: &mut HashMap<Uuid, LibraryRecord>,
    refused: &mut HashSet<(Uuid, PathBuf)>,
    removed: Vec<PathBuf>,
) {
    let mut cleared = removed;
    while let Some(root_removed) = cleared.pop() {
        let mut affected: Vec<(usize, Uuid, PathBuf)> =
            sharing_watches_with(watched, &root_removed)
                .into_iter()
                .filter_map(|id| {
                    watched.get(&id).map(|library| {
                        (
                            library.root_path.components().count(),
                            id,
                            library.root_path.clone(),
                        )
                    })
                })
                .collect();
        affected.sort();
        for (_, library_id, root) in affected {
            if !watched.contains_key(&library_id) {
                continue;
            }
            if let Err(error) = watcher.watch(&root) {
                watcher.unwatch(&root);
                tracing::warn!(
                    %library_id,
                    %error,
                    "the library cannot be watched again; it is scanned on the interval only"
                );
                watched.remove(&library_id);
                refused.insert((library_id, root.clone()));
                cleared.push(root);
            }
        }
    }
}

#[cfg(test)]
mod tests;
