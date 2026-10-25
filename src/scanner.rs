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
use tokio::sync::{broadcast, watch, Mutex};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::{
    catalog::{
        ApplyOutcome, ArtworkInput, CatalogApply, CatalogTrackInput, LibraryRecord, ScanJobRecord,
    },
    database::Database,
    lyrics::{self, LyricsInput},
};

const MAX_LYRICS_BYTES: u64 = 1024 * 1024;

/// One reading of a scan in flight, as it travels between the scanning task and
/// whoever is watching.
///
/// Deliberately **not** `Serialize`. It used to be, and `/api/v2/scans/{id}/events`
/// yielded it verbatim for every `progress` event while the opening `snapshot`
/// carried a [`ScanJobRecord`] — two shapes of the same reading on one stream,
/// disagreeing on three names out of thirteen. A watcher that bound `total_files`
/// showed `undefined` from the first progress frame on. The stream now converts
/// through [`ScanProgress::record`], and dropping the derive is what stops the
/// other shape coming back: there is no way to put this type on a wire.
#[derive(Debug, Clone)]
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
    /// Entries the walk left out (RFC-021 decision 2).
    pub excluded: i64,
    /// Symbolic links the walk did not follow (RFC-021 decision 3).
    pub skipped_links: i64,
    pub current_path: Option<String>,
    pub message: Option<String>,
}

impl ScanProgress {
    /// The same reading in the shape `GET /api/v2/scans/{id}` answers, which is
    /// the one shape the event stream speaks.
    pub fn record(&self) -> ScanJobRecord {
        ScanJobRecord {
            id: self.scan_id,
            library_id: self.library_id,
            status: self.status.clone(),
            total_files: self.total as i64,
            processed_files: self.processed as i64,
            added: self.added,
            updated: self.updated,
            moved: self.moved,
            skipped: self.skipped,
            unavailable: self.unavailable,
            errors: self.errors,
            excluded: self.excluded,
            skipped_links: self.skipped_links,
            current_path: self.current_path.clone(),
            message: self.message.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ScanManager {
    db: Database,
    artwork_dir: PathBuf,
    parallelism: usize,
    events: Arc<DashMap<Uuid, broadcast::Sender<ScanProgress>>>,
    library_locks: Arc<DashMap<Uuid, Arc<Mutex<()>>>>,
    /// How many scan jobs are running, of any library. The quality analysis
    /// (RFC-016 decision 4) starts no file while this is above zero, and wakes
    /// when it falls back to it.
    in_flight: Arc<watch::Sender<usize>>,
    /// Which libraries those scans belong to, with how many of each. The
    /// quality page says "waiting for the scan" only to someone who can see
    /// the library being scanned.
    scanning: Arc<DashMap<Uuid, usize>>,
    /// Moves whenever a library is added, moved or deleted through this
    /// process, so the watcher (RFC-021 decision 6) re-reads what to watch.
    libraries: Arc<watch::Sender<u64>>,
    /// The folders outside every library a followed link may reach into
    /// (`WAVEFLOW_LINK_ROOTS`).
    link_roots: crate::link_roots::LinkRoots,
}

/// One scan counted as running, for as long as this lives.
pub struct ScanInFlight {
    count: Arc<watch::Sender<usize>>,
    scanning: Arc<DashMap<Uuid, usize>>,
    library_id: Uuid,
}

impl Drop for ScanInFlight {
    fn drop(&mut self) {
        self.scanning.remove_if_mut(&self.library_id, |_, count| {
            *count = count.saturating_sub(1);
            *count == 0
        });
        self.count
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

impl ScanManager {
    /// Reads one track's file and returns what its tags say.
    ///
    /// For a caller that removed a correction and now needs the file's own
    /// values back: they were overwritten in the catalogue, so the file is the
    /// only place left holding them. Read before anything is written, so a file
    /// that cannot be read refuses the whole operation rather than leaving a
    /// correction removed and its rows behind.
    pub(crate) async fn read_track_input(
        &self,
        root_path: &std::path::Path,
        relative_path: &str,
    ) -> Result<CatalogTrackInput, String> {
        let root = tokio::fs::canonicalize(root_path)
            .await
            .map_err(|error| format!("library root: {error}"))?;
        let path = root.join(relative_path);
        let artwork_dir = self.artwork_dir.clone();
        let mut input = tokio::task::spawn_blocking(move || extract_file(&path, &artwork_dir))
            .await
            .map_err(|error| format!("extract worker failed: {error}"))??;
        input.relative_path = relative_path.to_owned();
        Ok(input)
    }

    pub fn new(db: Database, artwork_dir: PathBuf, parallelism: usize) -> Self {
        Self {
            db,
            artwork_dir,
            parallelism: parallelism.max(1),
            events: Arc::new(DashMap::new()),
            library_locks: Arc::new(DashMap::new()),
            in_flight: Arc::new(watch::Sender::new(0)),
            scanning: Arc::new(DashMap::new()),
            libraries: Arc::new(watch::Sender::new(0)),
            link_roots: crate::link_roots::LinkRoots::default(),
        }
    }

    /// The folders outside every library that a library following links may
    /// reach into.
    pub fn with_link_roots(mut self, link_roots: crate::link_roots::LinkRoots) -> Self {
        self.link_roots = link_roots;
        self
    }

    /// Queues a scan of `scope` only, folders relative to the library root,
    /// and runs it: what the watcher asks for once it can say where the
    /// changes were. A scope that is not plainly such a list scans the whole
    /// library instead, with the same trigger.
    pub async fn trigger_scoped(
        &self,
        library: LibraryRecord,
        trigger: &'static str,
        scope: &[String],
    ) -> anyhow::Result<Uuid> {
        let Some(scope) = normalised_scope(scope) else {
            return self.trigger(library, None, trigger).await;
        };
        let scan_id = self
            .db
            .create_scoped_scan_job(library.id, trigger, &scope)
            .await?;
        self.spawn(scan_id, library);
        Ok(scan_id)
    }

    /// The lock every scan of one library holds for its whole run.
    fn library_lock(&self, library_id: Uuid) -> Arc<Mutex<()>> {
        self.library_locks
            .entry(library_id)
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// That lock, if no scan of the library holds it now.
    ///
    /// For what must not interleave with a scan and must not wait for one
    /// either (RFC-021 decisions 4 and 5): a purge, a move, a deletion. A scan
    /// queued behind it waits until it is dropped, and then reads the library
    /// as it was left.
    pub fn try_lock_library(&self, library_id: Uuid) -> Option<tokio::sync::OwnedMutexGuard<()>> {
        self.library_lock(library_id).try_lock_owned().ok()
    }

    /// Says that the set of libraries, or a root, changed.
    pub fn libraries_changed(&self) {
        self.libraries
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    /// A way to wait for [`Self::libraries_changed`].
    pub fn library_changes(&self) -> watch::Receiver<u64> {
        self.libraries.subscribe()
    }

    /// Counts a scan of `library_id` as running until the guard is dropped.
    /// Every job takes one for its whole run, success or failure.
    pub fn begin_scan(&self, library_id: Uuid) -> ScanInFlight {
        *self.scanning.entry(library_id).or_insert(0) += 1;
        self.in_flight.send_modify(|count| *count += 1);
        ScanInFlight {
            count: Arc::clone(&self.in_flight),
            scanning: Arc::clone(&self.scanning),
            library_id,
        }
    }

    /// Whether a scan of this library is running.
    pub fn is_scanning(&self, library_id: Uuid) -> bool {
        self.scanning.contains_key(&library_id)
    }

    /// How many scans are running, and a way to wait for that to change.
    pub fn scans_in_flight(&self) -> watch::Receiver<usize> {
        self.in_flight.subscribe()
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
        let running = self.begin_scan(library_id);
        tokio::spawn(async move {
            let _running = running;
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
                    excluded: 0,
                    skipped_links: 0,
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
            manager.boot().await;
            let Some(interval) = interval else { return };
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                manager.trigger_all("scheduled", &HashSet::new()).await;
            }
        });
    }

    /// What the server does with its libraries when it starts: the jobs a
    /// previous run left queued are run, and every other library gets the
    /// boot scan. Public so a test can run it rather than restart a server.
    pub async fn boot(&self) {
        let resumed = self.resume_queued().await;
        self.trigger_all("boot", &resumed).await;
    }

    /// Runs the jobs a previous run queued and never started, and answers the
    /// libraries they belong to.
    ///
    /// A job is queued in the transaction of the change that owes it (RFC-021
    /// decision 5: a library moved, or its links setting changed), and started
    /// after the commit. A process that stops in between leaves a `queued` row
    /// no task holds — one that would also have Subsonic's `scanStatus` answer
    /// "scanning" for good. Running it is what the row promised. A library
    /// whose job resumes needs no boot scan of its own; one whose job cannot
    /// be resumed still gets it.
    pub async fn resume_queued(&self) -> HashSet<Uuid> {
        let mut resumed = HashSet::new();
        let queued = match self.db.queued_scan_jobs().await {
            Ok(queued) => queued,
            Err(error) => {
                tracing::warn!(error = %error, "could not list the scans a previous run left queued");
                return resumed;
            }
        };
        for (scan_id, library_id) in queued {
            match self.db.library_scan_settings(library_id).await {
                Ok(Some(settings)) => {
                    tracing::info!(%scan_id, %library_id, "resuming a scan a previous run left queued");
                    self.spawn(scan_id, settings.library);
                    resumed.insert(library_id);
                }
                // The job went with its library.
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%scan_id, error = %error, "could not resume a queued scan")
                }
            }
        }
        resumed
    }

    async fn trigger_all(&self, trigger: &'static str, skip: &HashSet<Uuid>) {
        match self.db.all_libraries().await {
            Ok(libraries) => {
                for library in libraries {
                    if skip.contains(&library.id) {
                        continue;
                    }
                    if let Err(error) = self.trigger(library, None, trigger).await {
                        tracing::warn!(error = %error, "could not queue background scan");
                    }
                }
            }
            Err(error) => tracing::warn!(error = %error, "could not list libraries for scanner"),
        }
    }

    async fn run(&self, scan_id: Uuid, library: LibraryRecord) -> anyhow::Result<()> {
        let lock = self.library_lock(library.id);
        let _library_guard = lock.lock().await;
        // Read again under the lock rather than trusted from the queue: the
        // library may have been moved, had its links setting changed, or been
        // deleted since this job was queued (RFC-021 decision 5).
        let Some(settings) = self.db.library_scan_settings(library.id).await? else {
            tracing::info!(library_id = %library.id, %scan_id, "the library was deleted before its scan ran");
            return Ok(());
        };
        let library = settings.library;
        tokio::fs::create_dir_all(&self.artwork_dir).await?;
        let root = tokio::fs::canonicalize(&library.root_path).await?;
        let follow_links = settings.follow_links;
        // The folders a followed link may reach outside the root, and every
        // other library's root, which none may reach into. Resolved now, so a
        // mount that appeared since the boot counts and one that went does not.
        let (link_roots, libraries) = if follow_links && !self.link_roots.is_empty() {
            let others = self
                .db
                .all_libraries()
                .await?
                .into_iter()
                .filter(|other| other.id != library.id)
                .filter_map(|other| std::fs::canonicalize(&other.root_path).ok())
                .collect::<Vec<_>>();
            (self.link_roots.resolved(), others)
        } else {
            (Vec::new(), Vec::new())
        };
        // Only a walk that reads no link is ever scoped: a folder reached
        // through a link is named by its target, and the watcher names the
        // path it saw, so the two could not be told to agree.
        let scope = match self.db.scan_scope(scan_id).await? {
            Some(scope) if !follow_links => normalised_scope(&scope),
            _ => None,
        };
        if let Some(scope) = scope.as_ref() {
            tracing::info!(
                library_id = %library.id,
                folders = scope.len(),
                "scanning the folders the watcher saw change"
            );
        }
        let Discovery {
            paths,
            unreadable,
            incomplete,
            excluded,
            skipped_links,
        } = tokio::task::spawn_blocking({
            let root = root.clone();
            let scope = scope.clone();
            move || {
                discover_audio(
                    &Reach {
                        root: &root,
                        follow_links,
                        link_roots: &link_roots,
                        libraries: &libraries,
                    },
                    scope.as_deref(),
                )
            }
        })
        .await??;
        let unreadable_folders: Vec<String> = unreadable
            .iter()
            .map(|folder| folder.folder.clone())
            .collect();
        let discovered: HashSet<String> =
            paths.iter().map(|(_, relative)| relative.clone()).collect();
        // Read here rather than taken as an argument: a scan that died halfway
        // left the request standing, and the run picking up after it has to
        // honour a request it never saw made.
        let full = self.db.full_scan_requested(library.id).await?;
        if full {
            tracing::info!(library_id = %library.id, "scanning every file, skipping nothing");
        }
        self.db
            .start_walked_scan_job(
                scan_id,
                paths.len() as i64,
                full,
                excluded as i64,
                skipped_links as i64,
            )
            .await?;

        // What the catalogue already knows, read once: the pre-check compares
        // against it before a file is opened (RFC-018 decision 1).
        let facts = Arc::new(self.db.track_skip_facts(library.id).await?);
        let artwork_dir = self.artwork_dir.clone();
        // In path order, as they finish, a few batches at most in memory
        // (RFC-018 decision 2): `buffered` keeps the order `paths` has, so the
        // outcome is the same however the extractions interleave.
        let mut scanned = stream::iter(paths.into_iter().map(|(path, relative)| {
            let artwork = artwork_dir.clone();
            let facts = Arc::clone(&facts);
            async move {
                let relative: anyhow::Result<String> = Ok(relative);
                let known = relative
                    .as_ref()
                    .ok()
                    .and_then(|relative| facts.get(relative))
                    .cloned();
                let result =
                    tokio::task::spawn_blocking(move || scan_file(&path, &artwork, known, full))
                        .await;
                (relative, result)
            }
        }))
        .buffered(self.parallelism)
        .chunks(25);

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
            excluded: excluded as i64,
            skipped_links: skipped_links as i64,
            current_path: None,
            message: None,
        };

        // A folder the walk could not read is reported, and its tracks are left
        // as they are below (RFC-018 decision 3). So is one whose ignore file
        // the walk could not use (RFC-021 decision 1), with its own reason.
        if !unreadable.is_empty() {
            progress.errors += unreadable.len() as i64;
            let errors: Vec<(String, String)> = unreadable
                .iter()
                .map(|folder| (folder.folder.clone(), folder.reason.to_owned()))
                .collect();
            self.db.add_scan_errors(scan_id, &errors).await?;
            // Written now, not left to the batches: a library whose only
            // content sits under the unreadable folder has no batch at all.
            self.tick(&progress).await?;
        }

        let walked = Walked {
            discovered: &discovered,
            unreadable: &unreadable_folders,
            incomplete,
            scope: scope.as_deref(),
        };
        while let Some(batch) = scanned.next().await {
            let mut applies = Vec::new();
            let mut seen = Vec::new();
            let mut errors = Vec::new();
            let mut claimed_relocations = HashSet::new();
            let mut hints = Vec::new();
            let mut signatures = Vec::new();
            for (relative, task_result) in batch {
                progress.processed += 1;
                let task_result = match task_result {
                    // Unchanged, and never opened.
                    Ok(Scanned::Skipped(id)) => {
                        seen.push(id);
                        progress.skipped += 1;
                        continue;
                    }
                    Ok(Scanned::Read(result, signature)) => {
                        // Only a read that succeeded records what it read: a
                        // failed one must be tried again, sidecars included.
                        if let (Ok(relative), Ok(_), Some(signature)) =
                            (&relative, result.as_ref(), signature)
                        {
                            signatures.push((relative.clone(), signature));
                        }
                        Ok(*result)
                    }
                    Err(error) => Err(error),
                };
                match (relative, task_result) {
                    (Ok(relative), Ok(Ok(input))) => {
                        let mut input = input;
                        progress.current_path = Some(relative.clone());
                        input.relative_path = relative.clone();
                        let existing = self
                            .db
                            .existing_track_by_path(library.id, &relative)
                            .await?;
                        if let Some(existing) = existing.as_ref() {
                            if !full
                                && existing.file_size == input.file_size
                                && existing.modified_at == input.modified_at
                                && existing.quick_hash == input.quick_hash
                                && existing.full_hash == input.full_hash
                                && existing.lyrics_hash.as_deref() == Some(&input.lyrics_hash)
                                && existing.available
                            {
                                // Skipped, but its hint may predate the
                                // column. Computed only while it is missing,
                                // so the scan after this one pays nothing.
                                if existing.pid.is_none() {
                                    hints.push((
                                        existing.id,
                                        crate::catalog::track_pid(
                                            self.db.pid(),
                                            library.id,
                                            &input,
                                        ),
                                    ));
                                }
                                seen.push(existing.id);
                                progress.skipped += 1;
                                continue;
                            }
                        }
                        let mut moved = false;
                        let existing_id = if let Some(existing) = existing {
                            Some(existing.id)
                        } else {
                            // Content first: a byte-identical file under a new
                            // path is the same file, and nothing else needs to
                            // be consulted.
                            let mut missing = unclaimed(
                                self.db
                                    .relocation_candidates(library.id, &input.full_hash)
                                    .await?,
                                &walked,
                                &claimed_relocations,
                            );
                            if missing.is_empty() {
                                // The bytes changed as well as the path — a
                                // re-encode, typically. The tags are the only
                                // thing left that can say the two files are
                                // one, so the hint is asked, and only here:
                                // where the hash spoke, it was believed.
                                missing = unclaimed(
                                    self.db
                                        .relocation_candidates_by_pid(
                                            library.id,
                                            crate::catalog::track_pid(
                                                self.db.pid(),
                                                library.id,
                                                &input,
                                            ),
                                        )
                                        .await?,
                                    &walked,
                                    &claimed_relocations,
                                );
                            }
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
                            // A scan is nobody's request in particular.
                            origin_device_id: None,
                        });
                    }
                    (relative, result) => {
                        progress.errors += 1;
                        let path = relative.unwrap_or_else(|_| "<invalid-relative-path>".into());
                        let message = match result {
                            Ok(Err(error)) => error,
                            Err(error) => format!("extract worker failed: {error}"),
                            Ok(Ok(_)) => "relative path failed".into(),
                        };
                        progress.current_path = Some(path.clone());
                        errors.push((path, message));
                    }
                }
            }
            self.db.mark_tracks_seen(&seen, scan_id).await?;
            self.db.backfill_track_pids(&hints).await?;
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
            self.db
                .set_sidecar_signatures(library.id, &signatures)
                .await?;
            self.db.add_scan_errors(scan_id, &errors).await?;
            self.tick(&progress).await?;
        }

        self.db
            .mark_tracks_under_seen(library.id, &unreadable_folders, scan_id)
            .await?;
        // A walk error that named no path leaves no folder to spare, so
        // nothing this walk missed can be called missing.
        progress.unavailable = if incomplete {
            tracing::warn!(
                library_id = %library.id,
                "the walk could not say what it failed to read; nothing is marked missing this scan"
            );
            0
        } else {
            self.db
                .mark_unseen_unavailable_within(library.id, scan_id, scope.as_deref())
                .await?
        };
        // After availability is settled, not before: the album and artist
        // identifiers are a majority vote over the tracks that are still there,
        // so a file that vanished must stop voting first.
        // Sort names first: the derivation pass closes by indexing every
        // artist name together with its sort name, so that name has to be
        // settled before it is read.
        self.db.consolidate_sort_names(library.id).await?;
        self.db.consolidate_catalog_derivations(library.id).await?;
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

fn scan_id(progress: &ScanProgress) -> Uuid {
    progress.scan_id
}

/// What a scan does with one discovered file.
enum Scanned {
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
fn scan_file(
    path: &Path,
    artwork_dir: &Path,
    known: Option<crate::catalog::SkipFacts>,
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
fn sidecar_options() -> fs::OpenOptions {
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
struct Unreadable {
    folder: String,
    reason: &'static str,
}

/// What the walk found: the audio files, in catalogue path order, each with
/// the path it is read from and the path the catalogue knows it by, and the
/// folders it could not read, relative to the root.
struct Discovery {
    paths: Vec<(PathBuf, String)>,
    unreadable: Vec<Unreadable>,
    /// An error named no path inside the root, so the walk cannot say which
    /// tracks it missed: none may be marked missing.
    incomplete: bool,
    /// Entries left out by an exclusion, a folder counting once whatever it
    /// holds (RFC-021 decision 2). They count as not discovered.
    excluded: usize,
    /// Symbolic links the walk did not follow (RFC-021 decision 3).
    skipped_links: usize,
}

/// Where one walk may go: the library root, whether it follows links, and —
/// when it does — the folders outside the root a link may reach into and the
/// other libraries' roots no link may reach into (RFC-021, links outside the
/// root). Every path in it is canonical.
struct Reach<'a> {
    root: &'a Path,
    follow_links: bool,
    link_roots: &'a [PathBuf],
    libraries: &'a [PathBuf],
}

impl Reach<'_> {
    /// Whether a followed link may lead to `target`, a canonical path.
    fn admits_target(&self, target: &Path) -> bool {
        target.starts_with(self.root)
            || crate::link_roots::admits(self.link_roots, self.libraries, target)
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
        (crate::link_roots::admits(self.link_roots, self.libraries, &canonical)
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
        let folder = folder.trim_matches('/');
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
    // Sorted, a folder's descendants follow it directly.
    let mut kept: Vec<String> = Vec::with_capacity(folders.len());
    for folder in folders {
        let nested = kept.last().is_some_and(|parent| {
            folder
                .strip_prefix(parent.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
        });
        if !nested {
            kept.push(folder);
        }
    }
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
fn discover_audio(reach: &Reach<'_>, scope: Option<&[String]>) -> anyhow::Result<Discovery> {
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
struct Walked<'a> {
    discovered: &'a HashSet<String>,
    unreadable: &'a [String],
    incomplete: bool,
    /// The folders a scoped walk covered, `None` for the whole library.
    scope: Option<&'a [String]>,
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
fn unclaimed(
    candidates: Vec<crate::catalog::ExistingTrack>,
    walked: &Walked<'_>,
    claimed: &HashSet<Uuid>,
) -> Vec<crate::catalog::ExistingTrack> {
    candidates
        .into_iter()
        .filter(|candidate| {
            walked.saw_missing(&candidate.relative_path) && !claimed.contains(&candidate.id)
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::{
        built_in_exclusion, extended_tags, load_rules, read_sidecar, Rules, Walked, IGNORE_FILE,
        MAX_IGNORE_BYTES, MAX_LYRICS_BYTES,
    };

    /// RFC-021 decision 1: what is left out by name, below the root only, and
    /// the one dot folder that is not.
    #[test]
    fn built_in_exclusions_spare_the_folder_uploads_land_in() {
        use std::ffi::OsStr;
        for name in [
            ".hidden",
            "._Track.flac",
            ".Trash-1000",
            "@eaDir",
            "#recycle",
            "$Recycle.Bin",
            "System Volume Information",
            "LOST+FOUND",
        ] {
            assert!(built_in_exclusion(OsStr::new(name), 1, true), "{name}");
        }
        assert!(!built_in_exclusion(OsStr::new("Album"), 1, true));
        assert!(!built_in_exclusion(OsStr::new("eaDir"), 1, true));
        // Received files: kept directly under the root, and only there.
        assert!(!built_in_exclusion(
            OsStr::new(".waveflow-managed"),
            1,
            true
        ));
        assert!(built_in_exclusion(OsStr::new(".waveflow-managed"), 2, true));
        assert!(built_in_exclusion(
            OsStr::new(".waveflow-managed"),
            1,
            false
        ));
    }

    /// RFC-021 decision 1: an empty file excludes its folder except at the
    /// root, and a file the walk cannot use says why.
    #[test]
    fn ignore_files_are_read_as_the_rfc_says() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        assert!(matches!(load_rules(dir, false), Rules::None));

        std::fs::write(dir.join(IGNORE_FILE), " \n\t\n").unwrap();
        assert!(matches!(load_rules(dir, false), Rules::ExcludeAll));
        assert!(
            matches!(load_rules(dir, true), Rules::None),
            "an empty file at the root must exclude nothing"
        );

        std::fs::write(dir.join(IGNORE_FILE), "# scans\n*.wav\n!Keep.wav\n").unwrap();
        let Rules::Matcher(rules) = load_rules(dir, false) else {
            panic!("a valid file is a matcher");
        };
        assert!(rules.matched(dir.join("Drop.wav"), false).is_ignore());
        assert!(rules.matched(dir.join("Keep.wav"), false).is_whitelist());

        std::fs::write(dir.join(IGNORE_FILE), "[z-a]\n").unwrap();
        assert!(matches!(load_rules(dir, false), Rules::Unusable(_)));

        std::fs::write(
            dir.join(IGNORE_FILE),
            vec![b'#'; MAX_IGNORE_BYTES as usize + 1],
        )
        .unwrap();
        assert!(matches!(load_rules(dir, false), Rules::Unusable(_)));

        std::fs::remove_file(dir.join(IGNORE_FILE)).unwrap();
        std::fs::create_dir(dir.join(IGNORE_FILE)).unwrap();
        assert!(matches!(load_rules(dir, false), Rules::Unusable(_)));
    }

    /// A track under a folder the walk could not read is not missing, so no
    /// file elsewhere may claim it; nor may any track when the walk could not
    /// name what it missed.
    #[test]
    fn a_track_the_walk_could_not_see_is_not_missing() {
        let discovered = std::collections::HashSet::from(["Found/One.wav".to_owned()]);
        let unreadable = vec!["Locked".to_owned()];
        let walked = Walked {
            discovered: &discovered,
            unreadable: &unreadable,
            incomplete: false,
            scope: None,
        };
        assert!(!walked.saw_missing("Found/One.wav"));
        assert!(!walked.saw_missing("Locked/Two.wav"));
        assert!(!walked.saw_missing("Locked"));
        assert!(walked.saw_missing("Lockedness/Three.wav"));
        assert!(walked.saw_missing("Gone/Four.wav"));
        let incomplete = Walked {
            incomplete: true,
            ..walked
        };
        assert!(!incomplete.saw_missing("Gone/Four.wav"));
        // A scoped walk looked in its folders only: a track anywhere else was
        // not looked for, so it was not seen missing.
        let scope = vec!["Gone".to_owned()];
        let scoped = Walked {
            scope: Some(&scope),
            ..walked
        };
        assert!(scoped.saw_missing("Gone/Four.wav"));
        assert!(!scoped.saw_missing("Elsewhere/Five.wav"));
        assert!(!scoped.saw_missing("Goner/Six.wav"));
    }

    /// A scope is a list of folders a walk can trust, or the whole library.
    #[test]
    fn a_scope_is_plain_folders_or_the_whole_library() {
        let folders = |list: &[&str]| list.iter().map(|f| (*f).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            super::normalised_scope(&folders(&["b/c", "a", "a/x/y", "b/c/", "bc"])),
            Some(folders(&["a", "b/c", "bc"])),
            "nested folders are covered by their parent, and a prefix is not a parent"
        );
        for whole in [&[""][..], &["a/../b"], &["/etc"], &["a//b"], &["."], &[]] {
            assert_eq!(super::normalised_scope(&folders(whole)), None, "{whole:?}");
        }
        let many: Vec<String> = (0..=super::MAX_SCOPE_FOLDERS)
            .map(|index| format!("f{index}"))
            .collect();
        assert_eq!(super::normalised_scope(&many), None);
    }
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

    /// Lyrics saved by a tool on a Japanese Windows, in that system's code
    /// page. A strict UTF-8 read dropped the file whole, so the track had no
    /// lyrics at all.
    #[test]
    fn sidecar_lyrics_that_are_not_utf8_are_read_in_their_own_encoding() {
        let temp = tempfile::tempdir().unwrap();
        let text = "[00:01.00]今日はいい天気ですね、一緒に公園を散歩しましょう\n\
                    [00:05.50]湖に光が映って、風が森を通り抜ける音がする";
        let (bytes, _, unmappable) = encoding_rs::SHIFT_JIS.encode(text);
        assert!(!unmappable, "Shift_JIS holds the sample");
        assert!(
            std::str::from_utf8(&bytes).is_err(),
            "the sample is not UTF-8 by accident"
        );
        let japanese = temp.path().join("japanese.lrc");
        std::fs::write(&japanese, &bytes).unwrap();
        assert_eq!(read_sidecar(&japanese).as_deref(), Some(text));

        // A byte-order mark names the encoding and is not part of the text.
        // Kept, it sat in front of the first timestamp.
        let marked = temp.path().join("marked.lrc");
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(b"[00:01]marked");
        std::fs::write(&marked, bytes).unwrap();
        assert_eq!(read_sidecar(&marked).as_deref(), Some("[00:01]marked"));
    }
}
