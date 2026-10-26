//! Which tracks a scan found: the facts an incremental scan compares before
//! reading a file, sidecar signatures, the seen stamps, and the sweep that
//! marks the unseen unavailable.

use sqlx::Row;
use uuid::Uuid;

use crate::{auth::authentication::now_ms, storage::database::Database};

use super::{parse_uuid, record_library_event, LibraryChange};

/// What an incremental scan compares before reading a file (RFC-018
/// decision 1): enough to skip it without opening it.
#[derive(Debug, Clone)]
pub struct SkipFacts {
    pub id: Uuid,
    pub file_size: i64,
    pub modified_at: i64,
    pub available: bool,
    /// The relocation hint is present; a row from before the column still
    /// needs its one extraction.
    pub has_pid: bool,
    /// `None` until a scan has computed it, which never matches.
    pub sidecar_signature: Option<String>,
}

impl Database {
    pub async fn mark_track_seen(&self, track_id: Uuid, scan_id: Uuid) -> Result<(), sqlx::Error> {
        self.mark_tracks_seen(&[track_id], scan_id).await
    }

    /// Every track of the library by its path, with what the pre-check compares.
    /// Read once per scan: a row per track, small, and one query instead of one
    /// per file.
    pub async fn track_skip_facts(
        &self,
        library_id: Uuid,
    ) -> Result<std::collections::HashMap<String, SkipFacts>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, relative_path, file_size, file_modified_at, is_available,                     pid IS NOT NULL AS has_pid, sidecar_signature              FROM track WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .fetch_all(self.pool())
        .await?;
        let mut facts = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let id: String = row.try_get("id")?;
            facts.insert(
                row.try_get("relative_path")?,
                SkipFacts {
                    id: Uuid::parse_str(&id).map_err(|error| sqlx::Error::Decode(error.into()))?,
                    file_size: row.try_get("file_size")?,
                    modified_at: row.try_get("file_modified_at")?,
                    available: row.try_get::<i64, _>("is_available")? != 0,
                    has_pid: row.try_get::<i64, _>("has_pid")? != 0,
                    sidecar_signature: row.try_get("sidecar_signature")?,
                },
            );
        }
        Ok(facts)
    }

    /// Records the sidecar signature of files a scan read, by path, so the next
    /// scan can skip them (RFC-018 decision 1).
    pub async fn set_sidecar_signatures(
        &self,
        library_id: Uuid,
        signatures: &[(String, String)],
    ) -> Result<(), sqlx::Error> {
        if signatures.is_empty() {
            return Ok(());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        for (relative_path, signature) in signatures {
            sqlx::query(
                "UPDATE track SET sidecar_signature = ? WHERE library_id = ? AND relative_path = ?",
            )
            .bind(signature)
            .bind(library_id.to_string())
            .bind(relative_path)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    /// Stamps as seen by this scan the tracks under folders the scan could not
    /// read, without touching their availability, so the pass that marks the
    /// unseen missing leaves them as they are (RFC-018 decision 3).
    pub async fn mark_tracks_under_seen(
        &self,
        library_id: Uuid,
        folders: &[String],
        scan_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        if folders.is_empty() {
            return Ok(());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        for folder in folders {
            // A prefix compared as text rather than with LIKE: a folder name
            // holding `%` or `_` must not widen the match.
            sqlx::query(
                "UPDATE track SET last_seen_scan_id = ?                  WHERE library_id = ?                    AND (relative_path = ? OR substr(relative_path, 1, length(?) + 1) = ? || '/')",
            )
            .bind(scan_id.to_string())
            .bind(library_id.to_string())
            .bind(folder)
            .bind(folder)
            .bind(folder)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }

    pub async fn mark_tracks_seen(
        &self,
        track_ids: &[Uuid],
        scan_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        if track_ids.is_empty() {
            return Ok(());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let now = now_ms();
        for track_id in track_ids {
            sqlx::query(
                // A file found again takes back a purge still within its
                // grace window: the track is there, so nothing is to go.
                "UPDATE track SET is_available = 1, purged_at = NULL, last_seen_scan_id = ?, \
                 updated_at = ? WHERE id = ?",
            )
            .bind(scan_id.to_string())
            .bind(now)
            .bind(track_id.to_string())
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn mark_unseen_unavailable(
        &self,
        library_id: Uuid,
        scan_id: Uuid,
    ) -> Result<i64, sqlx::Error> {
        self.mark_unseen_unavailable_within(library_id, scan_id, None)
            .await
    }

    /// [`Self::mark_unseen_unavailable`], for a scan that walked only
    /// `scope`: a track outside the folders it walked was not looked for, so
    /// it cannot have been missed (RFC-018 decision 3, which a scoped scan
    /// keeps).
    pub async fn mark_unseen_unavailable_within(
        &self,
        library_id: Uuid,
        scan_id: Uuid,
        scope: Option<&[String]>,
    ) -> Result<i64, sqlx::Error> {
        let scope = scope
            .map(|folders| serde_json::to_string(folders).expect("a list of strings serializes"));
        let _writer = self.writer_guard().await;
        let now = now_ms();
        let mut tx = self.begin_write(&_writer).await?;
        // `RETURNING` rather than a count: the feed needs to name what
        // disappeared, and a number cannot.
        // Two ways a track can be one this scan did not find, and they are not
        // the same fact.
        //
        // A track another scan stamped is the ordinary case: the walk covered
        // the library and this one was not in it, so its file is gone.
        //
        // A track no scan ever stamped is the other, and it now has a meaning —
        // `apply_catalog_track_in_transaction` writes NULL for a row that did
        // not come from a scan, which is what a received file is. Sweeping it
        // for not having been walked over would mark a file that is on disk as
        // gone, and announce a deletion to every client seconds after
        // announcing its arrival: a scan running when the file lands started
        // its walk before the file existed.
        //
        // So an unstamped track is only swept when it predates this scan, which
        // is the case where the walk genuinely should have found it. The first
        // scan to walk past a received file stamps it, and it rejoins the
        // ordinary case for good.
        let gone: Vec<String> = sqlx::query_scalar(
            "UPDATE track SET is_available = 0, unavailable_since = ?, updated_at = ? \
             WHERE library_id = ? AND is_available = 1 \
               AND ((last_seen_scan_id IS NOT NULL AND last_seen_scan_id <> ?) \
                 OR (last_seen_scan_id IS NULL \
                     AND created_at < COALESCE( \
                       (SELECT started_at FROM scan_job WHERE id = ?), ?))) \
               AND (? IS NULL OR EXISTS (SELECT 1 FROM json_each(?) folder \
                     WHERE folder.value = '' OR track.relative_path = folder.value \
                        OR substr(track.relative_path, 1, length(folder.value) + 1) \
                           = folder.value || '/')) \
             RETURNING id",
        )
        .bind(now)
        .bind(now)
        .bind(library_id.to_string())
        .bind(scan_id.to_string())
        .bind(scan_id.to_string())
        .bind(now)
        .bind(scope.as_deref())
        .bind(scope.as_deref())
        .fetch_all(&mut *tx)
        .await?;
        for id in &gone {
            record_library_event(
                &mut tx,
                LibraryChange {
                    library_id,
                    entity_type: "track",
                    entity_id: parse_uuid(id.clone())?,
                    action: "delete",
                    payload: serde_json::json!({}),
                    changed_at: now,
                    // A scan noticing a file is gone is nobody's request.
                    origin_device_id: None,
                },
            )
            .await?;
        }
        tx.commit().await?;
        Ok(gone.len() as i64)
    }
}
