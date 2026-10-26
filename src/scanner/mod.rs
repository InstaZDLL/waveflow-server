//! Server-authoritative filesystem scanner with SSE-ready progress events.

mod discovery;
mod extract;
mod job;

pub use discovery::IGNORE_FILE;
pub(crate) use discovery::{normalised_scope, MAX_SCOPE_FOLDERS};
pub(crate) use extract::extract_file;

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use dashmap::DashMap;
use tokio::sync::{broadcast, watch, Mutex};
use uuid::Uuid;

use crate::{
    catalog::{CatalogTrackInput, LibraryRecord, ScanJobRecord},
    database::Database,
};

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
}
