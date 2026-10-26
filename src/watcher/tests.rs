//! Unit tests of the watcher: debouncing, scoping a change to its folder,
//! which events count, and how watches are shared and re-established.

use super::*;

fn library(id: Uuid, root: &str) -> LibraryRecord {
    LibraryRecord {
        id,
        name: "watched".into(),
        root_path: PathBuf::from(root),
    }
}

/// inotify, as far as sharing goes: one descriptor per folder, shared by
/// every watch that covers it, and taking a recursive watch back clears
/// every descriptor beneath its root. A refused root fails partway, after
/// placing its own descriptor, as a watch limit reached mid-tree does.
struct SharedDescriptors {
    folders: Vec<PathBuf>,
    descriptors: HashSet<PathBuf>,
    refuse: HashSet<PathBuf>,
}

impl Watches for SharedDescriptors {
    fn watch(&mut self, root: &Path) -> Result<(), String> {
        for folder in self
            .folders
            .iter()
            .filter(|folder| folder.starts_with(root))
        {
            if self.refuse.contains(root) && folder != root {
                return Err("no space left for another watch".into());
            }
            self.descriptors.insert(folder.clone());
        }
        Ok(())
    }

    fn unwatch(&mut self, root: &Path) {
        self.descriptors.retain(|folder| !folder.starts_with(root));
    }
}

impl SharedDescriptors {
    fn covers(&self, root: &str) -> bool {
        self.folders
            .iter()
            .filter(|folder| folder.starts_with(root))
            .all(|folder| self.descriptors.contains(folder))
    }
}

/// The enclosing library is re-installed, then the nested one fails and
/// is cleaned up — which strips the enclosing library's descriptors in the
/// nested tree. The enclosing library must end up watched whole.
#[test]
fn a_nested_failure_does_not_leave_the_library_around_it_half_watched() {
    let folders: Vec<PathBuf> = [
        "/music",
        "/music/rock",
        "/music/classical",
        "/music/classical/bach",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    let mut kernel = SharedDescriptors {
        descriptors: folders.iter().cloned().collect(),
        folders,
        refuse: HashSet::from([PathBuf::from("/music/classical")]),
    };
    let (outer, inner) = (Uuid::new_v4(), Uuid::new_v4());
    let mut watched = HashMap::from([
        (outer, library(outer, "/music")),
        (inner, library(inner, "/music/classical")),
    ]);
    let mut refused = HashSet::new();
    // A library at the bottom of both went away, and took its folder's
    // descriptor with it.
    kernel.unwatch(Path::new("/music/classical/bach"));

    rewatch(
        &mut kernel,
        &mut watched,
        &mut refused,
        vec![PathBuf::from("/music/classical/bach")],
    );

    assert!(
        kernel.covers("/music"),
        "the enclosing library was left without its nested folders"
    );
    assert!(watched.contains_key(&outer));
    assert!(!watched.contains_key(&inner));
    assert!(refused.contains(&(inner, PathBuf::from("/music/classical"))));
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

/// Folders gather until the library settles; anything that needs the
/// whole library, or too many folders, turns the scope into all of it.
#[test]
fn a_settled_library_names_the_folders_its_changes_were_in() {
    let delay = Duration::from_secs(1);
    let mut settle = Settle::new(delay);
    let (scoped, whole, crowded) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let start = Instant::now();
    settle.touch_folder(scoped, "Rock/Album".into(), start);
    settle.touch_folder(scoped, "Jazz".into(), start);
    settle.touch_folder(scoped, "Jazz".into(), start);
    settle.touch_folder(whole, "Rock".into(), start);
    settle.touch(whole, start);
    settle.touch_folder(whole, "Jazz".into(), start);
    for index in 0..=crate::scanner::MAX_SCOPE_FOLDERS {
        settle.touch_folder(crowded, format!("f{index}"), start);
    }
    let mut due = settle.due_with_scope(start + delay, |_| false);
    due.sort_by_key(|(id, _)| *id);
    let scope = |id: Uuid| due.iter().find(|(due, _)| *due == id).unwrap().1.clone();
    assert_eq!(
        scope(scoped),
        Some(vec!["Jazz".to_owned(), "Rock/Album".to_owned()])
    );
    assert_eq!(scope(whole), None, "once whole, a library stays whole");
    assert_eq!(scope(crowded), None);
}

#[test]
fn a_change_is_scoped_to_the_folder_it_was_in() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("Artist/Album")).unwrap();
    std::fs::write(root.join("Artist/Album/a.flac"), b"x").unwrap();
    assert_eq!(
        folder_of(root, &root.join("Artist/Album/a.flac")).as_deref(),
        Some("Artist/Album")
    );
    assert_eq!(
        folder_of(root, &root.join("Artist/Album")).as_deref(),
        Some("Artist/Album"),
        "a folder is its own scope"
    );
    assert_eq!(
        folder_of(root, &root.join("Artist/Gone/b.flac")).as_deref(),
        Some("Artist/Gone"),
        "something gone is in its parent's"
    );
    assert_eq!(folder_of(root, &root.join("top.flac")), None);
    assert_eq!(folder_of(root, root), None);
    assert_eq!(folder_of(root, Path::new("/elsewhere/c.flac")), None);
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
