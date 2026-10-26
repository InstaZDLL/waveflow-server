//! The retention pass: finishing purges past their grace, and purging the
//! tracks missing for longer than the operator keeps them.

use super::purge::{hide_in, purge_in};
use super::*;

/// How long the retention pass waits between two looks at the libraries.
const RETENTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

impl DomainServices {
    /// Deletes for good, in every library not being scanned, the tracks
    /// purged at or before `cutoff` (epoch milliseconds) — which the hourly
    /// pass sets to now less the grace window. Public so a test can run it
    /// rather than wait. Answers how many went.
    pub async fn finish_purges(&self, cutoff: i64) -> Result<i64, ServiceError> {
        let mut total = 0;
        for library in self.db.all_libraries().await? {
            let Some(_scan_lock) = self.scanner.try_lock_library(library.id) else {
                continue;
            };
            total += self
                .delete_purged(library.id, &Selection::PurgedBy(cutoff))
                .await?;
        }
        Ok(total)
    }

    /// The deletion half of a purge, for one library whose scan lock the
    /// caller holds: the selected tracks and everything that hangs off them,
    /// each account told through its journal, the canvases released after.
    async fn delete_purged(
        &self,
        library_id: Uuid,
        selection: &Selection,
    ) -> Result<i64, ServiceError> {
        let purged = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let mut purged = purge_in(&mut tx, library_id, selection).await?;
            if purged.tracks > 0 && !selection.purged() {
                audit(
                    &mut tx,
                    None,
                    "library.tracks_purged",
                    library_id,
                    serde_json::json!({ "tracks": purged.tracks, "retention": true }),
                )
                .await?;
            }
            let receipts = self
                .journal_removed(&_writer, &mut tx, &std::mem::take(&mut purged.removed))
                .await?;
            tx.commit().await?;
            for (user_id, receipt) in receipts {
                self.sync.publish(user_id, receipt);
            }
            purged
        };
        for hash in &purged.canvases {
            self.release_canvas_blob(hash).await;
        }
        Ok(purged.tracks)
    }

    /// One retention pass: in every library not being scanned, the tracks
    /// unavailable since before `cutoff` (epoch milliseconds). Public so a test
    /// can run it rather than wait an hour. Answers how many went.
    ///
    /// A purge like any other: with a grace window it takes them out of sight,
    /// to be deleted once the window has passed, so a retention set too short
    /// can still be taken back.
    pub async fn purge_expired_unavailable_tracks(&self, cutoff: i64) -> Result<i64, ServiceError> {
        let mut total = 0;
        for library in self.db.all_libraries().await? {
            let Some(_scan_lock) = self.scanner.try_lock_library(library.id) else {
                continue;
            };
            if self.purge_grace.is_zero() {
                total += self
                    .delete_purged(library.id, &Selection::OlderThan(cutoff))
                    .await?;
                continue;
            }
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let hidden =
                hide_in(&mut tx, library.id, &Selection::OlderThan(cutoff), now_ms()).await?;
            if hidden > 0 {
                audit(
                    &mut tx,
                    None,
                    "library.tracks_purged",
                    library.id,
                    serde_json::json!({ "tracks": hidden, "retention": true }),
                )
                .await?;
            }
            tx.commit().await?;
            total += hidden;
        }
        Ok(total)
    }

    /// The hourly pass over missing tracks, once at boot and then every hour:
    /// the retention, when the operator set one
    /// (`WAVEFLOW_UNAVAILABLE_RETENTION_DAYS`), then the deletion of every
    /// purge whose grace window (`WAVEFLOW_PURGE_GRACE_HOURS`) has passed.
    ///
    /// Runs whatever the settings: with no grace window, the purges a former
    /// window left behind are due at once, and nothing else would delete them.
    pub fn spawn_unavailable_purge(&self) {
        let retention = self.unavailable_retention;
        let grace = self.purge_grace;
        let services = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(RETENTION_INTERVAL);
            loop {
                ticker.tick().await;
                if let Some(retention) = retention {
                    let cutoff = now_ms()
                        .saturating_sub(i64::try_from(retention.as_millis()).unwrap_or(i64::MAX));
                    match services.purge_expired_unavailable_tracks(cutoff).await {
                        Ok(0) => {}
                        Ok(purged) => {
                            tracing::info!(
                                purged,
                                "missing tracks past their retention were purged"
                            )
                        }
                        Err(error) => {
                            tracing::warn!(%error, "could not purge missing tracks past their retention")
                        }
                    }
                }
                let cutoff =
                    now_ms().saturating_sub(i64::try_from(grace.as_millis()).unwrap_or(i64::MAX));
                match services.finish_purges(cutoff).await {
                    Ok(0) => {}
                    Ok(deleted) => {
                        tracing::info!(
                            deleted,
                            "purged tracks past their grace window were deleted"
                        )
                    }
                    Err(error) => {
                        tracing::warn!(%error, "could not delete purged tracks past their grace window")
                    }
                }
            }
        });
    }
}
