//! Libraries as the catalogue reads them: which an account reaches, how each
//! is walked and watched, the instance-wide settings, and full-scan requests.

use std::{path::PathBuf, str::FromStr};

use serde::Serialize;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{auth::authentication::now_ms, storage::database::Database};

use super::parse_uuid;

#[derive(Debug, Clone)]
pub struct LibraryRecord {
    pub id: Uuid,
    pub name: String,
    pub root_path: PathBuf,
}

/// A library and the one setting that changes how it is walked.
#[derive(Debug, Clone)]
pub struct LibraryScanSettings {
    pub library: LibraryRecord,
    /// Follow symbolic links that stay inside the root (RFC-021 decision 3).
    pub follow_links: bool,
    /// Whether the watcher observes this library (RFC-021 decision 6, and the
    /// per-library switch it left open).
    pub watch: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryAccess {
    pub id: Uuid,
    pub name: String,
    pub visibility: crate::storage::database::LibraryVisibility,
    pub role: crate::storage::database::LibraryRole,
    pub last_scan_started_at: Option<i64>,
    pub last_scan_completed_at: Option<i64>,
    /// Whether the library takes files at all. The operator's decision, set
    /// through the CLI and never through the API, and shown to every member:
    /// the role says who may upload, this says whether anyone can. A client
    /// that knows it offers an upload only where the server would take one,
    /// instead of hashing a whole file to be told `library_closed`.
    pub accepts_uploads: bool,
    /// Whether the library takes canvases. A door of its own and not the upload
    /// one (RFC-009 decision 5): a read-only library that refuses to grow in
    /// audio may still take a few hundred kilobytes of loop, so a client that
    /// read `accepts_uploads` for both would hide the canvas exactly where the
    /// flag was split to allow it.
    pub accepts_canvas: bool,
    /// Whether the walk follows symbolic links that stay inside the library
    /// (RFC-021 decision 3). An administrator's setting, shown to every
    /// member because it explains what the library holds.
    pub follow_links: bool,
}

impl Database {
    pub async fn libraries_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<LibraryAccess>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT l.id, l.name, l.visibility, m.role, l.last_scan_started_at, \
                    l.last_scan_completed_at, l.accepts_uploads, l.accepts_canvas, \
                    l.follow_links \
             FROM library l JOIN library_member m ON m.library_id=l.id \
             WHERE m.user_id=? ORDER BY l.name COLLATE NOCASE, l.id",
        )
        .bind(user_id.to_string())
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(LibraryAccess {
                    id: parse_uuid(row.try_get("id")?)?,
                    name: row.try_get("name")?,
                    visibility: crate::storage::database::LibraryVisibility::from_str(
                        row.try_get::<&str, _>("visibility")?,
                    )
                    .map_err(|error| sqlx::Error::Decode(error.into()))?,
                    role: crate::storage::database::LibraryRole::from_str(
                        row.try_get::<&str, _>("role")?,
                    )
                    .map_err(|error| sqlx::Error::Decode(error.into()))?,
                    last_scan_started_at: row.try_get("last_scan_started_at")?,
                    last_scan_completed_at: row.try_get("last_scan_completed_at")?,
                    accepts_uploads: row.try_get::<i64, _>("accepts_uploads")? != 0,
                    accepts_canvas: row.try_get::<i64, _>("accepts_canvas")? != 0,
                    follow_links: row.try_get::<i64, _>("follow_links")? != 0,
                })
            })
            .collect()
    }

    pub async fn all_libraries(&self) -> Result<Vec<LibraryRecord>, sqlx::Error> {
        let rows = sqlx::query("SELECT id, name, root_path FROM library ORDER BY created_at")
            .fetch_all(self.pool())
            .await?;
        rows.into_iter().map(library_from_row).collect()
    }

    /// One library as a scan needs it, read when the scan starts rather than
    /// when it was queued: its root may have moved and its links setting
    /// changed in between, and it may be gone (RFC-021 decision 5).
    pub async fn library_scan_settings(
        &self,
        library_id: Uuid,
    ) -> Result<Option<LibraryScanSettings>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, name, root_path, follow_links, watch FROM library WHERE id = ?",
        )
        .bind(library_id.to_string())
        .fetch_optional(self.pool())
        .await?;
        row.map(|row| {
            let follow_links = row.try_get::<i64, _>("follow_links")? != 0;
            let watch = row.try_get::<i64, _>("watch")? != 0;
            Ok(LibraryScanSettings {
                library: library_from_row(row)?,
                follow_links,
                watch,
            })
        })
        .transpose()
    }

    /// The libraries the watcher observes: those whose `watch` switch is on,
    /// and the links setting each is walked with, which decides whether a
    /// watched change may be scanned folder by folder.
    pub async fn watched_libraries(&self) -> Result<Vec<(LibraryRecord, bool)>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, name, root_path, follow_links FROM library WHERE watch = 1 \
             ORDER BY created_at",
        )
        .fetch_all(self.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                let follow_links = row.try_get::<i64, _>("follow_links")? != 0;
                Ok((library_from_row(row)?, follow_links))
            })
            .collect()
    }

    /// Reads one instance-wide setting, or `None` when it was never written.
    ///
    /// The store exists so the server can notice that it is configured
    /// differently from the run that built the catalogue. Anything a scan
    /// derives from configuration belongs here: on a mismatch the catalogue is
    /// stale in a way no file timestamp can reveal.
    pub async fn server_property(&self, key: &str) -> Result<Option<String>, sqlx::Error> {
        sqlx::query_scalar("SELECT value FROM server_property WHERE key = ?")
            .bind(key)
            .fetch_optional(self.pool())
            .await
    }

    pub async fn set_server_property(&self, key: &str, value: &str) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        sqlx::query(
            "INSERT INTO server_property (key, value, updated_at) VALUES (?, ?, ?) \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value, \
             updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(value)
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Asks every library to rescan every file, skipping nothing.
    ///
    /// Returns how many libraries were marked. One already carrying the request
    /// keeps its original timestamp: the request is a state, not a counter, and
    /// re-stamping it would say a later run asked for something the earlier one
    /// has not yet delivered.
    pub async fn request_full_scan_everywhere(&self) -> Result<u64, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE library SET full_scan_requested_at = ?, updated_at = ? \
             WHERE full_scan_requested_at IS NULL",
        )
        .bind(now_ms())
        .bind(now_ms())
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn request_full_scan(&self, library_id: Uuid) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        sqlx::query(
            "UPDATE library SET full_scan_requested_at = ?, updated_at = ? \
             WHERE id = ? AND full_scan_requested_at IS NULL",
        )
        .bind(now_ms())
        .bind(now_ms())
        .bind(library_id.to_string())
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Whether the next scan of this library has to read every file.
    ///
    /// Read at the start of every run rather than passed in by the caller: a
    /// scan that was interrupted left the request standing, and the run that
    /// picks up after it has to honour the request it never saw made.
    pub async fn full_scan_requested(&self, library_id: Uuid) -> Result<bool, sqlx::Error> {
        let requested: Option<Option<i64>> =
            sqlx::query_scalar("SELECT full_scan_requested_at FROM library WHERE id = ?")
                .bind(library_id.to_string())
                .fetch_optional(self.pool())
                .await?;
        Ok(matches!(requested, Some(Some(_))))
    }

    pub async fn library_for_user(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<Option<LibraryRecord>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT l.id, l.name, l.root_path FROM library l \
             JOIN library_member m ON m.library_id = l.id \
             WHERE l.id = ? AND m.user_id = ?",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await?;
        row.map(library_from_row).transpose()
    }
}

fn library_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LibraryRecord, sqlx::Error> {
    Ok(LibraryRecord {
        id: parse_uuid(row.try_get("id")?)?,
        name: row.try_get("name")?,
        root_path: PathBuf::from(row.try_get::<String, _>("root_path")?),
    })
}
