//! Reading and changing a library's settings, and deleting a library.

use super::purge::Removed;
use super::*;

/// A root as creating a library accepts one: an existing directory, not
/// itself a link, stored canonical.
async fn validated_root(path: &str) -> Result<PathBuf, ServiceError> {
    let path = PathBuf::from(path);
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| ServiceError::Invalid)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ServiceError::Invalid);
    }
    tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| ServiceError::Invalid)
}

impl DomainServices {
    /// The settings an administrator manages, the root included.
    pub async fn library_settings(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
    ) -> Result<LibrarySettings, ServiceError> {
        if !actor.admin_credential {
            return Err(ServiceError::Forbidden);
        }
        self.require_admin(actor.id).await?;
        let settings = self
            .db
            .library_scan_settings(library_id)
            .await?
            .ok_or(ServiceError::NotFound)?;
        Ok(LibrarySettings {
            id: settings.library.id,
            name: settings.library.name,
            path: settings.library.root_path.to_string_lossy().into_owned(),
            follow_links: settings.follow_links,
            watch: settings.watch,
        })
    }

    /// Renames a library, moves its root, or changes whether its walk follows
    /// links (RFC-021 decision 5).
    ///
    /// A new name is the owner's or an administrator's to give; the path and
    /// the links setting are an administrator's. A new root, or a changed links
    /// setting, queues a scan; since tracks are matched by their path relative
    /// to the root, a root moved with its tree intact keeps every identifier.
    /// Either refuses while the library is being scanned.
    pub async fn update_library(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        update: LibraryUpdate,
    ) -> Result<LibraryUpdated, ServiceError> {
        if update.name.is_none() && !update.is_administrative() {
            return Err(ServiceError::Invalid);
        }
        let standing = {
            let mut connection = self.db.pool().acquire().await?;
            standing_on(&mut connection, actor, library_id).await?
        };
        let allowed = if update.is_administrative() {
            standing == Standing::Admin
        } else {
            matches!(standing, Standing::Admin | Standing::Owner)
        };
        if !allowed {
            return Err(ServiceError::Forbidden);
        }
        let name = match update.name.as_deref().map(str::trim) {
            Some(name) if name.is_empty() || name.chars().count() > 160 => {
                return Err(ServiceError::Invalid)
            }
            other => other.map(str::to_owned),
        };
        let root = match update.path.as_deref() {
            Some(path) => Some(validated_root(path).await?),
            None => None,
        };
        // Only what changes how the library is walked has to wait for no scan;
        // a new name is a label.
        let _scan_lock = if update.is_administrative() {
            Some(
                self.scanner
                    .try_lock_library(library_id)
                    .ok_or(ServiceError::Conflict)?,
            )
        } else {
            None
        };
        let before = self
            .db
            .library_scan_settings(library_id)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let moved = root
            .as_ref()
            .is_some_and(|root| *root != before.library.root_path);
        let relinked = update
            .follow_links
            .is_some_and(|follow| follow != before.follow_links);
        let rewatched = update.watch.is_some_and(|watch| watch != before.watch);
        let scan_id = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let written = sqlx::query(
                "UPDATE library SET name = COALESCE(?, name), \
                   root_path = COALESCE(?, root_path), \
                   follow_links = COALESCE(?, follow_links), \
                   watch = COALESCE(?, watch), updated_at = ? \
                 WHERE id = ?",
            )
            .bind(name.as_deref())
            .bind(
                root.as_ref()
                    .map(|root| root.to_string_lossy().into_owned()),
            )
            .bind(update.follow_links.map(i64::from))
            .bind(update.watch.map(i64::from))
            .bind(now_ms())
            .bind(library_id.to_string())
            .execute(&mut *tx)
            .await;
            match written {
                Ok(result) if result.rows_affected() == 0 => return Err(ServiceError::NotFound),
                Ok(_) => {}
                // Another library already has that root.
                Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                    return Err(ServiceError::Conflict)
                }
                Err(error) => return Err(error.into()),
            }
            if let Some(name) = name.as_deref().filter(|name| *name != before.library.name) {
                audit(
                    &mut tx,
                    Some(actor.id),
                    "library.renamed",
                    library_id,
                    serde_json::json!({ "name": name }),
                )
                .await?;
            }
            if moved {
                audit(
                    &mut tx,
                    Some(actor.id),
                    "library.moved",
                    library_id,
                    serde_json::json!({}),
                )
                .await?;
            }
            if relinked {
                audit(
                    &mut tx,
                    Some(actor.id),
                    "library.follow_links_set",
                    library_id,
                    serde_json::json!({ "follow_links": update.follow_links }),
                )
                .await?;
            }
            if rewatched {
                audit(
                    &mut tx,
                    Some(actor.id),
                    "library.watch_set",
                    library_id,
                    serde_json::json!({ "watch": update.watch }),
                )
                .await?;
            }
            // The scan a new root or links setting owes is queued in the same
            // transaction: a change committed without its job would leave the
            // catalogue on the old walk until some other scan came along —
            // and with no interval set, none would.
            let scan_id = if moved || relinked {
                Some(
                    Database::insert_scan_job_in(
                        &mut tx,
                        library_id,
                        Some(actor.id),
                        if moved { "library_moved" } else { "manual" },
                    )
                    .await?,
                )
            } else {
                None
            };
            tx.commit().await?;
            scan_id
        };
        drop(_scan_lock);
        // The watcher reads which libraries to watch, and how, again.
        if moved || rewatched || relinked {
            self.scanner.libraries_changed();
        }
        let Some(scan_id) = scan_id else {
            return Ok(LibraryUpdated { scan_id: None });
        };
        // Started from what this call already knows, with nothing that can
        // fail between the commit and the start: the scan reads the library
        // again when it runs anyway. A process that stops right here leaves
        // the job queued, and the next boot runs it (`ScanManager::boot`).
        self.scanner.spawn(
            scan_id,
            crate::storage::catalog::LibraryRecord {
                id: library_id,
                name: name.unwrap_or(before.library.name),
                root_path: root.unwrap_or(before.library.root_path),
            },
        );
        Ok(LibraryUpdated {
            scan_id: Some(scan_id),
        })
    }

    /// Deletes a library from the catalogue, and nothing from the disk
    /// (RFC-021 decision 5). An administrator's, confirmed by the library's
    /// current name, and refused while the library is being scanned.
    ///
    /// Every track goes, with everything that cascades from it for every
    /// member — playlist entries, favourites, plays, corrections — and every
    /// album, artist and genre, with their favourites and search rows; then the
    /// library row, which takes the rest with it. Received files stay where
    /// they are; the staging files of transfers still open are removed after
    /// the commit.
    pub async fn delete_library(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        confirm: &str,
    ) -> Result<LibraryDeleted, ServiceError> {
        let standing = {
            let mut connection = self.db.pool().acquire().await?;
            standing_on(&mut connection, actor, library_id).await?
        };
        if standing != Standing::Admin {
            return Err(ServiceError::Forbidden);
        }
        let settings = self
            .db
            .library_scan_settings(library_id)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if confirm != settings.library.name {
            return Err(ServiceError::Invalid);
        }
        let _scan_lock = self
            .scanner
            .try_lock_library(library_id)
            .ok_or(ServiceError::Conflict)?;
        let library = library_id.to_string();
        let deleted;
        let staging: Vec<String>;
        let canvases: Vec<String>;
        let receipts: Vec<(Uuid, MutationReceipt)>;
        {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let (tracks, albums, artists): (i64, i64, i64) = sqlx::query_as(
                "SELECT (SELECT COUNT(*) FROM track WHERE library_id = ?1), \
                        (SELECT COUNT(*) FROM album WHERE library_id = ?1), \
                        (SELECT COUNT(*) FROM artist WHERE library_id = ?1)",
            )
            .bind(&library)
            .fetch_one(&mut *tx)
            .await?;
            staging = sqlx::query_scalar("SELECT id FROM upload_session WHERE library_id = ?")
                .bind(&library)
                .fetch_all(&mut *tx)
                .await?;
            // The loops the library's tracks carry: their links cascade, the
            // blobs do not, and another library may share them.
            canvases = sqlx::query_scalar(
                "SELECT DISTINCT canvas_hash FROM track_canvas WHERE library_id = ?",
            )
            .bind(&library)
            .fetch_all(&mut *tx)
            .await?;
            // What every account loses, read as it goes, so that each one's
            // journal can say so (RFC-021, what was still open).
            let mut removed = Removed {
                playlists: sqlx::query_scalar(
                    "SELECT DISTINCT pt.playlist_id FROM playlist_track pt \
                     JOIN track t ON t.id = pt.track_id WHERE t.library_id = ?",
                )
                .bind(&library)
                .fetch_all(&mut *tx)
                .await?,
                ..Removed::default()
            };
            removed.stars = sqlx::query_as(
                "DELETE FROM user_star WHERE \
                   (entity_type = 'track' AND entity_id IN (SELECT id FROM track WHERE library_id = ?1)) \
                   OR (entity_type = 'album' AND entity_id IN (SELECT id FROM album WHERE library_id = ?1)) \
                   OR (entity_type = 'artist' AND entity_id IN (SELECT id FROM artist WHERE library_id = ?1)) \
                 RETURNING user_id, entity_type, entity_id",
            )
            .bind(&library)
            .fetch_all(&mut *tx)
            .await?;
            removed.ratings = sqlx::query_as(
                "DELETE FROM user_rating WHERE \
                   (entity_type = 'track' AND entity_id IN (SELECT id FROM track WHERE library_id = ?1)) \
                   OR (entity_type = 'album' AND entity_id IN (SELECT id FROM album WHERE library_id = ?1)) \
                   OR (entity_type = 'artist' AND entity_id IN (SELECT id FROM artist WHERE library_id = ?1)) \
                 RETURNING user_id, entity_type, entity_id",
            )
            .bind(&library)
            .fetch_all(&mut *tx)
            .await?;
            for statement in [
                // Virtual tables: nothing cascades into them.
                "DELETE FROM track_fts WHERE library_id = ?1",
                "DELETE FROM artist_fts WHERE library_id = ?1",
                // In this order, because a track restricts the deletion of its
                // album and an album that of its album artist: left to the
                // library's cascade, which visits them in no promised order,
                // the deletion could trip over its own rows.
                "DELETE FROM track WHERE library_id = ?1",
                "DELETE FROM album_participant WHERE library_id = ?1",
                "DELETE FROM album WHERE library_id = ?1",
                "DELETE FROM artist WHERE library_id = ?1",
                "DELETE FROM genre WHERE library_id = ?1",
            ] {
                sqlx::query(statement)
                    .bind(&library)
                    .execute(&mut *tx)
                    .await?;
            }
            audit(
                &mut tx,
                Some(actor.id),
                "library.deleted",
                library_id,
                serde_json::json!({
                    "name": settings.library.name,
                    "tracks": tracks,
                    "albums": albums,
                    "artists": artists,
                }),
            )
            .await?;
            sqlx::query("DELETE FROM library WHERE id = ?")
                .bind(&library)
                .execute(&mut *tx)
                .await?;
            receipts = self.journal_removed(&_writer, &mut tx, &removed).await?;
            tx.commit().await?;
            deleted = LibraryDeleted {
                tracks,
                albums,
                artists,
            };
        }
        for (user_id, receipt) in receipts {
            self.sync.publish(user_id, receipt);
        }
        // After the commit, and best effort: a file left behind is litter in
        // the server's own folder, and the rows that named it are gone.
        let managed = settings.library.root_path.join(uploads::MANAGED_DIR);
        for session in staging {
            let _ = tokio::fs::remove_file(managed.join(format!("{session}.part"))).await;
        }
        for hash in &canvases {
            self.release_canvas_blob(hash).await;
        }
        self.scanner.libraries_changed();
        Ok(deleted)
    }
}
