//! The store sweep: orphaned blobs and stale working files.

use super::*;

/// How long a working file may sit before the sweep treats it as abandoned.
///
/// A `.part` or `.silent` belongs to one in-flight request, which the router's
/// own timeout already bounds in seconds. An hour is far past anything a live
/// placement can still be doing, and it is the margin that keeps this from
/// racing a request whose clock and this one's disagree.
const STAGING_GRACE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How often the store is walked.
///
/// Orphans are made by failures, not by use, so this is a repair pass rather
/// than housekeeping: daily costs one directory listing and finds nothing on a
/// server that has not crashed.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

impl DomainServices {
    /// Walks the canvas store and collects what a failure left in it.
    ///
    /// RFC-009 decision 6 says when a blob stops being referenced and by which
    /// transaction; it does not say who picks up the bytes a crash left behind.
    /// This does. Two sources, both named there: a failure between the commit
    /// and the unlink, and a placement that died between writing its file and
    /// inserting its row.
    ///
    /// The shape is `spawn_upload_sweeper`'s, deliberately — boot first, then a
    /// pass per interval.
    pub fn spawn_canvas_sweeper(&self) {
        let services = self.clone();
        tokio::spawn(async move {
            services.sweep_canvas_now().await;
            let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.sweep_canvas_now().await;
            }
        });
    }

    async fn sweep_canvas_now(&self) {
        match self.sweep_canvas_store().await {
            Ok(swept) if swept == CanvasSweep::default() => {}
            Ok(swept) => tracing::info!(
                blobs = swept.blobs_removed,
                staging = swept.staging_removed,
                dead_links = swept.dead_links,
                unknown = swept.unknown,
                "canvas store swept"
            ),
            Err(error) => tracing::warn!(%error, "could not sweep the canvas store"),
        }
    }

    /// One pass. Public so a test can run it rather than wait a day for it.
    ///
    /// **Nothing is removed that this does not recognise.** The store lives
    /// under the operator's `data/`, and a sweep that deletes what it cannot
    /// name is a sweep nobody should run. A file is only a candidate if it is
    /// `<64 hex>.<format>` for a format the whitelist admits, or a working name
    /// this module writes; everything else is counted and left where it is.
    pub async fn sweep_canvas_store(&self) -> Result<CanvasSweep, ServiceError> {
        let mut swept = CanvasSweep::default();
        let mut entries = match tokio::fs::read_dir(&self.canvas_dir).await {
            Ok(entries) => entries,
            // No store yet is not a failure: no library has taken a canvas.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(swept);
            }
            Err(error) => {
                tracing::warn!(%error, "cannot read the canvas store");
                return Err(ServiceError::Unavailable);
            }
        };

        let mut seen = std::collections::HashSet::new();
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            tracing::warn!(%error, "cannot walk the canvas store");
            ServiceError::Unavailable
        })? {
            let name = entry.file_name().to_string_lossy().into_owned();
            match classify_store_entry(&name) {
                StoreEntry::Blob { hash, .. } => {
                    seen.insert(hash.clone());
                    if self.discard_orphan_blob(&hash, &entry.path()).await {
                        swept.blobs_removed += 1;
                    }
                }
                StoreEntry::Working => {
                    if self.discard_stale_working_file(&entry.path()).await {
                        swept.staging_removed += 1;
                    }
                }
                StoreEntry::Unknown => {
                    tracing::warn!(file = %name, "unrecognised file left in the canvas store");
                    swept.unknown += 1;
                }
            }
        }

        // The other direction, which is reported and never repaired: a row is
        // somebody's canvas, and deleting it to tidy the store would answer a
        // dead link by making it a missing one.
        let named: Vec<String> = sqlx::query_scalar("SELECT hash FROM canvas")
            .fetch_all(self.db.pool())
            .await?;
        for hash in named {
            if !seen.contains(&hash) {
                tracing::warn!(%hash, "a canvas row names a file the store does not hold");
                swept.dead_links += 1;
            }
        }
        Ok(swept)
    }

    /// Removes a blob nothing references, under the lock that makes the
    /// question answerable.
    ///
    /// Without the lock this is the sweep that creates the damage it exists to
    /// repair: a placement writes its file, and between that write and its row
    /// the sweep sees bytes no row names and takes them — leaving the placement
    /// to commit a row whose file is gone. Holding the blob's lock makes the
    /// two mutually exclusive, and the count is re-read inside it rather than
    /// trusted from the walk.
    ///
    /// **Both halves are covered now.** Removing the count fails a test — a
    /// referenced blob is taken. Removing the lock fails
    /// `tests::a_placement_that_commits_while_the_sweep_waits_keeps_its_bytes`,
    /// which does not race for the interleaving but arranges it: the test holds
    /// this lock, which is what a placement holds between its bytes and its
    /// row, and the sweep has to wait. That needed a unit test rather than an
    /// integration one — the lock is private, and reaching it is the whole
    /// trick.
    async fn discard_orphan_blob(&self, hash: &str, path: &std::path::Path) -> bool {
        let removed = {
            let lock = self.canvas_lock(hash);
            let _held = lock.lock().await;
            let referenced: Result<i64, _> =
                sqlx::query_scalar("SELECT COUNT(*) FROM canvas WHERE hash=?")
                    .bind(hash)
                    .fetch_one(self.db.pool())
                    .await;
            match referenced {
                Ok(0) => match tokio::fs::remove_file(path).await {
                    Ok(()) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                    Err(error) => {
                        tracing::warn!(%error, "cannot remove an orphaned canvas");
                        false
                    }
                },
                Ok(_) => false,
                Err(error) => {
                    tracing::warn!(%error, "cannot decide whether a canvas is referenced");
                    false
                }
            }
        };
        self.release_canvas_lock(hash);
        removed
    }

    /// Removes a working file older than any request could still be using.
    ///
    /// Age rather than a lock, because these names belong to one call and
    /// nothing else can look them up: `<uuid>.part` and `<uuid>.silent` are
    /// known only to the placement that made them, so there is no lock to take
    /// and no row to consult. The grace period is what stands in for both.
    async fn discard_stale_working_file(&self, path: &std::path::Path) -> bool {
        let Ok(metadata) = tokio::fs::metadata(path).await else {
            return false;
        };
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > STAGING_GRACE);
        if !stale {
            return false;
        }
        match tokio::fs::remove_file(path).await {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                tracing::warn!(%error, "cannot remove an abandoned canvas working file");
                false
            }
        }
    }
}

/// What a name in the store is, if it is anything this module wrote.
enum StoreEntry {
    Blob { hash: String },
    Working,
    Unknown,
}

/// Reads a file name without touching the disk.
///
/// Deliberately strict. A blob is sixty-four lowercase hex characters and a
/// format the whitelist admits — the same two rules that built the name — and
/// anything that does not match both is not something this sweep will delete.
fn classify_store_entry(name: &str) -> StoreEntry {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return StoreEntry::Unknown;
    };
    if ACCEPTED_FORMATS.contains(&extension)
        && stem.len() == 64
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return StoreEntry::Blob {
            hash: stem.to_owned(),
        };
    }
    // The two names a placement writes before it knows the final one, and the
    // stem is checked as strictly as a blob's. It is a UUID this module
    // invented — `place_canvas` writes `<uuid>.part`, and the strip renames it
    // to `<uuid>.silent` — so requiring one to parse costs nothing and closes
    // the asymmetry an earlier version of this function had: sixty-four hex
    // demanded of a blob, and any stem at all accepted before `.part`. The
    // operator's `notes.part` is not this module's file.
    if matches!(extension, "part" | "silent") && Uuid::parse_str(stem).is_ok() {
        return StoreEntry::Working;
    }
    StoreEntry::Unknown
}
