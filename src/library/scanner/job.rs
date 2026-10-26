//! One scan job from start to finish: the walk, the reads in batches, what
//! each batch applies to the catalogue, and the progress it reports.

use std::{collections::HashSet, path::PathBuf, sync::Arc};

use futures_util::{stream, StreamExt};
use uuid::Uuid;

use super::discovery::{discover_audio, unclaimed, Discovery, Reach, Walked};
use super::extract::{scan_file, Scanned};
use super::{normalised_scope, ScanManager, ScanProgress};
use crate::storage::catalog::{ApplyOutcome, CatalogApply, LibraryRecord};

impl ScanManager {
    pub(super) async fn run(&self, scan_id: Uuid, library: LibraryRecord) -> anyhow::Result<()> {
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
        // other library's root, which none may reach into. Resolved when the
        // walk starts, on the blocking pool with it, so a mount that appeared
        // since the boot counts, one that went does not, and one that hangs
        // holds no async worker.
        let (link_roots, other_roots) = if follow_links && !self.link_roots.is_empty() {
            let others = self
                .db
                .all_libraries()
                .await?
                .into_iter()
                .filter(|other| other.id != library.id)
                .map(|other| other.root_path)
                .collect::<Vec<_>>();
            (Some(self.link_roots.clone()), others)
        } else {
            (None, Vec::new())
        };
        // Read before the walk is chosen, not after it: a scan that died
        // halfway left the request standing, and the run picking up after it
        // has to honour a request it never saw made. A scoped walk is no
        // answer to it — it would record a partial walk as the full one and
        // clear the request — so a pending full scan walks everything.
        let full = self.db.full_scan_requested(library.id).await?;
        if full {
            tracing::info!(library_id = %library.id, "scanning every file, skipping nothing");
        }
        // Only a walk that reads no link is ever scoped: a folder reached
        // through a link is named by its target, and the watcher names the
        // path it saw, so the two could not be told to agree.
        let scope = match self.db.scan_scope(scan_id).await? {
            Some(scope) if !follow_links && !full => normalised_scope(&scope),
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
                let link_roots = link_roots.map(|roots| roots.resolved()).unwrap_or_default();
                let libraries: Vec<PathBuf> = other_roots
                    .iter()
                    .filter_map(|other| std::fs::canonicalize(other).ok())
                    .collect();
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
                                        crate::storage::catalog::track_pid(
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
                                            crate::storage::catalog::track_pid(
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

    pub(super) fn publish(&self, progress: ScanProgress) {
        if let Some(sender) = self.events.get(&progress.scan_id) {
            let _ = sender.send(progress);
        }
    }
}

fn scan_id(progress: &ScanProgress) -> Uuid {
    progress.scan_id
}
