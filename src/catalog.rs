//! Tenant-scoped catalogue repository used by scanner and HTTP surfaces.

use std::{path::PathBuf, str::FromStr};

use serde::Serialize;
use sqlx::{Row, Sqlite, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{authentication::now_ms, database::Database};

#[derive(Debug, Clone)]
pub struct LibraryRecord {
    pub id: Uuid,
    pub name: String,
    pub root_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryAccess {
    pub id: Uuid,
    pub name: String,
    pub visibility: crate::database::LibraryVisibility,
    pub role: crate::database::LibraryRole,
    pub last_scan_started_at: Option<i64>,
    pub last_scan_completed_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct ExistingTrack {
    pub id: Uuid,
    pub relative_path: String,
    pub file_size: i64,
    pub modified_at: i64,
    pub quick_hash: String,
    pub full_hash: String,
    pub lyrics_hash: Option<String>,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct StreamTrack {
    pub id: Uuid,
    pub library_id: Uuid,
    pub library_root: PathBuf,
    pub relative_path: String,
    pub full_hash: String,
    pub codec: Option<String>,
    pub bitrate: Option<i64>,
    pub duration_ms: i64,
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct ArtworkInput {
    pub hash: String,
    pub format: String,
    pub source: String,
    pub byte_size: i64,
}

#[derive(Debug, Clone)]
pub struct CatalogTrackInput {
    pub relative_path: String,
    pub file_size: i64,
    pub modified_at: i64,
    pub quick_hash: String,
    pub full_hash: String,
    pub title: String,
    pub artist: Option<String>,
    /// The multi-valued spellings of the two artist tags, when a file used
    /// them. They win over the singular ones and are never cut: a tagger
    /// writing `ARTISTS` has already said where the boundaries are.
    pub artists: Vec<String>,
    pub album_artists: Vec<String>,
    /// Every other role the file names, as it spelled them.
    pub roles: Vec<(crate::tags::Role, Vec<String>)>,
    /// Performer credits, already split into instrument and name.
    pub performer_pairs: Vec<(String, String)>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub is_compilation: bool,
    pub genre: Option<String>,
    pub year: Option<i64>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub duration_ms: i64,
    pub bitrate: Option<i64>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub bit_depth: Option<i64>,
    pub codec: Option<String>,
    pub musical_key: Option<String>,
    pub tag_rating: Option<i64>,
    /// The MusicBrainz recording identifier: the performance, which is what a
    /// song's `musicBrainzId` means. Release and artist are separate entities
    /// and are kept separate.
    pub musicbrainz_recording_id: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    pub musicbrainz_artist_id: Option<String>,
    /// Decibel gains and linear peaks, as tagged.
    pub replay_gain_track_gain: Option<f64>,
    pub replay_gain_track_peak: Option<f64>,
    pub replay_gain_album_gain: Option<f64>,
    pub replay_gain_album_peak: Option<f64>,
    pub bpm: Option<i64>,
    pub sort_title: Option<String>,
    /// Sort forms of the album title and of the two artist credits, as
    /// tagged. Multi-valued credits carry them joined by the same separator.
    pub sort_album: Option<String>,
    pub sort_album_artist: Option<String>,
    pub sort_artist: Option<String>,
    pub comment: Option<String>,
    /// Multi-valued, split like `artist` and `genre`.
    pub isrc: Option<String>,
    pub moods: Option<String>,
    /// Normalised to `explicit` or `clean`; any other tag value is no value.
    pub explicit_status: Option<String>,
    pub artwork: Option<ArtworkInput>,
    pub lyrics_hash: String,
    pub lyrics: Vec<crate::lyrics::LyricsInput>,
}

#[derive(Debug, Clone)]
pub struct CatalogApply {
    pub input: CatalogTrackInput,
    pub existing_id: Option<Uuid>,
    pub moved: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    Added,
    Updated,
    Moved,
}

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
    pub current_path: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TrackRecord {
    pub id: Uuid,
    pub library_id: Uuid,
    pub relative_path: String,
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    /// Primary credited artist, matching the first artist in `artist`.
    pub artist_id: Option<Uuid>,
    pub genre: Option<String>,
    pub duration_ms: i64,
    pub codec: Option<String>,
    pub artwork_hash: Option<String>,
    /// Content fingerprint: BLAKE3, unkeyed, hexadecimal, over the whole file.
    /// See `services::SongItem::full_hash` for the contract.
    pub full_hash: String,
    pub available: bool,
}

impl Database {
    pub async fn libraries_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<LibraryAccess>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT l.id, l.name, l.visibility, m.role, l.last_scan_started_at, \
                    l.last_scan_completed_at \
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
                    visibility: crate::database::LibraryVisibility::from_str(
                        row.try_get::<&str, _>("visibility")?,
                    )
                    .map_err(|error| sqlx::Error::Decode(error.into()))?,
                    role: crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
                        .map_err(|error| sqlx::Error::Decode(error.into()))?,
                    last_scan_started_at: row.try_get("last_scan_started_at")?,
                    last_scan_completed_at: row.try_get("last_scan_completed_at")?,
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

    /// Notices that this instance derives identifiers differently from the run
    /// that built the catalogue, and asks for a full rescan when it does.
    ///
    /// No file timestamp can reveal this: the files have not changed, the rule
    /// reading them has. Comparing the persisted spec against the configured
    /// one is the only way the difference is visible at all.
    ///
    /// Returns how many libraries were marked, so a caller can say so.
    pub async fn reconcile_catalog_identity(
        &self,
        specs: &crate::pid::PidSpecs,
    ) -> Result<u64, sqlx::Error> {
        // `pid.track` is recorded by a completed scan but deliberately not
        // compared. A track's identity is its path, then its content hash with
        // the relocation candidates eliminated — six tables cascade off it, so
        // moving track ids would delete playlists and play history rather than
        // orphan them. Nothing is derived from the track spec today, so a
        // change to it re-identifies nothing and must not cost a full rescan.
        let active = [
            ("pid.album", specs.album.source()),
            ("pid.artist", specs.artist.source()),
        ];
        let mut changed = Vec::new();
        for (key, value) in active {
            match self.server_property(key).await? {
                // A catalogue built before the property existed is not
                // evidence of a different rule, only of an older server. The
                // first scan writes what it used, and later boots compare.
                None => {}
                Some(stored) if stored == value => {}
                Some(stored) => changed.push((key, stored, value)),
            }
        }
        if changed.is_empty() {
            return Ok(0);
        }
        for (key, stored, active) in &changed {
            tracing::warn!(
                setting = key,
                %stored,
                %active,
                "catalogue identity setting changed since the last scan"
            );
        }
        let libraries = self.request_full_scan_everywhere().await?;
        tracing::warn!(
            libraries,
            "every library will be rescanned in full so its identifiers follow the new rule"
        );
        Ok(libraries)
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
    /// someone else's disk on it. [`crate::database::LibraryRole::may_scan`]
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
        let _writer = self.writer_guard().await;
        let now = now_ms();
        sqlx::query(
            "UPDATE scan_job SET status = 'running', total_files = ?, full_scan = ?, \
             started_at = ? WHERE id = ?",
        )
        .bind(total)
        .bind(i64::from(full))
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

    /// Recomputes the derived MusicBrainz identifier of every album and artist
    /// in one library from the tracks that currently back them.
    ///
    /// Tracks of one album routinely disagree: a release id is written per file
    /// by whatever tagger touched it, and a library assembled over years holds
    /// files tagged against different releases of the same record. The album's
    /// identifier is therefore the one most of its available tracks agree on,
    /// not the one the last file scanned happened to carry.
    ///
    /// Ties are broken by the earliest disc and track number, zero-padded into
    /// a single sortable key so `MIN` compares the pair rather than each half
    /// independently, and finally by the identifier itself. A tie means the
    /// tags genuinely split the album down the middle; what matters is that two
    /// scans of unchanged files answer the same thing, so nothing flaps.
    ///
    /// An artist takes the identifier from the tracks it is the *primary*
    /// credit of (`track_artist.position = 0`), because `musicbrainz_artist_id`
    /// is one value on a track that may credit several artists, and only the
    /// first credit is the one the tag is about.
    ///
    /// Albums and artists whose tracks carry no identifier are set back to
    /// `NULL` rather than left alone: this runs after every scan, so a tag
    /// removed from the files has to disappear from the catalogue too.
    /// Re-derives `album.sort_name` and `artist.sort_name` from the tags the
    /// library's available tracks still carry.
    ///
    /// Sibling of [`Self::consolidate_catalog_derivations`] and run beside it, for
    /// the same reason and by the same rule: a value derived from tags has to
    /// follow the tags, including when they go away. Writing the sort name
    /// during the per-track upsert could not do that — the row is rewritten
    /// once per track of every album the artist appears on, so a file carrying
    /// no tag had to be prevented from erasing what a sibling supplied, and
    /// that preservation outlived the tag itself.
    ///
    /// The majority wins, ties broken deterministically, so two scans of
    /// unchanged files answer the same thing. An album takes the sort title
    /// its tracks agree on; an artist takes the sort form its credits agree
    /// on, from `track_artist` where the joined tag was already paired with
    /// the joined credit. An album artist credited on none of the tracks
    /// therefore has no sort form of its own — the catalogue holds no tag
    /// that is about it alone.
    pub async fn consolidate_sort_names(&self, library_id: Uuid) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "UPDATE album SET sort_name = ( \
               SELECT t.sort_album FROM track t \
               WHERE t.album_id = album.id AND t.is_available = 1 \
                 AND t.sort_album IS NOT NULL \
               GROUP BY t.sort_album \
               ORDER BY COUNT(*) DESC, t.sort_album \
               LIMIT 1) \
             WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE artist SET sort_name = ( \
               SELECT tp.sort_name FROM track_participant tp \
               JOIN track t ON t.id = tp.track_id \
               WHERE tp.artist_id = artist.id AND t.is_available = 1 \
                 AND tp.sort_name IS NOT NULL \
               GROUP BY tp.sort_name \
               ORDER BY COUNT(*) DESC, tp.sort_name \
               LIMIT 1) \
             WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Everything the catalogue derives from its own contents, after a scan.
    ///
    /// It was named for the MusicBrainz identifiers alone, which stopped being
    /// true: it also rebuilds each album's credits from its tracks', recounts
    /// what every artist has to show for each capacity, and drops the artist
    /// rows nothing credits any more. All of it is a majority or a union over
    /// the tracks that are still available, so all of it has to run after
    /// `mark_unseen_unavailable` — a file that vanished must stop voting first.
    ///
    /// A test that builds a catalogue by applying tracks directly runs this
    /// too, for the same reason the scanner does: without it the album has no
    /// credits and no artist has a count.
    pub async fn consolidate_catalog_derivations(
        &self,
        library_id: Uuid,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool().begin().await?;
        sqlx::query(
            "UPDATE album SET musicbrainz_id = ( \
               SELECT t.musicbrainz_release_id FROM track t \
               WHERE t.album_id = album.id AND t.is_available = 1 \
                 AND t.musicbrainz_release_id IS NOT NULL \
               GROUP BY t.musicbrainz_release_id \
               ORDER BY COUNT(*) DESC, \
                        MIN(printf('%06d%06d', COALESCE(t.disc_number, 1), \
                                   COALESCE(t.track_number, 0))), \
                        t.musicbrainz_release_id \
               LIMIT 1) \
             WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE artist SET musicbrainz_id = ( \
               SELECT t.musicbrainz_artist_id FROM track t \
               JOIN track_participant tp ON tp.track_id = t.id AND tp.role = 'artist' AND tp.position = 0 \
               WHERE tp.artist_id = artist.id AND t.is_available = 1 \
                 AND t.musicbrainz_artist_id IS NOT NULL \
               GROUP BY t.musicbrainz_artist_id \
               ORDER BY COUNT(*) DESC, t.musicbrainz_artist_id \
               LIMIT 1) \
             WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        // The album's own credits, derived here rather than written per track.
        // A track-by-track write would be a last-writer-wins race between
        // files of one album that disagree; deriving after availability is
        // settled makes the answer the same whatever order the scan ran in.
        sqlx::query("DELETE FROM album_participant WHERE library_id = ?")
            .bind(library_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO album_participant \
               (album_id, artist_id, library_id, role, sub_role, position) \
             SELECT t.album_id, tp.artist_id, t.library_id, tp.role, tp.sub_role, \
                    ROW_NUMBER() OVER ( \
                      PARTITION BY t.album_id, tp.role \
                      ORDER BY MIN(tp.position), tp.sub_role, tp.artist_id) - 1 \
               FROM track t \
               JOIN track_participant tp ON tp.track_id = t.id \
              WHERE t.library_id = ? AND t.is_available = 1 AND t.album_id IS NOT NULL \
              GROUP BY t.album_id, tp.artist_id, tp.role, tp.sub_role",
        )
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        // What each artist has to show for each capacity. `albumCount` used to
        // be a count over the album's single artist column, which could only
        // ever see the first credit; the roles an artist appears in are now
        // simply the keys of this table.
        sqlx::query("DELETE FROM artist_role_stats WHERE library_id = ?")
            .bind(library_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO artist_role_stats \
               (artist_id, library_id, role, album_count, song_count) \
             SELECT tp.artist_id, t.library_id, tp.role, \
                    COUNT(DISTINCT t.album_id), COUNT(DISTINCT t.id) \
               FROM track_participant tp \
               JOIN track t ON t.id = tp.track_id \
              WHERE t.library_id = ? AND t.is_available = 1 \
              GROUP BY tp.artist_id, t.library_id, tp.role",
        )
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        // An album no track depends on is no longer in the catalogue. This
        // matters most on the scan that follows the identity migration: the
        // rows kept by the migration hold their old identifiers, the rescan
        // mints derived ones and moves every track across, and without this
        // the library would answer with each album twice — once full, once
        // empty. It runs before the artist sweep because `album_artist_id`
        // restricts on delete, so a ghost album would hold its artist hostage.
        sqlx::query(
            "DELETE FROM album WHERE library_id = ? \
               AND NOT EXISTS (SELECT 1 FROM track t WHERE t.album_id = album.id)",
        )
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        // An artist row outlives the tag that created it: rename a credit and
        // the old name stays in the index forever. An artist credited on
        // nothing is no longer part of the catalogue, so it goes with the same
        // scan that made it unreferenced. Both relations are checked, because
        // an album artist need not be credited on any of the album's tracks.
        sqlx::query(
            "DELETE FROM artist WHERE library_id = ? \
               AND NOT EXISTS (SELECT 1 FROM track_participant tp WHERE tp.artist_id = artist.id) \
               AND NOT EXISTS (SELECT 1 FROM album_participant ap WHERE ap.artist_id = artist.id) \
               AND NOT EXISTS (SELECT 1 FROM album al WHERE al.album_artist_id = artist.id)",
        )
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn finish_scan_job(
        &self,
        scan_id: Uuid,
        unavailable: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.pool().begin().await?;
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

    pub async fn existing_track_by_path(
        &self,
        library_id: Uuid,
        relative_path: &str,
    ) -> Result<Option<ExistingTrack>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, relative_path, file_size, file_modified_at, quick_hash, full_hash, lyrics_hash, is_available \
             FROM track WHERE library_id = ? AND relative_path = ?",
        )
        .bind(library_id.to_string())
        .bind(relative_path)
        .fetch_optional(self.pool())
        .await?;
        row.map(existing_track_from_row).transpose()
    }

    pub async fn relocation_candidates(
        &self,
        library_id: Uuid,
        full_hash: &str,
    ) -> Result<Vec<ExistingTrack>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, relative_path, file_size, file_modified_at, quick_hash, full_hash, lyrics_hash, is_available \
             FROM track WHERE library_id = ? AND full_hash = ?",
        )
        .bind(library_id.to_string())
        .bind(full_hash)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(existing_track_from_row).collect()
    }

    pub async fn mark_track_seen(&self, track_id: Uuid, scan_id: Uuid) -> Result<(), sqlx::Error> {
        self.mark_tracks_seen(&[track_id], scan_id).await
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
        let mut tx = self.pool().begin().await?;
        let now = now_ms();
        for track_id in track_ids {
            sqlx::query(
                "UPDATE track SET is_available = 1, last_seen_scan_id = ?, updated_at = ? WHERE id = ?",
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

    pub async fn apply_catalog_track(
        &self,
        library_id: Uuid,
        scan_id: Uuid,
        input: &CatalogTrackInput,
        existing_id: Option<Uuid>,
        moved: bool,
    ) -> Result<ApplyOutcome, sqlx::Error> {
        let mut outcomes = self
            .apply_catalog_tracks(
                library_id,
                scan_id,
                &[CatalogApply {
                    input: input.clone(),
                    existing_id,
                    moved,
                }],
            )
            .await?;
        Ok(outcomes.pop().expect("single-item catalogue batch"))
    }

    pub async fn apply_catalog_tracks(
        &self,
        library_id: Uuid,
        scan_id: Uuid,
        applies: &[CatalogApply],
    ) -> Result<Vec<ApplyOutcome>, sqlx::Error> {
        if applies.is_empty() {
            return Ok(Vec::new());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.pool().begin().await?;
        let now = now_ms();
        let mut outcomes = Vec::with_capacity(applies.len());
        for apply in applies {
            outcomes.push(
                Self::apply_catalog_track_in_transaction(
                    &mut tx,
                    self.pid(),
                    library_id,
                    scan_id,
                    apply,
                    now,
                )
                .await?,
            );
        }
        tx.commit().await?;
        Ok(outcomes)
    }

    #[allow(clippy::too_many_arguments)]
    async fn apply_catalog_track_in_transaction(
        tx: &mut Transaction<'_, Sqlite>,
        pid: &crate::pid::PidSpecs,
        library_id: Uuid,
        scan_id: Uuid,
        apply: &CatalogApply,
        now: i64,
    ) -> Result<ApplyOutcome, sqlx::Error> {
        let input = &apply.input;
        let existing_id = apply.existing_id;
        // Credits are derived here rather than by the scanner because this is
        // where identity is written: the test suite applies catalogue rows
        // directly, and reading the tag values one way there and another way
        // in production is how the two drift apart.
        let credits = crate::tags::credits(&crate::tags::RawCredits {
            artist: input.artist.iter().cloned().collect(),
            artists: input.artists.clone(),
            album_artist: input.album_artist.iter().cloned().collect(),
            album_artists: input.album_artists.clone(),
            sort_artist: input.sort_artist.clone(),
            sort_album_artist: input.sort_album_artist.clone(),
            is_compilation: input.is_compilation,
            roles: input.roles.clone(),
            performer_pairs: input.performer_pairs.clone(),
        });
        let moved = apply.moved;
        let track_id = existing_id.unwrap_or_else(Uuid::new_v4);
        let artwork_hash = upsert_artwork(tx, input.artwork.as_ref(), now).await?;
        // The credits arrive already split, already paired with their sort
        // forms and already carrying the album-artist fallback — that is the
        // tag mapper's whole job, and it applies the reference's rules rather
        // than this file's old `;`-only cut. What is left here is turning
        // names into rows.
        let artist_names: Vec<String> = credit_names(&credits, crate::tags::Role::Artist);
        let mut artist_ids = Vec::with_capacity(artist_names.len());
        for artist in &artist_names {
            artist_ids.push(upsert_artist(tx, pid, library_id, artist, now).await?);
        }
        let album_artist_names: Vec<String> =
            credit_names(&credits, crate::tags::Role::AlbumArtist);
        // The album keeps the joined string the file wrote as its display
        // name, and hangs off the first credited artist.
        let album_artist = input
            .album_artist
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| (album_artist_names.len() == 1).then(|| album_artist_names[0].clone()))
            .or_else(|| (!album_artist_names.is_empty()).then(|| album_artist_names.join("; ")));
        let mut album_artist_ids = Vec::with_capacity(album_artist_names.len());
        for name in &album_artist_names {
            album_artist_ids.push(upsert_artist(tx, pid, library_id, name, now).await?);
        }
        // The album still names one artist in the frozen `artistId` field, and
        // that is the first credit. The rest reach clients through the album's
        // participants rather than through this column.
        let album_artist_id = album_artist_ids.first().copied();
        // The sort form of each credit *on this track*. A credit the track tag
        // says nothing about falls back to the album artist tag when the two
        // name the same artist — ALBUMARTISTSORT is the more commonly written
        // of the two, and matching on the name rather than on position means
        // this cannot file one artist under another's sort form.
        let credit_sorts = {
            let track_sorts = credit_sort_names(&credits, crate::tags::Role::Artist);
            let album_sorts = credit_sort_names(&credits, crate::tags::Role::AlbumArtist);
            artist_names
                .iter()
                .zip(track_sorts)
                .map(|(name, sort)| {
                    sort.or_else(|| {
                        album_artist_names
                            .iter()
                            .position(|candidate| candidate == name)
                            .and_then(|at| album_sorts.get(at).cloned().flatten())
                    })
                })
                .collect::<Vec<_>>()
        };
        let album_id = upsert_album(
            tx,
            pid,
            library_id,
            input,
            album_artist.as_deref(),
            album_artist_id,
            artwork_hash.as_deref(),
            now,
        )
        .await?;
        let genres = split_values(input.genre.as_deref());
        let mut genre_ids = Vec::with_capacity(genres.len());
        for genre in &genres {
            genre_ids.push(upsert_genre(tx, library_id, genre, now).await?);
        }

        sqlx::query(
            "INSERT INTO track (id, library_id, album_id, artwork_hash, relative_path, file_size, \
               file_modified_at, quick_hash, full_hash, title, album_title, artist_display, genre_display, \
               year, track_number, disc_number, duration_ms, bitrate, sample_rate, channels, bit_depth, \
               codec, musical_key, tag_rating, musicbrainz_recording_id, musicbrainz_release_id, \
               musicbrainz_artist_id, replay_gain_track_gain, replay_gain_track_peak, \
               replay_gain_album_gain, replay_gain_album_peak, bpm, sort_title, sort_album, \
               comment, isrc, \
               moods, explicit_status, \
               lyrics_hash, is_available, last_seen_scan_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                     ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, \
                     1, ?, ?, ?) \
             ON CONFLICT (id) DO UPDATE SET album_id=excluded.album_id, artwork_hash=excluded.artwork_hash, \
               relative_path=excluded.relative_path, file_size=excluded.file_size, \
               file_modified_at=excluded.file_modified_at, quick_hash=excluded.quick_hash, \
               full_hash=excluded.full_hash, title=excluded.title, album_title=excluded.album_title, \
               artist_display=excluded.artist_display, genre_display=excluded.genre_display, year=excluded.year, \
               track_number=excluded.track_number, disc_number=excluded.disc_number, duration_ms=excluded.duration_ms, \
               bitrate=excluded.bitrate, sample_rate=excluded.sample_rate, channels=excluded.channels, \
               bit_depth=excluded.bit_depth, codec=excluded.codec, musical_key=excluded.musical_key, \
               tag_rating=excluded.tag_rating, \
               musicbrainz_recording_id=excluded.musicbrainz_recording_id, \
               musicbrainz_release_id=excluded.musicbrainz_release_id, \
               musicbrainz_artist_id=excluded.musicbrainz_artist_id, \
               replay_gain_track_gain=excluded.replay_gain_track_gain, \
               replay_gain_track_peak=excluded.replay_gain_track_peak, \
               replay_gain_album_gain=excluded.replay_gain_album_gain, \
               replay_gain_album_peak=excluded.replay_gain_album_peak, \
               bpm=excluded.bpm, sort_title=excluded.sort_title, sort_album=excluded.sort_album, \
               comment=excluded.comment, \
               isrc=excluded.isrc, moods=excluded.moods, \
               explicit_status=excluded.explicit_status, \
               lyrics_hash=excluded.lyrics_hash, is_available=1, last_seen_scan_id=excluded.last_seen_scan_id, \
               updated_at=excluded.updated_at",
        )
        .bind(track_id.to_string()).bind(library_id.to_string())
        .bind(album_id.map(|id| id.to_string())).bind(artwork_hash.as_deref())
        .bind(&input.relative_path).bind(input.file_size).bind(input.modified_at)
        .bind(&input.quick_hash).bind(&input.full_hash).bind(&input.title)
        .bind(input.album.as_deref()).bind(input.artist.as_deref()).bind(input.genre.as_deref())
        .bind(input.year).bind(input.track_number).bind(input.disc_number).bind(input.duration_ms)
        .bind(input.bitrate).bind(input.sample_rate).bind(input.channels).bind(input.bit_depth)
        .bind(input.codec.as_deref()).bind(input.musical_key.as_deref()).bind(input.tag_rating)
        .bind(input.musicbrainz_recording_id.as_deref())
        .bind(input.musicbrainz_release_id.as_deref())
        .bind(input.musicbrainz_artist_id.as_deref())
        .bind(input.replay_gain_track_gain).bind(input.replay_gain_track_peak)
        .bind(input.replay_gain_album_gain).bind(input.replay_gain_album_peak)
        .bind(input.bpm).bind(input.sort_title.as_deref()).bind(input.sort_album.as_deref())
        .bind(input.comment.as_deref())
        .bind(input.isrc.as_deref())
        .bind(input.moods.as_deref()).bind(input.explicit_status.as_deref())
        .bind(&input.lyrics_hash)
        .bind(scan_id.to_string()).bind(now).bind(now)
        .execute(&mut **tx).await?;

        sqlx::query("DELETE FROM track_lyrics WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        for (position, lyrics) in input.lyrics.iter().enumerate() {
            sqlx::query(
                "INSERT INTO track_lyrics \
                 (track_id, library_id, position, source, lang, synced, content) \
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(track_id.to_string())
            .bind(library_id.to_string())
            .bind(position as i64)
            .bind(lyrics.source)
            .bind(&lyrics.lang)
            .bind(i64::from(lyrics.synced))
            .bind(&lyrics.content)
            .execute(&mut **tx)
            .await?;
        }

        // Every credit the file names, not just its artists. The delete comes
        // first for the reason the reference gives: an artist who was both an
        // album artist and a composer, and is now only a composer, has to lose
        // the row that said otherwise.
        sqlx::query("DELETE FROM track_participant WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        for credit in &credits {
            let artist_id = match credit.role {
                crate::tags::Role::Artist => artist_ids
                    .get(credit.position)
                    .copied()
                    .expect("every artist credit was upserted above"),
                crate::tags::Role::AlbumArtist => album_artist_ids
                    .get(credit.position)
                    .copied()
                    .expect("every album-artist credit was upserted above"),
                _ => upsert_artist(tx, pid, library_id, &credit.name, now).await?,
            };
            let sort_name = match credit.role {
                crate::tags::Role::Artist => credit_sorts.get(credit.position).cloned().flatten(),
                _ => credit.sort_name.clone(),
            };
            // A file can credit the same artist twice under one role — two
            // spellings folding onto one row — and the second insert would
            // then collide on the key. The first credit wins, as it does in
            // tag order.
            sqlx::query(
                "INSERT INTO track_participant \
                   (track_id, artist_id, library_id, role, sub_role, position, sort_name) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (track_id, artist_id, role, sub_role) DO NOTHING",
            )
            .bind(track_id.to_string())
            .bind(artist_id.to_string())
            .bind(library_id.to_string())
            .bind(credit.role.as_str())
            .bind(&credit.sub_role)
            .bind(credit.position as i64)
            .bind(sort_name)
            .execute(&mut **tx)
            .await?;
        }
        sqlx::query("DELETE FROM track_genre WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        for genre_id in &genre_ids {
            sqlx::query(
                "INSERT INTO track_genre (track_id, genre_id, library_id) VALUES (?, ?, ?)",
            )
            .bind(track_id.to_string())
            .bind(genre_id.to_string())
            .bind(library_id.to_string())
            .execute(&mut **tx)
            .await?;
        }
        sqlx::query("DELETE FROM track_fts WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        // Every participant's name, not just the track artist's. The artist
        // index lists only artists an album is credited to, and the argument
        // for that is that a composer stays reachable by search — which is
        // only true if the search can see the composer's name.
        let searchable = {
            let mut names: Vec<&str> = credits.iter().map(|credit| credit.name.as_str()).collect();
            names.sort_unstable();
            names.dedup();
            if let Some(display) = input.artist.as_deref() {
                names.push(display);
            }
            names.join(" ")
        };
        sqlx::query("INSERT INTO track_fts (track_id, library_id, title, album, artists, genres) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(track_id.to_string()).bind(library_id.to_string()).bind(&input.title)
            .bind(input.album.as_deref()).bind(&searchable).bind(input.genre.as_deref())
            .execute(&mut **tx).await?;
        Ok(if existing_id.is_none() {
            ApplyOutcome::Added
        } else if moved {
            ApplyOutcome::Moved
        } else {
            ApplyOutcome::Updated
        })
    }

    pub async fn mark_unseen_unavailable(
        &self,
        library_id: Uuid,
        scan_id: Uuid,
    ) -> Result<i64, sqlx::Error> {
        let _writer = self.writer_guard().await;
        let result = sqlx::query(
            "UPDATE track SET is_available = 0, updated_at = ? \
             WHERE library_id = ? AND is_available = 1 \
               AND (last_seen_scan_id IS NULL OR last_seen_scan_id <> ?)",
        )
        .bind(now_ms())
        .bind(library_id.to_string())
        .bind(scan_id.to_string())
        .execute(self.pool())
        .await?;
        Ok(result.rows_affected() as i64)
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
        let mut tx = self.pool().begin().await?;
        let now = now_ms();
        for (path, message) in errors {
            sqlx::query("INSERT INTO scan_error (scan_id, relative_path, message, created_at) VALUES (?, ?, ?, ?)")
                .bind(scan_id.to_string()).bind(path).bind(message).bind(now)
                .execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list_tracks_for_user(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<Vec<TrackRecord>, sqlx::Error> {
        fetch_tracks(self, user_id, library_id, None, 0, 500).await
    }

    pub async fn search_tracks_for_user(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        query: &str,
    ) -> Result<Vec<TrackRecord>, sqlx::Error> {
        fetch_tracks(self, user_id, library_id, Some(query), 0, 200).await
    }

    pub async fn browse_tracks_for_user(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        query: Option<&str>,
        offset: i64,
        limit: i64,
    ) -> Result<Vec<TrackRecord>, sqlx::Error> {
        fetch_tracks(self, user_id, library_id, query, offset, limit).await
    }

    pub async fn stream_track_for_user(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<Option<StreamTrack>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT t.id, t.library_id, l.root_path, t.relative_path, t.full_hash, \
               t.codec, t.bitrate, t.duration_ms, t.is_available FROM track t \
             JOIN library l ON l.id = t.library_id \
             JOIN library_member m ON m.library_id = t.library_id \
             WHERE t.id = ? AND m.user_id = ?",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await?;
        row.map(stream_track_from_row).transpose()
    }
}

/// Turns free-form user input into an FTS5 `MATCH` expression.
///
/// Every term is quoted so punctuation cannot be read as FTS syntax, and terms
/// are ANDed so extra words narrow the result. Returns `None` when the input
/// carries no searchable term — `MATCH ''` is a SQLite error, not an empty
/// result.
///
/// The trailing term also matches as a prefix, because search-as-you-type is
/// how clients actually query: the user has typed "ech" and expects "Echo",
/// and a bare token match returns nothing until the word is complete. Earlier
/// terms stay exact — "dark side of the m" should narrow, not widen, and
/// prefixing every token would make short words match far too much.
pub(crate) fn fts_prefix_query(query: &str) -> Option<String> {
    let mut terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .peekable();
    let mut expression = String::new();
    while let Some(term) = terms.next() {
        if !expression.is_empty() {
            expression.push_str(" AND ");
        }
        expression.push('"');
        expression.push_str(&term.replace('"', "\"\""));
        expression.push('"');
        if terms.peek().is_none() {
            expression.push('*');
        }
    }
    (!expression.is_empty()).then_some(expression)
}

async fn fetch_tracks(
    db: &Database,
    user: Uuid,
    library: Uuid,
    query: Option<&str>,
    offset: i64,
    limit: i64,
) -> Result<Vec<TrackRecord>, sqlx::Error> {
    // Same prefix behaviour as /api/v2/search: this is the same user gesture,
    // typed into a library rather than the whole catalogue.
    let rows = if let Some(fts_query) = query.and_then(fts_prefix_query) {
        sqlx::query("SELECT t.id, t.library_id, t.relative_path, t.title, t.album_title, t.artist_display, \
            (SELECT tp.artist_id FROM track_participant tp \
              WHERE tp.track_id=t.id AND tp.role='artist' \
             ORDER BY tp.position LIMIT 1) AS artist_id, \
            t.genre_display, t.duration_ms, t.codec, t.artwork_hash, t.full_hash, t.is_available FROM track t \
            JOIN library_member m ON m.library_id=t.library_id JOIN track_fts f ON f.track_id=t.id \
            WHERE m.user_id=? AND t.library_id=? AND track_fts MATCH ? ORDER BY rank, t.id LIMIT ? OFFSET ?")
            .bind(user.to_string()).bind(library.to_string()).bind(fts_query).bind(limit).bind(offset).fetch_all(db.pool()).await?
    } else {
        sqlx::query("SELECT t.id, t.library_id, t.relative_path, t.title, t.album_title, t.artist_display, \
            (SELECT tp.artist_id FROM track_participant tp \
              WHERE tp.track_id=t.id AND tp.role='artist' \
             ORDER BY tp.position LIMIT 1) AS artist_id, \
            t.genre_display, t.duration_ms, t.codec, t.artwork_hash, t.full_hash, t.is_available FROM track t \
            JOIN library_member m ON m.library_id=t.library_id WHERE m.user_id=? AND t.library_id=? \
            ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?")
            .bind(user.to_string()).bind(library.to_string()).bind(limit).bind(offset).fetch_all(db.pool()).await?
    };
    rows.into_iter().map(track_from_row).collect()
}

async fn upsert_artwork(
    tx: &mut Transaction<'_, Sqlite>,
    artwork: Option<&ArtworkInput>,
    now: i64,
) -> Result<Option<String>, sqlx::Error> {
    let Some(artwork) = artwork else {
        return Ok(None);
    };
    sqlx::query("INSERT INTO artwork (hash, format, source, byte_size, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (hash) DO NOTHING")
        .bind(&artwork.hash).bind(&artwork.format).bind(&artwork.source).bind(artwork.byte_size).bind(now)
        .execute(&mut **tx).await?;
    Ok(Some(artwork.hash.clone()))
}

/// A scanned file, seen through the identity engine.
///
/// The album artist is the resolved display string rather than the raw tag,
/// because that is what the reference hashes: the fallback to `Various
/// Artists` or to the track's own artists is part of what makes two files the
/// same album.
///
/// The disc and track numbers are rendered once here rather than looked up as
/// tags, and they matter: without them the default track spec would identify
/// two tracks of one album by title alone, and an album holding two takes of
/// the same song would lose one.
///
/// A spec naming a tag the scanner does not read resolves empty, and an empty
/// attribute simply lets the next group answer. That is the fall-through the
/// grammar is built around, not a gap.
struct TrackIdentity<'a> {
    input: &'a CatalogTrackInput,
    album_artist: &'a str,
    folder: &'a str,
    disc_number: String,
    track_number: String,
}

impl<'a> TrackIdentity<'a> {
    fn new(input: &'a CatalogTrackInput, album_artist: &'a str) -> Self {
        Self {
            input,
            album_artist,
            folder: input
                .relative_path
                .rsplit_once(['/', '\\'])
                .map(|(folder, _)| folder)
                .unwrap_or(""),
            disc_number: input.disc_number.map(|v| v.to_string()).unwrap_or_default(),
            track_number: input
                .track_number
                .map(|v| v.to_string())
                .unwrap_or_default(),
        }
    }
}

impl crate::pid::PidSource for TrackIdentity<'_> {
    fn tag(&self, name: &str) -> Option<&str> {
        let value = match name {
            "musicbrainz_albumid" => self.input.musicbrainz_release_id.as_deref(),
            "musicbrainz_trackid" => self.input.musicbrainz_recording_id.as_deref(),
            "musicbrainz_artistid" => self.input.musicbrainz_artist_id.as_deref(),
            "albumartist" => self.input.album_artist.as_deref(),
            "artist" => self.input.artist.as_deref(),
            "genre" => self.input.genre.as_deref(),
            "discnumber" => Some(self.disc_number.as_str()),
            "tracknumber" => Some(self.track_number.as_str()),
            "isrc" => self.input.isrc.as_deref(),
            _ => None,
        };
        value.filter(|value| !value.is_empty())
    }

    fn folder(&self) -> &str {
        self.folder
    }

    fn album_artist_display(&self) -> &str {
        self.album_artist
    }

    fn title(&self) -> &str {
        &self.input.title
    }

    fn album(&self) -> &str {
        self.input.album.as_deref().unwrap_or_default()
    }
}

/// Upserts one artist row, whose id is the identity of its name.
///
/// The name reaches the row twice: as written, which is what a client renders,
/// and folded through `canonical_name`, which is what search matches on. The
/// id is neither — it is the identity hash, so two spellings that differ by a
/// real character are two artists, as they are in the reference.
async fn upsert_artist(
    tx: &mut Transaction<'_, Sqlite>,
    pid: &crate::pid::PidSpecs,
    library: Uuid,
    name: &str,
    now: i64,
) -> Result<Uuid, sqlx::Error> {
    let name = name.trim();
    let id = pid.artist_id(library, name);
    let canonical = waveflow_core::scanner::canonical_name(name);
    sqlx::query(
        "INSERT INTO artist (id, library_id, name, canonical_name, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET name=excluded.name, \
         canonical_name=excluded.canonical_name, updated_at=excluded.updated_at",
    )
    .bind(id.to_string())
    .bind(library.to_string())
    .bind(name)
    .bind(canonical)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

async fn upsert_genre(
    tx: &mut Transaction<'_, Sqlite>,
    library: Uuid,
    name: &str,
    now: i64,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let canonical = waveflow_core::scanner::canonical_name(name);
    let value: String = sqlx::query_scalar("INSERT INTO genre (id, library_id, name, canonical_name, created_at) VALUES (?, ?, ?, ?, ?) \
        ON CONFLICT (library_id, canonical_name) DO UPDATE SET name=excluded.name RETURNING id")
        .bind(id.to_string()).bind(library.to_string()).bind(name.trim()).bind(canonical).bind(now)
        .fetch_one(&mut **tx).await?;
    parse_uuid(value)
}

#[allow(clippy::too_many_arguments)]
/// Upserts one album row, whose id is the identity of the tags that name it.
///
/// The identity key this replaces embedded the row id of the album's first
/// credited artist, which is what made "Live" by `A; B` and "Live" by `A; C`
/// answer the same key — and what needed a special case to tell them apart.
/// The engine hashes the credit as written, so the two are simply different
/// and nothing has to notice.
async fn upsert_album(
    tx: &mut Transaction<'_, Sqlite>,
    pid: &crate::pid::PidSpecs,
    library: Uuid,
    input: &CatalogTrackInput,
    album_artist: Option<&str>,
    album_artist_id: Option<Uuid>,
    artwork: Option<&str>,
    now: i64,
) -> Result<Option<Uuid>, sqlx::Error> {
    let Some(title) = input.album.as_deref().filter(|s| !s.trim().is_empty()) else {
        return Ok(None);
    };
    let canonical = waveflow_core::scanner::canonical_name(title);
    let id = pid.album_id(
        library,
        &TrackIdentity::new(input, album_artist.unwrap_or_default()),
    );
    sqlx::query(
        "INSERT INTO album (id, library_id, title, canonical_title, album_artist_id, \
           album_artist_name, is_compilation, year, artwork_hash, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT (id) DO UPDATE SET title=excluded.title, \
           canonical_title=excluded.canonical_title, \
           album_artist_id=excluded.album_artist_id, \
           album_artist_name=excluded.album_artist_name, \
           is_compilation=excluded.is_compilation, \
           year=COALESCE(excluded.year, album.year), \
           artwork_hash=COALESCE(excluded.artwork_hash, album.artwork_hash), \
           updated_at=excluded.updated_at",
    )
    .bind(id.to_string())
    .bind(library.to_string())
    .bind(title.trim())
    .bind(canonical)
    .bind(album_artist_id.map(|id| id.to_string()))
    .bind(album_artist)
    .bind(i64::from(input.is_compilation))
    .bind(input.year)
    .bind(artwork)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(Some(id))
}

fn split_values(raw: Option<&str>) -> Vec<String> {
    raw.into_iter()
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn library_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LibraryRecord, sqlx::Error> {
    Ok(LibraryRecord {
        id: parse_uuid(row.try_get("id")?)?,
        name: row.try_get("name")?,
        root_path: PathBuf::from(row.try_get::<String, _>("root_path")?),
    })
}

fn existing_track_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ExistingTrack, sqlx::Error> {
    Ok(ExistingTrack {
        id: parse_uuid(row.try_get("id")?)?,
        relative_path: row.try_get("relative_path")?,
        file_size: row.try_get("file_size")?,
        modified_at: row.try_get("file_modified_at")?,
        quick_hash: row.try_get("quick_hash")?,
        full_hash: row.try_get("full_hash")?,
        lyrics_hash: row.try_get("lyrics_hash")?,
        available: row.try_get::<i64, _>("is_available")? != 0,
    })
}

fn stream_track_from_row(row: sqlx::sqlite::SqliteRow) -> Result<StreamTrack, sqlx::Error> {
    Ok(StreamTrack {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        library_root: PathBuf::from(row.try_get::<String, _>("root_path")?),
        relative_path: row.try_get("relative_path")?,
        full_hash: row.try_get("full_hash")?,
        codec: row.try_get("codec")?,
        bitrate: row.try_get("bitrate")?,
        duration_ms: row.try_get("duration_ms")?,
        available: row.try_get::<i64, _>("is_available")? != 0,
    })
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
        current_path: row.try_get("current_path")?,
        message: row.try_get("message")?,
    })
}

fn track_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TrackRecord, sqlx::Error> {
    Ok(TrackRecord {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        relative_path: row.try_get("relative_path")?,
        title: row.try_get("title")?,
        album: row.try_get("album_title")?,
        artist: row.try_get("artist_display")?,
        artist_id: row
            .try_get::<Option<String>, _>("artist_id")?
            .map(parse_uuid)
            .transpose()?,
        genre: row.try_get("genre_display")?,
        duration_ms: row.try_get("duration_ms")?,
        codec: row.try_get("codec")?,
        artwork_hash: row.try_get("artwork_hash")?,
        full_hash: row.try_get("full_hash")?,
        available: row.try_get::<i64, _>("is_available")? != 0,
    })
}

fn parse_uuid(value: String) -> Result<Uuid, sqlx::Error> {
    Uuid::from_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

/// The credits of one role, in tag order.
///
/// The two lists derived from it are read in lockstep — a name and the sort
/// form paired with it — so they have to come from one ordering.
fn credits_in_role(
    credits: &[crate::tags::Credit],
    role: crate::tags::Role,
) -> Vec<&crate::tags::Credit> {
    let mut named: Vec<&crate::tags::Credit> = credits
        .iter()
        .filter(|credit| credit.role == role)
        .collect();
    named.sort_by_key(|credit| credit.position);
    named
}

fn credit_names(credits: &[crate::tags::Credit], role: crate::tags::Role) -> Vec<String> {
    credits_in_role(credits, role)
        .iter()
        .map(|credit| credit.name.clone())
        .collect()
}

fn credit_sort_names(
    credits: &[crate::tags::Credit],
    role: crate::tags::Role,
) -> Vec<Option<String>> {
    credits_in_role(credits, role)
        .iter()
        .map(|credit| credit.sort_name.clone())
        .collect()
}
