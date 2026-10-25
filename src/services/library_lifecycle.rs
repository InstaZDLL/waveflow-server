//! A library's lifecycle and its missing tracks (RFC-021 decisions 4 and 5):
//! renaming, moving and deleting a library, listing the tracks its scans could
//! no longer find, and purging them.
//!
//! The one implementation behind the native API, the CLI and the web client.

use super::*;

/// How long the retention pass waits between two looks at the libraries.
const RETENTION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// What a library may be changed into. A field left out is left as it is.
#[derive(Debug, Default, Clone, Deserialize, ToSchema)]
pub struct LibraryUpdate {
    /// A new name, 1 to 160 characters once trimmed. The library's owner or an
    /// administrator.
    pub name: Option<String>,
    /// A new root: an existing directory, not itself a link, and not another
    /// library's. An administrator only; queues a scan.
    pub path: Option<String>,
    /// Follow symbolic links that stay inside the root. An administrator only;
    /// queues a scan.
    pub follow_links: Option<bool>,
}

impl LibraryUpdate {
    /// Whether this needs an administrator, and an administrative credential:
    /// the path and the links setting decide which part of the server's file
    /// system the server reads.
    pub fn is_administrative(&self) -> bool {
        self.path.is_some() || self.follow_links.is_some()
    }
}

/// What an update set in motion.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryUpdated {
    /// The scan a new path or links setting queued, if it queued one.
    pub scan_id: Option<Uuid>,
}

/// A library as an administrator manages it, its root included. Never shown
/// to a member who is not one: where the server keeps its files is the
/// operator's business.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibrarySettings {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    pub follow_links: bool,
}

/// What a deletion removed from the catalogue. Nothing was removed from disk.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryDeleted {
    pub tracks: i64,
    pub albums: i64,
    pub artists: i64,
}

/// A track whose file the scans can no longer find.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UnavailableTrack {
    pub id: Uuid,
    /// Where the file was, relative to the library.
    pub relative_path: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: i64,
    /// Since when, in epoch milliseconds.
    pub unavailable_since: i64,
    /// What a purge would lose, across every account: playlist entries,
    /// favourites and recorded plays holding this track.
    pub playlist_entries: i64,
    pub favorites: i64,
    pub plays: i64,
}

/// One page of a library's missing tracks, oldest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UnavailableTracks {
    /// How many there are in all, not on this page.
    pub total: i64,
    pub tracks: Vec<UnavailableTrack>,
}

/// Which missing tracks to purge: named ones, or all of them, and never
/// neither — a purge is explicit or it does not happen.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct PurgeRequest {
    pub track_ids: Option<Vec<Uuid>>,
    #[serde(default)]
    pub all: bool,
}

/// What a purge removed.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PurgeOutcome {
    pub purged: i64,
}

/// Which tracks one purge is about.
enum Selection {
    All,
    Named(String),
    OlderThan(i64),
}

/// What the caller is to one library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    Admin,
    Owner,
    Manager,
    Listener,
}

impl Standing {
    /// Owners, managers and administrators look after a library's missing
    /// tracks, as they look after what needs attention in it.
    fn curates(self) -> bool {
        matches!(self, Self::Admin | Self::Owner | Self::Manager)
    }
}

/// Who is asking, and with what authority.
///
/// An administrator's account is not enough to act as one: the credential the
/// request came in on must carry administrative authority too. A token whose
/// scopes stop short of `admin` holds only what its account's memberships
/// give it, as `AuthUser::scopes` says a narrowed credential must.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryActor {
    pub id: Uuid,
    /// Whether the credential grants administrative authority. The account's
    /// role is checked again on every operation; this can only narrow it.
    pub admin_credential: bool,
}

impl LibraryActor {
    /// The operator at the CLI, whose authority is the account's own.
    pub fn operator(id: Uuid) -> Self {
        Self {
            id,
            admin_credential: true,
        }
    }
}

/// The caller's standing on a library, read on the connection the operation
/// runs on. A library that does not exist, and one the caller is neither a
/// member of nor an administrator over, are the same answer: not found.
///
/// Administrative standing needs both halves: an active administrator's
/// account, and a credential that carries that authority.
async fn standing_on(
    connection: &mut SqliteConnection,
    actor: LibraryActor,
    library_id: Uuid,
) -> Result<Standing, ServiceError> {
    let actor_id = actor.id;
    let row = sqlx::query(
        "SELECT (SELECT role FROM library_member WHERE library_id = l.id AND user_id = ?) AS role, \
                EXISTS (SELECT 1 FROM account a WHERE a.id = ? AND a.role = 'admin' \
                        AND a.disabled = 0) AS admin \
         FROM library l WHERE l.id = ?",
    )
    .bind(actor_id.to_string())
    .bind(actor_id.to_string())
    .bind(library_id.to_string())
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(ServiceError::NotFound)?;
    if actor.admin_credential && row.try_get::<i64, _>("admin")? != 0 {
        return Ok(Standing::Admin);
    }
    match row.try_get::<Option<String>, _>("role")?.as_deref() {
        Some("owner") => Ok(Standing::Owner),
        Some("manager") => Ok(Standing::Manager),
        Some("listener") => Ok(Standing::Listener),
        _ => Err(ServiceError::NotFound),
    }
}

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

/// An audit row with what it is about, in the transaction of the change.
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor_id: Option<Uuid>,
    kind: &str,
    library_id: Uuid,
    details: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_event (actor_user_id, kind, subject_id, occurred_at, details_json) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(actor_id.map(|id| id.to_string()))
    .bind(kind)
    .bind(library_id.to_string())
    .bind(now_ms())
    .bind(details.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the favourites and ratings of the albums and artists among
/// `albums` and `artists` (JSON arrays of ids) that no longer exist.
async fn forget_vanished_entities(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    albums: &str,
    artists: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM user_star WHERE entity_type = 'album' \
           AND entity_id IN (SELECT value FROM json_each(?)) \
           AND NOT EXISTS (SELECT 1 FROM album WHERE album.id = user_star.entity_id)",
    )
    .bind(albums)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM user_rating WHERE entity_type = 'album' \
           AND entity_id IN (SELECT value FROM json_each(?)) \
           AND NOT EXISTS (SELECT 1 FROM album WHERE album.id = user_rating.entity_id)",
    )
    .bind(albums)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM user_star WHERE entity_type = 'artist' \
           AND entity_id IN (SELECT value FROM json_each(?)) \
           AND NOT EXISTS (SELECT 1 FROM artist WHERE artist.id = user_star.entity_id)",
    )
    .bind(artists)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM user_rating WHERE entity_type = 'artist' \
           AND entity_id IN (SELECT value FROM json_each(?)) \
           AND NOT EXISTS (SELECT 1 FROM artist WHERE artist.id = user_rating.entity_id)",
    )
    .bind(artists)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// What one purge removed, and the canvases it may have left with no link:
/// those are released after the commit, under their own locks, as removing a
/// canvas releases them.
#[derive(Default)]
struct Purged {
    tracks: i64,
    canvases: Vec<String>,
}

/// The purge itself, in the caller's transaction under the writer gate it
/// holds: the tracks that are **still** unavailable among those selected, what
/// cascades from them, their favourites and search rows, then the scan's own
/// consolidation, so no album or artist is left holding nothing.
async fn purge_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    library_id: Uuid,
    selection: &Selection,
) -> Result<Purged, sqlx::Error> {
    let library = library_id.to_string();
    let targets: Vec<(String, Option<String>)> = match selection {
        Selection::All => {
            sqlx::query_as(
                "SELECT id, album_id FROM track WHERE library_id = ? AND is_available = 0",
            )
            .bind(&library)
            .fetch_all(&mut **tx)
            .await?
        }
        Selection::Named(ids) => {
            sqlx::query_as(
                "SELECT id, album_id FROM track WHERE library_id = ? AND is_available = 0 \
                   AND id IN (SELECT value FROM json_each(?))",
            )
            .bind(&library)
            .bind(ids)
            .fetch_all(&mut **tx)
            .await?
        }
        Selection::OlderThan(cutoff) => {
            sqlx::query_as(
                "SELECT id, album_id FROM track WHERE library_id = ? AND is_available = 0 \
                   AND COALESCE(unavailable_since, updated_at) < ?",
            )
            .bind(&library)
            .bind(cutoff)
            .fetch_all(&mut **tx)
            .await?
        }
    };
    if targets.is_empty() {
        return Ok(Purged::default());
    }
    let tracks = serde_json::to_string(
        &targets
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
    )
    .expect("a list of strings serializes");
    // What may be left holding nothing once these go, read before they do.
    let albums: Vec<String> = targets
        .iter()
        .filter_map(|(_, album)| album.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut artists: std::collections::BTreeSet<String> = sqlx::query_scalar(
        "SELECT DISTINCT artist_id FROM track_participant \
         WHERE track_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&tracks)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .collect();
    let albums = serde_json::to_string(&albums).expect("a list of strings serializes");
    artists.extend(
        sqlx::query_scalar::<_, String>(
            "SELECT album_artist_id FROM album WHERE album_artist_id IS NOT NULL \
               AND id IN (SELECT value FROM json_each(?))",
        )
        .bind(&albums)
        .fetch_all(&mut **tx)
        .await?,
    );
    let artists = serde_json::to_string(&artists).expect("a list of strings serializes");
    // The loops these tracks carry: their links cascade, the blobs do not.
    let canvases: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT canvas_hash FROM track_canvas \
         WHERE track_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&tracks)
    .fetch_all(&mut **tx)
    .await?;

    // Neither has a foreign key to cascade through: an entity id names a
    // track, an album or an artist, whichever the type says.
    sqlx::query(
        "DELETE FROM user_star WHERE entity_type = 'track' \
           AND entity_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&tracks)
    .execute(&mut **tx)
    .await?;
    sqlx::query(
        "DELETE FROM user_rating WHERE entity_type = 'track' \
           AND entity_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&tracks)
    .execute(&mut **tx)
    .await?;
    // A virtual table: nothing cascades into it.
    sqlx::query("DELETE FROM track_fts WHERE track_id IN (SELECT value FROM json_each(?))")
        .bind(&tracks)
        .execute(&mut **tx)
        .await?;
    // `is_available = 0` again, in the statement that deletes: the selection
    // above was read in this transaction under the writer gate, so nothing can
    // have brought a track back since, and this says so where it matters.
    let purged = sqlx::query(
        "DELETE FROM track WHERE library_id = ? AND is_available = 0 \
           AND id IN (SELECT value FROM json_each(?))",
    )
    .bind(&library)
    .bind(&tracks)
    .execute(&mut **tx)
    .await?
    .rows_affected() as i64;
    // The scan's own consolidation: the album and artist sweeps, the credits
    // and the counts, the artist search index — over what is left.
    Database::consolidate_sort_names_in(tx, library_id).await?;
    Database::consolidate_catalog_derivations_in(tx, library_id).await?;
    forget_vanished_entities(tx, &albums, &artists).await?;
    sqlx::query(
        "DELETE FROM genre WHERE library_id = ? \
           AND NOT EXISTS (SELECT 1 FROM track_genre tg WHERE tg.genre_id = genre.id)",
    )
    .bind(&library)
    .execute(&mut **tx)
    .await?;
    Ok(Purged {
        tracks: purged,
        canvases,
    })
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
        let scan_id = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let written = sqlx::query(
                "UPDATE library SET name = COALESCE(?, name), \
                   root_path = COALESCE(?, root_path), \
                   follow_links = COALESCE(?, follow_links), updated_at = ? \
                 WHERE id = ?",
            )
            .bind(name.as_deref())
            .bind(
                root.as_ref()
                    .map(|root| root.to_string_lossy().into_owned()),
            )
            .bind(update.follow_links.map(i64::from))
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
        if moved {
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
            crate::catalog::LibraryRecord {
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
            for statement in [
                "DELETE FROM user_star WHERE \
                   (entity_type = 'track' AND entity_id IN (SELECT id FROM track WHERE library_id = ?1)) \
                   OR (entity_type = 'album' AND entity_id IN (SELECT id FROM album WHERE library_id = ?1)) \
                   OR (entity_type = 'artist' AND entity_id IN (SELECT id FROM artist WHERE library_id = ?1))",
                "DELETE FROM user_rating WHERE \
                   (entity_type = 'track' AND entity_id IN (SELECT id FROM track WHERE library_id = ?1)) \
                   OR (entity_type = 'album' AND entity_id IN (SELECT id FROM album WHERE library_id = ?1)) \
                   OR (entity_type = 'artist' AND entity_id IN (SELECT id FROM artist WHERE library_id = ?1))",
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
            tx.commit().await?;
            deleted = LibraryDeleted {
                tracks,
                albums,
                artists,
            };
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

    /// One page of the tracks a library's scans can no longer find, oldest
    /// first, with what purging each would cost (RFC-021 decision 4).
    pub async fn unavailable_tracks(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        page: BrowsePage,
    ) -> Result<UnavailableTracks, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        if !standing_on(&mut connection, actor, library_id)
            .await?
            .curates()
        {
            return Err(ServiceError::Forbidden);
        }
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM track WHERE library_id = ? AND is_available = 0",
        )
        .bind(library_id.to_string())
        .fetch_one(&mut *connection)
        .await?;
        let rows = sqlx::query(
            "SELECT t.id, t.relative_path, COALESCE(ovr.title, t.title) AS title, \
                    t.artist_display, t.album_title, t.duration_ms, \
                    COALESCE(t.unavailable_since, t.updated_at) AS since, \
                    (SELECT COUNT(*) FROM playlist_track pt WHERE pt.track_id = t.id) \
                      AS playlist_entries, \
                    (SELECT COUNT(*) FROM user_star us \
                      WHERE us.entity_type = 'track' AND us.entity_id = t.id) AS favorites, \
                    (SELECT COUNT(*) FROM play_event pe WHERE pe.track_id = t.id) AS plays \
             FROM track t LEFT JOIN track_override ovr ON ovr.track_id = t.id \
             WHERE t.library_id = ? AND t.is_available = 0 \
             ORDER BY since, t.relative_path, t.id LIMIT ? OFFSET ?",
        )
        .bind(library_id.to_string())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(&mut *connection)
        .await?;
        let tracks = rows
            .into_iter()
            .map(|row| {
                Ok(UnavailableTrack {
                    id: parse_uuid(row.try_get("id")?)?,
                    relative_path: row.try_get("relative_path")?,
                    title: row.try_get("title")?,
                    artist: row.try_get("artist_display")?,
                    album: row.try_get("album_title")?,
                    duration_ms: row.try_get("duration_ms")?,
                    unavailable_since: row.try_get("since")?,
                    playlist_entries: row.try_get("playlist_entries")?,
                    favorites: row.try_get("favorites")?,
                    plays: row.try_get("plays")?,
                })
            })
            .collect::<Result<Vec<_>, sqlx::Error>>()?;
        Ok(UnavailableTracks { total, tracks })
    }

    /// Purges missing tracks, explicitly (RFC-021 decision 4).
    ///
    /// Only those still unavailable when it runs: a track that came back
    /// between the listing and the request is kept. One transaction under the
    /// writer gate, refused while the library is being scanned — a running
    /// scan may be about to relocate one of these onto a file it just found.
    pub async fn purge_unavailable_tracks(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        request: PurgeRequest,
    ) -> Result<PurgeOutcome, ServiceError> {
        let selection = match (request.all, request.track_ids) {
            (true, None) => Selection::All,
            (false, Some(ids)) if !ids.is_empty() => Selection::Named(
                serde_json::to_string(&ids.iter().map(|id| id.to_string()).collect::<Vec<_>>())
                    .expect("a list of strings serializes"),
            ),
            _ => return Err(ServiceError::Invalid),
        };
        // Standing before the scan lock: a `409` for a library the caller may
        // not see would say that it exists, and that it is being scanned. Read
        // again in the transaction below, for a membership revoked meanwhile.
        {
            let mut connection = self.db.pool().acquire().await?;
            if !standing_on(&mut connection, actor, library_id)
                .await?
                .curates()
            {
                return Err(ServiceError::Forbidden);
            }
        }
        let _scan_lock = self
            .scanner
            .try_lock_library(library_id)
            .ok_or(ServiceError::Conflict)?;
        let purged = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            if !standing_on(&mut tx, actor, library_id).await?.curates() {
                return Err(ServiceError::Forbidden);
            }
            let purged = purge_in(&mut tx, library_id, &selection).await?;
            if purged.tracks > 0 {
                audit(
                    &mut tx,
                    Some(actor.id),
                    "library.tracks_purged",
                    library_id,
                    serde_json::json!({ "tracks": purged.tracks }),
                )
                .await?;
            }
            tx.commit().await?;
            purged
        };
        // After the commit and out of the writer gate, which releasing takes.
        for hash in &purged.canvases {
            self.release_canvas_blob(hash).await;
        }
        Ok(PurgeOutcome {
            purged: purged.tracks,
        })
    }

    /// One retention pass: in every library not being scanned, the tracks
    /// unavailable since before `cutoff` (epoch milliseconds). Public so a test
    /// can run it rather than wait an hour. Answers how many went.
    pub async fn purge_expired_unavailable_tracks(&self, cutoff: i64) -> Result<i64, ServiceError> {
        let mut total = 0;
        for library in self.db.all_libraries().await? {
            let Some(_scan_lock) = self.scanner.try_lock_library(library.id) else {
                continue;
            };
            let purged = {
                let _writer = self.db.writer_guard().await;
                let mut tx = self.db.begin_write(&_writer).await?;
                let purged = purge_in(&mut tx, library.id, &Selection::OlderThan(cutoff)).await?;
                if purged.tracks > 0 {
                    audit(
                        &mut tx,
                        None,
                        "library.tracks_purged",
                        library.id,
                        serde_json::json!({ "tracks": purged.tracks, "retention": true }),
                    )
                    .await?;
                }
                tx.commit().await?;
                purged
            };
            for hash in &purged.canvases {
                self.release_canvas_blob(hash).await;
            }
            total += purged.tracks;
        }
        Ok(total)
    }

    /// The retention of missing tracks, when the operator set one
    /// (`WAVEFLOW_UNAVAILABLE_RETENTION_DAYS`): once at boot, then hourly.
    pub fn spawn_unavailable_purge(&self) {
        let Some(retention) = self.unavailable_retention else {
            return;
        };
        let services = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(RETENTION_INTERVAL);
            loop {
                ticker.tick().await;
                let cutoff = now_ms()
                    .saturating_sub(i64::try_from(retention.as_millis()).unwrap_or(i64::MAX));
                match services.purge_expired_unavailable_tracks(cutoff).await {
                    Ok(0) => {}
                    Ok(purged) => {
                        tracing::info!(purged, "missing tracks past their retention were purged")
                    }
                    Err(error) => {
                        tracing::warn!(%error, "could not purge missing tracks past their retention")
                    }
                }
            }
        });
    }
}
