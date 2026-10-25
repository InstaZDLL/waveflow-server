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
    /// Watch the library for changes (RFC-021 decision 6). An administrator
    /// only: a watch costs the server descriptors, and finds nothing on a
    /// network share changed by another machine. Queues nothing; the periodic
    /// scan stays either way.
    pub watch: Option<bool>,
}

impl LibraryUpdate {
    /// Whether this needs an administrator, and an administrative credential:
    /// the path and the links setting decide which part of the server's file
    /// system the server reads, and the watch what the server spends on it.
    pub fn is_administrative(&self) -> bool {
        self.path.is_some() || self.follow_links.is_some() || self.watch.is_some()
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
    /// Whether the watcher observes this library.
    pub watch: bool,
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
    /// When a purge took it out of sight, for a track purged and still
    /// restorable; `null` on the missing-tracks listing.
    pub purged_at: Option<i64>,
    /// When it is deleted for good, unless restored first; `null` on the
    /// missing-tracks listing.
    pub deleted_after: Option<i64>,
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

/// What a purge did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PurgeOutcome {
    /// How many tracks it purged.
    pub purged: i64,
    /// Until when they can be restored, in epoch milliseconds; `null` when
    /// the server keeps no grace window (`WAVEFLOW_PURGE_GRACE_HOURS=0`) and
    /// they are already gone.
    pub restorable_until: Option<i64>,
}

/// What a restoration put back.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RestoreOutcome {
    pub restored: i64,
}

/// Which tracks one purge is about.
enum Selection {
    All,
    Named(String),
    OlderThan(i64),
    /// Purged at or before this instant, and past their grace window.
    PurgedBy(i64),
}

impl Selection {
    /// Whether the selection names purged tracks rather than missing ones.
    fn purged(&self) -> bool {
        matches!(self, Self::PurgedBy(_))
    }
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
/// `albums` and `artists` (JSON arrays of ids) that no longer exist, and
/// answers them, so the accounts that held them can be told.
async fn forget_vanished_entities(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    albums: &str,
    artists: &str,
    removed: &mut Removed,
) -> Result<(), sqlx::Error> {
    removed.stars.extend(
        sqlx::query_as::<_, (String, String, String)>(
            "DELETE FROM user_star WHERE entity_type = 'album' \
               AND entity_id IN (SELECT value FROM json_each(?)) \
               AND NOT EXISTS (SELECT 1 FROM album WHERE album.id = user_star.entity_id) \
             RETURNING user_id, entity_type, entity_id",
        )
        .bind(albums)
        .fetch_all(&mut **tx)
        .await?,
    );
    removed.ratings.extend(
        sqlx::query_as::<_, (String, String, String)>(
            "DELETE FROM user_rating WHERE entity_type = 'album' \
               AND entity_id IN (SELECT value FROM json_each(?)) \
               AND NOT EXISTS (SELECT 1 FROM album WHERE album.id = user_rating.entity_id) \
             RETURNING user_id, entity_type, entity_id",
        )
        .bind(albums)
        .fetch_all(&mut **tx)
        .await?,
    );
    removed.stars.extend(
        sqlx::query_as::<_, (String, String, String)>(
            "DELETE FROM user_star WHERE entity_type = 'artist' \
               AND entity_id IN (SELECT value FROM json_each(?)) \
               AND NOT EXISTS (SELECT 1 FROM artist WHERE artist.id = user_star.entity_id) \
             RETURNING user_id, entity_type, entity_id",
        )
        .bind(artists)
        .fetch_all(&mut **tx)
        .await?,
    );
    removed.ratings.extend(
        sqlx::query_as::<_, (String, String, String)>(
            "DELETE FROM user_rating WHERE entity_type = 'artist' \
               AND entity_id IN (SELECT value FROM json_each(?)) \
               AND NOT EXISTS (SELECT 1 FROM artist WHERE artist.id = user_rating.entity_id) \
             RETURNING user_id, entity_type, entity_id",
        )
        .bind(artists)
        .fetch_all(&mut **tx)
        .await?,
    );
    Ok(())
}

/// What a removal took from accounts, so their sync journals can say so
/// (RFC-021, what was still open): each `(account, entity type, entity id)`
/// favourite and rating that went, and the playlists that lost an entry.
#[derive(Default)]
struct Removed {
    stars: Vec<(String, String, String)>,
    ratings: Vec<(String, String, String)>,
    playlists: Vec<String>,
}

/// The playlists holding any of `tracks` (a JSON array of ids), read before
/// the tracks go and take their entries with them.
async fn playlists_holding(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    tracks: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT playlist_id FROM playlist_track \
         WHERE track_id IN (SELECT value FROM json_each(?))",
    )
    .bind(tracks)
    .fetch_all(&mut **tx)
    .await
}

/// What one purge removed, and the canvases it may have left with no link:
/// those are released after the commit, under their own locks, as removing a
/// canvas releases them.
#[derive(Default)]
struct Purged {
    tracks: i64,
    canvases: Vec<String>,
    removed: Removed,
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
        Selection::PurgedBy(cutoff) => {
            sqlx::query_as(
                "SELECT id, album_id FROM track WHERE library_id = ? AND is_available = 0 \
                   AND purged_at IS NOT NULL AND purged_at <= ?",
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
    let mut removed = Removed {
        playlists: playlists_holding(tx, &tracks).await?,
        ..Removed::default()
    };

    // Neither has a foreign key to cascade through: an entity id names a
    // track, an album or an artist, whichever the type says. Answered, so the
    // accounts that held them are told through their journals.
    removed.stars = sqlx::query_as(
        "DELETE FROM user_star WHERE entity_type = 'track' \
           AND entity_id IN (SELECT value FROM json_each(?)) \
         RETURNING user_id, entity_type, entity_id",
    )
    .bind(&tracks)
    .fetch_all(&mut **tx)
    .await?;
    removed.ratings = sqlx::query_as(
        "DELETE FROM user_rating WHERE entity_type = 'track' \
           AND entity_id IN (SELECT value FROM json_each(?)) \
         RETURNING user_id, entity_type, entity_id",
    )
    .bind(&tracks)
    .fetch_all(&mut **tx)
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
    forget_vanished_entities(tx, &albums, &artists, &mut removed).await?;
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
        removed,
    })
}

/// Takes the selected missing tracks out of sight without deleting anything:
/// the first half of a purge when the server keeps a grace window. Only those
/// still unavailable and not already purged. Answers how many.
async fn hide_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    library_id: Uuid,
    selection: &Selection,
    now: i64,
) -> Result<i64, sqlx::Error> {
    let library = library_id.to_string();
    let hidden = match selection {
        Selection::All => {
            sqlx::query(
                "UPDATE track SET purged_at = ? \
                 WHERE library_id = ? AND is_available = 0 AND purged_at IS NULL",
            )
            .bind(now)
            .bind(&library)
            .execute(&mut **tx)
            .await?
        }
        Selection::Named(ids) => {
            sqlx::query(
                "UPDATE track SET purged_at = ? \
                 WHERE library_id = ? AND is_available = 0 AND purged_at IS NULL \
                   AND id IN (SELECT value FROM json_each(?))",
            )
            .bind(now)
            .bind(&library)
            .bind(ids)
            .execute(&mut **tx)
            .await?
        }
        Selection::OlderThan(cutoff) => {
            sqlx::query(
                "UPDATE track SET purged_at = ? \
                 WHERE library_id = ? AND is_available = 0 AND purged_at IS NULL \
                   AND COALESCE(unavailable_since, updated_at) < ?",
            )
            .bind(now)
            .bind(&library)
            .bind(cutoff)
            .execute(&mut **tx)
            .await?
        }
        Selection::PurgedBy(_) => return Ok(0),
    };
    Ok(hidden.rows_affected() as i64)
}

/// One page of a library's missing tracks with what purging each would cost:
/// those not purged, oldest missing first, when `grace` is `None`; those
/// purged and still restorable, soonest deleted first, when it is the grace
/// window in milliseconds.
async fn list_missing_on(
    connection: &mut SqliteConnection,
    library_id: Uuid,
    page: BrowsePage,
    grace: Option<i64>,
) -> Result<UnavailableTracks, ServiceError> {
    let purged = grace.is_some();
    // A purge past its window is due for deletion, not restorable: it is
    // listed nowhere until the finishing pass takes it.
    let restorable_after = grace.map(|grace| now_ms().saturating_sub(grace));
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM track WHERE library_id = ? AND is_available = 0 \
           AND (purged_at IS NOT NULL) = ? AND (? IS NULL OR purged_at > ?)",
    )
    .bind(library_id.to_string())
    .bind(purged)
    .bind(restorable_after)
    .bind(restorable_after)
    .fetch_one(&mut *connection)
    .await?;
    let rows = sqlx::query(
        "SELECT t.id, t.relative_path, COALESCE(ovr.title, t.title) AS title, \
                t.artist_display, t.album_title, t.duration_ms, \
                COALESCE(t.unavailable_since, t.updated_at) AS since, t.purged_at, \
                (SELECT COUNT(*) FROM playlist_track pt WHERE pt.track_id = t.id) \
                  AS playlist_entries, \
                (SELECT COUNT(*) FROM user_star us \
                  WHERE us.entity_type = 'track' AND us.entity_id = t.id) AS favorites, \
                (SELECT COUNT(*) FROM play_event pe WHERE pe.track_id = t.id) AS plays \
         FROM track t LEFT JOIN track_override ovr ON ovr.track_id = t.id \
         WHERE t.library_id = ? AND t.is_available = 0 AND (t.purged_at IS NOT NULL) = ? \
           AND (? IS NULL OR t.purged_at > ?) \
         ORDER BY t.purged_at, since, t.relative_path, t.id LIMIT ? OFFSET ?",
    )
    .bind(library_id.to_string())
    .bind(purged)
    .bind(restorable_after)
    .bind(restorable_after)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&mut *connection)
    .await?;
    let tracks = rows
        .into_iter()
        .map(|row| {
            let purged_at: Option<i64> = row.try_get("purged_at")?;
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
                purged_at,
                deleted_after: purged_at
                    .zip(grace)
                    .map(|(at, grace)| at.saturating_add(grace)),
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    Ok(UnavailableTracks { total, tracks })
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
        list_missing_on(&mut connection, library_id, page, None).await
    }

    /// One page of a library's purged tracks still within their grace window,
    /// soonest deleted first, for the people who may purge them and so may
    /// take the purge back.
    pub async fn purged_tracks(
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
        let grace = i64::try_from(self.purge_grace.as_millis()).unwrap_or(i64::MAX);
        list_missing_on(&mut connection, library_id, page, Some(grace)).await
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
        // With a grace window, the purge only takes the tracks out of sight,
        // and the finishing pass deletes them once it has passed (RFC-021,
        // undoing a purge). Without one, it deletes them now, as it always did.
        if !self.purge_grace.is_zero() {
            let now = now_ms();
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            if !standing_on(&mut tx, actor, library_id).await?.curates() {
                return Err(ServiceError::Forbidden);
            }
            let hidden = hide_in(&mut tx, library_id, &selection, now).await?;
            let restorable_until =
                now.saturating_add(i64::try_from(self.purge_grace.as_millis()).unwrap_or(i64::MAX));
            if hidden > 0 {
                audit(
                    &mut tx,
                    Some(actor.id),
                    "library.tracks_purged",
                    library_id,
                    serde_json::json!({ "tracks": hidden, "restorable_until": restorable_until }),
                )
                .await?;
            }
            tx.commit().await?;
            return Ok(PurgeOutcome {
                purged: hidden,
                restorable_until: Some(restorable_until),
            });
        }
        let purged = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            if !standing_on(&mut tx, actor, library_id).await?.curates() {
                return Err(ServiceError::Forbidden);
            }
            let mut purged = purge_in(&mut tx, library_id, &selection).await?;
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
            let receipts = self
                .journal_removed(&_writer, &mut tx, &std::mem::take(&mut purged.removed))
                .await?;
            tx.commit().await?;
            for (user_id, receipt) in receipts {
                self.sync.publish(user_id, receipt);
            }
            purged
        };
        // After the commit and out of the writer gate, which releasing takes.
        for hash in &purged.canvases {
            self.release_canvas_blob(hash).await;
        }
        Ok(PurgeOutcome {
            purged: purged.tracks,
            restorable_until: None,
        })
    }

    /// Takes a purge back, for tracks still within their grace window: they
    /// return to the missing-tracks listing, every playlist entry, favourite
    /// and play still attached, as if the purge had never been asked for.
    /// Named tracks, or all of them, and never neither. Nothing is scanned:
    /// a restored track is still missing until its file is found again.
    pub async fn restore_purged_tracks(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        request: PurgeRequest,
    ) -> Result<RestoreOutcome, ServiceError> {
        let named = match (request.all, request.track_ids) {
            (true, None) => None,
            (false, Some(ids)) if !ids.is_empty() => Some(
                serde_json::to_string(&ids.iter().map(|id| id.to_string()).collect::<Vec<_>>())
                    .expect("a list of strings serializes"),
            ),
            _ => return Err(ServiceError::Invalid),
        };
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        if !standing_on(&mut tx, actor, library_id).await?.curates() {
            return Err(ServiceError::Forbidden);
        }
        // Under the writer gate, which the finishing pass takes too: a track
        // is either restored here or deleted there, never both. Only within
        // the window: one past it is due for deletion even if the pass has not
        // reached it yet — it ran before, or the library was being scanned.
        let restorable_after = now_ms()
            .saturating_sub(i64::try_from(self.purge_grace.as_millis()).unwrap_or(i64::MAX));
        let restored = sqlx::query(
            "UPDATE track SET purged_at = NULL \
             WHERE library_id = ? AND purged_at IS NOT NULL AND purged_at > ? \
               AND (? IS NULL OR id IN (SELECT value FROM json_each(?)))",
        )
        .bind(library_id.to_string())
        .bind(restorable_after)
        .bind(named.as_deref())
        .bind(named.as_deref())
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;
        if restored > 0 {
            audit(
                &mut tx,
                Some(actor.id),
                "library.purge_restored",
                library_id,
                serde_json::json!({ "tracks": restored }),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(RestoreOutcome { restored })
    }

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

    /// Tells each account what a removal took from it, through its own sync
    /// journal (RFC-021, what was still open): a favourite or a rating that
    /// went with its track, album or artist is a `delete`, and a playlist
    /// that lost entries is an `upsert` of what it now holds, as an edit of
    /// it would journal. A device mirroring the account then drops them at its
    /// next pull instead of keeping them until the playlist next changes.
    ///
    /// In the removal's own transaction, under the writer gate it holds: the
    /// journal and the catalogue land together or not at all. The receipts
    /// are published by the caller once that transaction has committed.
    async fn journal_removed(
        &self,
        writer: &tokio::sync::OwnedMutexGuard<()>,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        removed: &Removed,
    ) -> Result<Vec<(Uuid, MutationReceipt)>, ServiceError> {
        let mut receipts = Vec::new();
        for (kind, rows) in [("favorite", &removed.stars), ("rating", &removed.ratings)] {
            for (user, entity_type, entity) in rows {
                let user_id = parse_uuid(user.clone())?;
                let entity_id = parse_uuid(entity.clone())?;
                let action = if kind == "favorite" {
                    "unstar"
                } else {
                    "set-rating"
                };
                let context = self
                    .server_claim(
                        writer,
                        tx,
                        user_id,
                        action,
                        &format!("{entity_type}:{entity_id}"),
                    )
                    .await?;
                let payload = if kind == "favorite" {
                    serde_json::json!({
                        "entity_type": entity_type,
                        "entity_id": entity_id,
                        "starred": false,
                    })
                } else {
                    serde_json::json!({
                        "entity_type": entity_type,
                        "entity_id": entity_id,
                        "rating": 0,
                    })
                };
                let receipt = self
                    .sync
                    .complete_operation(
                        tx,
                        user_id,
                        context,
                        kind,
                        entity_id,
                        "delete",
                        &payload,
                        Some(entity_id),
                    )
                    .await?;
                receipts.push((user_id, receipt));
            }
        }
        let now = now_ms();
        for playlist in &removed.playlists {
            // Gone with the library's own owner, or with nothing left to say.
            let Some(row) = sqlx::query(
                "UPDATE playlist SET updated_at = ? WHERE id = ? \
                 RETURNING owner_user_id, name, comment, public",
            )
            .bind(now)
            .bind(playlist)
            .fetch_optional(&mut **tx)
            .await?
            else {
                continue;
            };
            let user_id = parse_uuid(row.try_get("owner_user_id")?)?;
            let playlist_id = parse_uuid(playlist.clone())?;
            let track_ids: Vec<String> = sqlx::query_scalar(
                "SELECT track_id FROM playlist_track WHERE playlist_id = ? ORDER BY position",
            )
            .bind(playlist)
            .fetch_all(&mut **tx)
            .await?;
            let context = self
                .server_claim(
                    writer,
                    tx,
                    user_id,
                    "update",
                    &format!("playlist:{playlist_id}"),
                )
                .await?;
            let receipt = self
                .sync
                .complete_operation(
                    tx,
                    user_id,
                    context,
                    "playlist",
                    playlist_id,
                    "upsert",
                    &serde_json::json!({
                        "id": playlist_id,
                        "name": row.try_get::<String, _>("name")?,
                        "comment": row.try_get::<Option<String>, _>("comment")?,
                        "public": row.try_get::<i64, _>("public")? != 0,
                        "track_ids": track_ids,
                    }),
                    Some(playlist_id),
                )
                .await?;
            receipts.push((user_id, receipt));
        }
        Ok(receipts)
    }

    /// Opens a journal operation the server makes on an account's behalf. A
    /// fresh operation id cannot be a replay, so anything but a new claim is a
    /// fault rather than an answer.
    async fn server_claim(
        &self,
        writer: &tokio::sync::OwnedMutexGuard<()>,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        user_id: Uuid,
        action: &str,
        target: &str,
    ) -> Result<MutationContext, ServiceError> {
        let context = MutationContext::server_generated();
        let intent = MutationIntent::new(
            action,
            target,
            &serde_json::json!({ "removed_from_catalogue": true }),
        );
        match self
            .sync
            .claim_operation(writer, tx, user_id, context, intent)
            .await?
        {
            OperationClaim::New => Ok(context),
            OperationClaim::Replayed(_) => Err(ServiceError::Conflict),
        }
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
