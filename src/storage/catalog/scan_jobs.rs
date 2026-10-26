//! The scan job lifecycle: queueing, starting, progress, finishing or failing,
//! the errors a scan records, and reading a job back.

use serde::Serialize;
use sqlx::{Row, Sqlite, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::authentication::now_ms, storage::database::Database};

use super::parse_uuid;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScanJobRecord {
    pub id: Uuid,
    pub library_id: Uuid,
    pub status: String,
    pub total_files: i64,
    pub processed_files: i64,
    pub added: i64,
    pub updated: i64,
    pub moved: i64,
    pub skipped: i64,
    pub unavailable: i64,
    pub errors: i64,
    /// Entries the walk left out by an exclusion or an ignore file, a folder
    /// counting once whatever it holds (RFC-021 decision 2).
    pub excluded: i64,
    /// Symbolic links the walk did not follow, for whatever reason
    /// (RFC-021 decision 3).
    pub skipped_links: i64,
    pub current_path: Option<String>,
    pub message: Option<String>,
}

impl Database {
    /// Whether any library the user can see is being scanned, and how many
    /// available tracks they can currently reach.
    ///
    /// Both halves of the Subsonic `scanStatus` shape, resolved in one
    /// round trip and scoped to the caller: `count` is what *this* account
    /// can see, not what the instance holds.
    pub async fn scan_progress_for_user(&self, user_id: Uuid) -> Result<(bool, i64), sqlx::Error> {
        let row = sqlx::query(
            "SELECT EXISTS (SELECT 1 FROM scan_job sj \
                       JOIN library_member m ON m.library_id=sj.library_id \
                       WHERE m.user_id=? AND sj.status IN ('queued', 'running')) AS scanning, \
                    (SELECT COUNT(*) FROM track t \
                     JOIN library_member m ON m.library_id=t.library_id \
                     WHERE m.user_id=? AND t.is_available=1) AS scanned",
        )
        .bind(user_id.to_string())
        .bind(user_id.to_string())
        .fetch_one(self.pool())
        .await?;
        Ok((
            row.try_get::<i64, _>("scanning")? != 0,
            row.try_get("scanned")?,
        ))
    }

    pub async fn create_scan_job(
        &self,
        library_id: Uuid,
        requested_by: Option<Uuid>,
        trigger: &str,
    ) -> Result<Uuid, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO scan_job (id, library_id, requested_by, trigger, status, created_at) \
             VALUES (?, ?, ?, ?, 'queued', ?)",
        )
        .bind(id.to_string())
        .bind(library_id.to_string())
        .bind(requested_by.map(|id| id.to_string()))
        .bind(trigger)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    /// A job that walks only `scope`, folders relative to the library root:
    /// what the watcher queues once a library's changes settle and it can
    /// name where they were (RFC-021, what decision 6 left open).
    pub async fn create_scoped_scan_job(
        &self,
        library_id: Uuid,
        trigger: &str,
        scope: &[String],
    ) -> Result<Uuid, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let id = Uuid::new_v4();
        let scope = serde_json::to_string(scope).expect("a list of strings serializes");
        sqlx::query(
            "INSERT INTO scan_job (id, library_id, requested_by, trigger, status, created_at, \
                                   scope_json) \
             VALUES (?, ?, NULL, ?, 'queued', ?, ?)",
        )
        .bind(id.to_string())
        .bind(library_id.to_string())
        .bind(trigger)
        .bind(now_ms())
        .bind(scope)
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    /// The folders a job walks, or `None` for the whole library. A scope that
    /// cannot be read is the whole library: walking more than asked is always
    /// safe, and walking less is not.
    pub async fn scan_scope(&self, scan_id: Uuid) -> Result<Option<Vec<String>>, sqlx::Error> {
        let scope: Option<Option<String>> =
            sqlx::query_scalar("SELECT scope_json FROM scan_job WHERE id = ?")
                .bind(scan_id.to_string())
                .fetch_optional(self.pool())
                .await?;
        Ok(scope
            .flatten()
            .and_then(|scope| serde_json::from_str::<Vec<String>>(&scope).ok()))
    }

    /// Every job still `queued`, oldest first, with its library: what a
    /// previous run committed and never started.
    pub async fn queued_scan_jobs(&self) -> Result<Vec<(Uuid, Uuid)>, sqlx::Error> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, library_id FROM scan_job WHERE status = 'queued' ORDER BY created_at, id",
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|(id, library)| Ok((parse_uuid(id)?, parse_uuid(library)?)))
            .collect()
    }

    /// A queued job, in a transaction the caller holds under the writer gate:
    /// for a change that owes a scan and must not commit without it (RFC-021
    /// decision 5).
    pub(crate) async fn insert_scan_job_in(
        tx: &mut Transaction<'_, Sqlite>,
        library_id: Uuid,
        requested_by: Option<Uuid>,
        trigger: &str,
    ) -> Result<Uuid, sqlx::Error> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO scan_job (id, library_id, requested_by, trigger, status, created_at) \
             VALUES (?, ?, ?, ?, 'queued', ?)",
        )
        .bind(id.to_string())
        .bind(library_id.to_string())
        .bind(requested_by.map(|id| id.to_string()))
        .bind(trigger)
        .bind(now_ms())
        .execute(&mut **tx)
        .await?;
        Ok(id)
    }

    /// Creates a scan job only if the user is entitled to one, testing that
    /// inside the insert rather than before it.
    ///
    /// A caller that checked entitlement with a separate read leaves a window
    /// where a revocation lands between the check and the insert, queuing
    /// work for a library the requester can no longer reach. Making the
    /// `library_member` row a condition of the statement closes it: there is
    /// no moment at which the job exists without the grant that justified it.
    ///
    /// Membership alone is not that grant. A scan walks the owner's files and
    /// competes for the writer gate, so it is reserved to `owner` and
    /// `manager`: a `listener` is entitled to read the catalogue, not to spend
    /// someone else's disk on it. [`crate::storage::database::LibraryRole::may_scan`]
    /// names the same rule for callers that have the row in hand.
    ///
    /// Returns `None` when the user is not, or no longer, entitled. Callers
    /// map that onto the answer a missing library gets, so a refusal never
    /// confirms that the library exists.
    pub async fn create_scan_job_for_user(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        trigger: &str,
    ) -> Result<Option<Uuid>, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let id = Uuid::new_v4();
        let inserted = sqlx::query(
            "INSERT INTO scan_job (id, library_id, requested_by, trigger, status, created_at) \
             SELECT ?, ?, ?, ?, 'queued', ? WHERE EXISTS \
               (SELECT 1 FROM library_member WHERE library_id=? AND user_id=? \
                AND role IN ('owner', 'manager'))",
        )
        .bind(id.to_string())
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .bind(trigger)
        .bind(now_ms())
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .execute(self.pool())
        .await?;
        Ok((inserted.rows_affected() > 0).then_some(id))
    }

    pub async fn start_scan_job(
        &self,
        scan_id: Uuid,
        total: i64,
        full: bool,
    ) -> Result<(), sqlx::Error> {
        self.start_walked_scan_job(scan_id, total, full, 0, 0).await
    }

    /// Marks a job running, with what its walk found: how many files it will
    /// look at, and what it left out (RFC-021 decision 7).
    pub async fn start_walked_scan_job(
        &self,
        scan_id: Uuid,
        total: i64,
        full: bool,
        excluded: i64,
        skipped_links: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let now = now_ms();
        sqlx::query(
            "UPDATE scan_job SET status = 'running', total_files = ?, full_scan = ?, \
             excluded = ?, skipped_links = ?, started_at = ? WHERE id = ?",
        )
        .bind(total)
        .bind(i64::from(full))
        .bind(excluded)
        .bind(skipped_links)
        .bind(now)
        .bind(scan_id.to_string())
        .execute(self.pool())
        .await?;
        sqlx::query(
            "UPDATE library SET last_scan_started_at = ?, updated_at = ? \
             WHERE id = (SELECT library_id FROM scan_job WHERE id = ?)",
        )
        .bind(now)
        .bind(now)
        .bind(scan_id.to_string())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_scan_job(
        &self,
        scan_id: Uuid,
        processed: i64,
        added: i64,
        updated: i64,
        moved: i64,
        skipped: i64,
        errors: i64,
        current_path: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        sqlx::query(
            "UPDATE scan_job SET processed_files = ?, added = ?, updated = ?, moved = ?, \
             skipped = ?, errors = ?, current_path = ? WHERE id = ?",
        )
        .bind(processed)
        .bind(added)
        .bind(updated)
        .bind(moved)
        .bind(skipped)
        .bind(errors)
        .bind(current_path)
        .bind(scan_id.to_string())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn finish_scan_job(
        &self,
        scan_id: Uuid,
        unavailable: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let now = now_ms();
        sqlx::query(
            "UPDATE scan_job SET status = 'completed', unavailable = ?, current_path = NULL, \
             completed_at = ? WHERE id = ?",
        )
        .bind(unavailable)
        .bind(now)
        .bind(scan_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE library SET last_scan_completed_at = ?, updated_at = ? \
             WHERE id = (SELECT library_id FROM scan_job WHERE id = ?)",
        )
        .bind(now)
        .bind(now)
        .bind(scan_id.to_string())
        .execute(&mut *tx)
        .await?;
        // The full-scan request is lifted here and nowhere else, and only by a
        // run that actually read every file. A run that dies halfway leaves it
        // standing, so the next one reads every file again instead of skipping
        // the half it had already rewritten. And a scan that had already begun
        // when the request arrived cannot spend it: it started under the old
        // rules, so `scan_job.full_scan` says what it really did.
        sqlx::query(
            "UPDATE library SET full_scan_requested_at = NULL, updated_at = ? \
             WHERE id = (SELECT library_id FROM scan_job WHERE id = ?) \
               AND full_scan_requested_at IS NOT NULL \
               AND (SELECT full_scan FROM scan_job WHERE id = ?) = 1",
        )
        .bind(now)
        .bind(scan_id.to_string())
        .bind(scan_id.to_string())
        .execute(&mut *tx)
        .await?;
        // The rules this run wrote the catalogue under, recorded at the end
        // and never at the start: a scan that dies halfway would otherwise
        // look complete to the next boot, which would find no mismatch and
        // leave half the catalogue keyed under the old rule for good.
        for (key, value) in [
            ("pid.album", self.pid().album.source()),
            ("pid.track", self.pid().track.source()),
            ("pid.artist", self.pid().artist.source()),
        ] {
            sqlx::query(
                "INSERT INTO server_property (key, value, updated_at) VALUES (?, ?, ?) \
                 ON CONFLICT (key) DO UPDATE SET value = excluded.value, \
                 updated_at = excluded.updated_at",
            )
            .bind(key)
            .bind(value)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn fail_scan_job(&self, scan_id: Uuid, message: &str) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        sqlx::query(
            "UPDATE scan_job SET status = 'failed', message = ?, current_path = NULL, \
             completed_at = ? WHERE id = ?",
        )
        .bind(message)
        .bind(now_ms())
        .bind(scan_id.to_string())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn scan_job_for_user(
        &self,
        user_id: Uuid,
        scan_id: Uuid,
    ) -> Result<Option<ScanJobRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT j.* FROM scan_job j JOIN library_member m ON m.library_id = j.library_id \
             WHERE j.id = ? AND m.user_id = ?",
        )
        .bind(scan_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await?;
        row.map(scan_job_from_row).transpose()
    }

    /// A scan job by its id alone, for the operator's CLI, which acts as the
    /// instance rather than as a member of the library.
    pub async fn scan_job(&self, scan_id: Uuid) -> Result<Option<ScanJobRecord>, sqlx::Error> {
        let row = sqlx::query("SELECT * FROM scan_job WHERE id = ?")
            .bind(scan_id.to_string())
            .fetch_optional(self.pool())
            .await?;
        row.map(scan_job_from_row).transpose()
    }

    pub async fn add_scan_error(
        &self,
        scan_id: Uuid,
        path: &str,
        message: &str,
    ) -> Result<(), sqlx::Error> {
        self.add_scan_errors(scan_id, &[(path.to_owned(), message.to_owned())])
            .await
    }

    pub async fn add_scan_errors(
        &self,
        scan_id: Uuid,
        errors: &[(String, String)],
    ) -> Result<(), sqlx::Error> {
        if errors.is_empty() {
            return Ok(());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let now = now_ms();
        for (path, message) in errors {
            sqlx::query("INSERT INTO scan_error (scan_id, relative_path, message, created_at) VALUES (?, ?, ?, ?)")
                .bind(scan_id.to_string()).bind(path).bind(message).bind(now)
                .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

fn scan_job_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ScanJobRecord, sqlx::Error> {
    Ok(ScanJobRecord {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        status: row.try_get("status")?,
        total_files: row.try_get("total_files")?,
        processed_files: row.try_get("processed_files")?,
        added: row.try_get("added")?,
        updated: row.try_get("updated")?,
        moved: row.try_get("moved")?,
        skipped: row.try_get("skipped")?,
        unavailable: row.try_get("unavailable")?,
        errors: row.try_get("errors")?,
        excluded: row.try_get("excluded")?,
        skipped_links: row.try_get("skipped_links")?,
        current_path: row.try_get("current_path")?,
        message: row.try_get("message")?,
    })
}
