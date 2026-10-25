//! Watching library roots, so a change is scanned in seconds rather than at
//! the next interval (RFC-021 decision 6).
//!
//! One `notify` watcher observes every root recursively. Its events are only a
//! hint that something moved: they name a library, never what to read, and the
//! scan they trigger is the ordinary incremental one, which works out by itself
//! what changed. The debouncing is this module's own, per library: a library is
//! scanned once its changes have been quiet for the configured delay, or once
//! thirty delays have passed since the first of them when they never are.
//!
//! The periodic scan stays the safety net. A watch that cannot be established —
//! a mount that does not support it, the inotify limit reached, a backend that
//! will not start — is logged once and the library is left to that net.

use std::{
    collections::{HashMap, HashSet},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use notify::{
    event::{MetadataKind, ModifyKind},
    Event, EventKind, RecursiveMode, Watcher,
};
use tokio::{sync::mpsc, time::Instant};
use uuid::Uuid;

use crate::{catalog::LibraryRecord, database::Database, scanner::ScanManager};

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
    pending: HashMap<Uuid, (Instant, Instant)>,
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

    /// Records a change in one library.
    pub fn touch(&mut self, library_id: Uuid, now: Instant) {
        self.pending
            .entry(library_id)
            .and_modify(|(_, last)| *last = now)
            .or_insert((now, now));
    }

    /// The libraries whose changes have settled and that no scan is running
    /// for, now no longer pending. One that has settled while a scan of it
    /// runs stays pending, and is answered here once that scan is over: the
    /// running scan may have walked past the change.
    pub fn due(&mut self, now: Instant, scanning: impl Fn(Uuid) -> bool) -> Vec<Uuid> {
        let mut due = Vec::new();
        let (delay, max_wait) = (self.delay, self.max_wait);
        self.pending.retain(|library_id, (first, last)| {
            let settled =
                now.duration_since(*last) >= delay || now.duration_since(*first) >= max_wait;
            if settled && !scanning(*library_id) {
                due.push(*library_id);
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

/// Starts watching every library, and keeps the watched set in step with the
/// catalogue. Answers at once; the watcher runs until the process ends.
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
        let mut refused: HashSet<(Uuid, PathBuf)> = HashSet::new();
        let mut changes = scanner.library_changes();
        let mut refresh = tokio::time::interval(REFRESH_INTERVAL);
        let mut tick = tokio::time::interval(delay.min(Duration::from_secs(1)));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut settle = Settle::new(delay);
        loop {
            tokio::select! {
                _ = refresh.tick() => {
                    reconcile(&db, &mut watcher, &mut watched, &mut refused).await;
                }
                changed = changes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    reconcile(&db, &mut watcher, &mut watched, &mut refused).await;
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
                                if let Some(library_id) = library_of(&watched, path) {
                                    settle.touch(library_id, now);
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
    for library_id in settle.due(now, |id| scanner.is_scanning(id)) {
        let Some(library) = watched.get(&library_id).cloned() else {
            continue;
        };
        if let Err(error) = scanner.trigger(library, None, "watch").await {
            tracing::warn!(%library_id, %error, "could not queue a scan for a watched change");
            settle.touch(library_id, now);
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

/// Watches what the catalogue holds and nothing else.
async fn reconcile(
    db: &Database,
    watcher: &mut notify::RecommendedWatcher,
    watched: &mut HashMap<Uuid, LibraryRecord>,
    refused: &mut HashSet<(Uuid, PathBuf)>,
) {
    let libraries = match db.all_libraries().await {
        Ok(libraries) => libraries,
        Err(error) => {
            tracing::warn!(%error, "could not list the libraries to watch");
            return;
        }
    };
    let current: HashMap<Uuid, LibraryRecord> = libraries
        .into_iter()
        .map(|library| (library.id, library))
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
            let _ = watcher.unwatch(&library.root_path);
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
        match watcher.watch(&library.root_path, RecursiveMode::Recursive) {
            Ok(()) => {
                tracing::debug!(%library_id, "watching the library for changes");
                watched.insert(library_id, library);
            }
            Err(error) => {
                // A recursive watch can fail halfway; take back what it set.
                let _ = watcher.unwatch(&library.root_path);
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
    // What those removals took from libraries still watched is placed again.
    let mut affected: Vec<Uuid> = removed
        .iter()
        .flat_map(|root| sharing_watches_with(watched, root))
        .collect();
    affected.sort();
    affected.dedup();
    for library_id in affected {
        let Some(root) = watched
            .get(&library_id)
            .map(|library| library.root_path.clone())
        else {
            continue;
        };
        if let Err(error) = watcher.watch(&root, RecursiveMode::Recursive) {
            let _ = watcher.unwatch(&root);
            tracing::warn!(
                %library_id,
                %error,
                "the library cannot be watched again; it is scanned on the interval only"
            );
            watched.remove(&library_id);
            refused.insert((library_id, root));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(id: Uuid, root: &str) -> LibraryRecord {
        LibraryRecord {
            id,
            name: "watched".into(),
            root_path: PathBuf::from(root),
        }
    }

    /// `WAVEFLOW_WATCH_DELAY_SECS` has no ceiling: a delay too long to
    /// multiply must not panic the watcher's task.
    #[test]
    fn an_enormous_delay_does_not_panic() {
        let mut settle = Settle::new(Duration::from_secs(u64::MAX));
        let library = Uuid::new_v4();
        let now = Instant::now();
        settle.touch(library, now);
        assert!(settle.due(now, |_| false).is_empty());
    }

    /// Removing a recursive inotify watch removes every descriptor beneath
    /// it, so the libraries nested under it — and the one around it, when the
    /// removed one was nested — are named to be watched again.
    #[test]
    fn a_removed_watch_names_the_libraries_it_shared_descriptors_with() {
        let (outer, inner, beside) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let libraries = HashMap::from([
            (outer, library(outer, "/music")),
            (inner, library(inner, "/music/classical")),
            (beside, library(beside, "/podcasts")),
        ]);
        let mut both = vec![outer, inner];
        both.sort();
        assert_eq!(sharing_watches_with(&libraries, Path::new("/music")), both);
        assert_eq!(
            sharing_watches_with(&libraries, Path::new("/music/classical")),
            both
        );
        assert_eq!(
            sharing_watches_with(&libraries, Path::new("/music/rock")),
            vec![outer]
        );
        assert!(sharing_watches_with(&libraries, Path::new("/elsewhere")).is_empty());
    }

    #[test]
    fn a_library_is_due_once_its_changes_are_quiet() {
        let delay = Duration::from_secs(10);
        let mut settle = Settle::new(delay);
        let library = Uuid::new_v4();
        let start = Instant::now();
        settle.touch(library, start);
        settle.touch(library, start + Duration::from_secs(8));
        assert!(settle
            .due(start + Duration::from_secs(17), |_| false)
            .is_empty());
        assert_eq!(
            settle.due(start + Duration::from_secs(18), |_| false),
            vec![library]
        );
        assert!(
            !settle.is_pending(library),
            "a due library is no longer pending"
        );
    }

    #[test]
    fn changes_that_never_settle_are_scanned_after_thirty_delays() {
        let delay = Duration::from_secs(1);
        let mut settle = Settle::new(delay);
        let library = Uuid::new_v4();
        let start = Instant::now();
        // A change every second, for thirty seconds: never quiet for a delay.
        for second in 0..30 {
            settle.touch(library, start + Duration::from_secs(second));
            assert!(settle
                .due(start + Duration::from_millis(second * 1000 + 100), |_| {
                    false
                })
                .is_empty());
        }
        assert_eq!(
            settle.due(start + Duration::from_secs(30), |_| false),
            vec![library]
        );
    }

    /// Never a second scan while one runs: the library stays pending, and is
    /// due as soon as the scan is over.
    #[test]
    fn no_scan_starts_while_one_runs_for_the_library() {
        let delay = Duration::from_secs(1);
        let mut settle = Settle::new(delay);
        let (busy, idle) = (Uuid::new_v4(), Uuid::new_v4());
        let start = Instant::now();
        settle.touch(busy, start);
        settle.touch(idle, start);
        let later = start + Duration::from_secs(5);
        assert_eq!(settle.due(later, |id| id == busy), vec![idle]);
        assert!(settle.is_pending(busy));
        assert_eq!(settle.due(later, |_| false), vec![busy]);
    }

    #[test]
    fn reads_and_received_files_are_not_changes() {
        let read = Event::new(EventKind::Access(notify::event::AccessKind::Any));
        assert!(!relevant(&read));
        let atime = Event::new(EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::AccessTime,
        )));
        assert!(!relevant(&atime));
        let created = Event::new(EventKind::Create(notify::event::CreateKind::File));
        assert!(relevant(&created));

        let (outer, inner) = (Uuid::new_v4(), Uuid::new_v4());
        let libraries = HashMap::from([
            (outer, library(outer, "/music")),
            (inner, library(inner, "/music/classical")),
        ]);
        assert_eq!(
            library_of(&libraries, Path::new("/music/rock/a.flac")),
            Some(outer)
        );
        assert_eq!(
            library_of(&libraries, Path::new("/music/classical/b.flac")),
            Some(inner),
            "the deepest root owns the path"
        );
        assert_eq!(
            library_of(&libraries, Path::new("/music/.waveflow-managed/c.part")),
            None
        );
        assert_eq!(library_of(&libraries, Path::new("/elsewhere/d.flac")), None);
    }
}
