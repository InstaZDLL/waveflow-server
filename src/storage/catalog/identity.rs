//! Catalogue identity: noticing a changed identifier spec, carrying artist
//! user data across it, matching a file to an existing track, and the
//! relocation hint a track spec evaluates to.

use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use crate::storage::database::Database;

use super::{credits::credit_names, parse_uuid, CatalogTrackInput};

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
    /// `None` on a row written before the relocation hint existed, which is
    /// what the backfill in the scanner's skip path watches for.
    pub pid: Option<String>,
}

impl Database {
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
        specs: &crate::library::pid::PidSpecs,
    ) -> Result<u64, sqlx::Error> {
        // `pid.track` is handled apart, and never by re-identifying anything:
        // a track's id is its path, then its content hash, then the relocation
        // hint — six tables cascade off it, so moving track ids would delete
        // playlists and play history rather than orphan them.
        //
        // What a changed track spec does invalidate is every stored hint. A
        // lookup compares a hint computed under the new spec against values
        // written under the old one, and those two are not separate namespaces:
        // two different specs can evaluate to the same string — `title` reading
        // "intro" and `album` reading "intro" — and the hint would then hand a
        // new file the identity, and the favourites, of an unrelated track. So
        // the stale values go, and the scanner's backfill puts fresh ones back
        // on the next ordinary scan. No full rescan is charged for it.
        self.clear_track_hints_if_spec_changed(specs).await?;
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
        // Before the rescan, while the artist rows still hold the identifiers
        // their favourites name.
        if changed.iter().any(|(key, _, _)| *key == "pid.artist") {
            let moved = self.remap_artist_user_data(specs).await?;
            if moved > 0 {
                tracing::warn!(
                    rows = moved,
                    "favourites and ratings moved onto the artist identifiers the new rule derives"
                );
            }
        }
        let libraries = self.request_full_scan_everywhere().await?;
        tracing::warn!(
            libraries,
            "every library will be rescanned in full so its identifiers follow the new rule"
        );
        Ok(libraries)
    }

    /// Drops every relocation hint when the track spec no longer matches the
    /// one the catalogue's hints were written under.
    ///
    /// Deliberately not paired with a full rescan: nothing is re-identified,
    /// and the scanner refills a missing hint from its skip path, so one
    /// ordinary scan restores what this clears. `pid IS NOT NULL` keeps a boot
    /// that changes nothing from writing anything, and keeps the repeat boots
    /// before that first scan free.
    async fn clear_track_hints_if_spec_changed(
        &self,
        specs: &crate::library::pid::PidSpecs,
    ) -> Result<(), sqlx::Error> {
        let active = specs.track.source();
        // A catalogue built before the property existed is not evidence of a
        // different rule, only of an older server — the same reading the album
        // and artist specs get below.
        match self.server_property("pid.track").await? {
            None => return Ok(()),
            Some(stored) if stored == active => return Ok(()),
            Some(stored) => {
                tracing::warn!(
                    setting = "pid.track",
                    %stored,
                    %active,
                    "track spec changed; dropping the relocation hints written under the old one"
                );
            }
        }
        let _writer = self.writer_guard().await;
        let cleared = sqlx::query("UPDATE track SET pid = NULL WHERE pid IS NOT NULL")
            .execute(self.pool())
            .await?
            .rows_affected();
        if cleared > 0 {
            tracing::warn!(
                cleared,
                "relocation hints dropped; the next scan writes them again under the new spec"
            );
        }
        Ok(())
    }

    /// Carries artist favourites and ratings across a change of artist spec.
    ///
    /// `user_star` and `user_rating` hold an untyped identifier with no foreign
    /// key, so a rescan that re-derives artist ids leaves their rows pointing at
    /// identifiers nothing answers for — invisible rather than wrong, since every
    /// projection resolves through an `EXISTS`, but lost all the same. The old
    /// identifier and the name that produced it are both still on the `artist`
    /// row at this point, which is the only moment the two can be paired.
    ///
    /// Albums are not remapped and cannot be: their spec reads `albumversion`
    /// and `releasedate`, which live on the files rather than on the album row,
    /// so there is nothing here to derive the new identifier from.
    ///
    /// **Moved in two phases, through a namespace no identifier can occupy.**
    /// One artist's new identifier can be another's old one, and moving them one
    /// at a time would then carry the first artist's favourite onto the second's
    /// row — a result that depends on the order the rows came back in, which is
    /// no result at all. Staging every row first and landing them afterwards
    /// makes the outcome the same whatever that order was. Both phases run in
    /// one transaction, so a staged value is never visible to anything.
    ///
    /// `UPDATE OR IGNORE` then `DELETE` at each phase: a coarser spec can fold
    /// two artists onto one identifier, and the second row would collide on
    /// `(user_id, entity_type, entity_id)`. Losing the duplicate is right — the
    /// user already stars what it would have become.
    async fn remap_artist_user_data(
        &self,
        specs: &crate::library::pid::PidSpecs,
    ) -> Result<u64, sqlx::Error> {
        let rows = sqlx::query("SELECT id, library_id, name FROM artist")
            .fetch_all(self.pool())
            .await?;
        let mut moves = Vec::new();
        for row in rows {
            let old: String = row.try_get("id")?;
            let library_id = parse_uuid(row.try_get("library_id")?)?;
            let name: String = row.try_get("name")?;
            let new = specs.artist_id(library_id, &name).to_string();
            if new != old {
                // A UUID holds no colon, so nothing that reaches these columns
                // by any other route can be mistaken for a staged value.
                moves.push((old, format!("{REMAP_STAGE}{new}"), new));
            }
        }
        if moves.is_empty() {
            return Ok(0);
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        let mut moved = 0;
        for (from, to) in moves
            .iter()
            .map(|(old, staged, _)| (old, staged))
            .chain(moves.iter().map(|(_, staged, new)| (staged, new)))
        {
            let landed = move_artist_user_data(&mut tx, from, to).await?;
            // Counted on the second pass only, where a row reaches the
            // identifier it will actually be read under.
            if from.starts_with(REMAP_STAGE) {
                moved += landed;
            }
        }
        tx.commit().await?;
        Ok(moved)
    }

    pub async fn existing_track_by_path(
        &self,
        library_id: Uuid,
        relative_path: &str,
    ) -> Result<Option<ExistingTrack>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT id, relative_path, file_size, file_modified_at, quick_hash, full_hash, lyrics_hash, is_available, pid \
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
            "SELECT id, relative_path, file_size, file_modified_at, quick_hash, full_hash, lyrics_hash, is_available, pid \
             FROM track WHERE library_id = ? AND full_hash = ?",
        )
        .bind(library_id.to_string())
        .bind(full_hash)
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(existing_track_from_row).collect()
    }

    /// Tracks carrying one relocation hint.
    ///
    /// Only rows that already answered a scan under the current spec have a
    /// `pid` at all, and the column is nullable, so this never widens to
    /// "every track whose hint was never computed".
    pub async fn relocation_candidates_by_pid(
        &self,
        library_id: Uuid,
        pid: Uuid,
    ) -> Result<Vec<ExistingTrack>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT id, relative_path, file_size, file_modified_at, quick_hash, full_hash, lyrics_hash, is_available, pid \
             FROM track WHERE library_id = ? AND pid = ?",
        )
        .bind(library_id.to_string())
        .bind(pid.to_string())
        .fetch_all(self.pool())
        .await?;
        rows.into_iter().map(existing_track_from_row).collect()
    }

    /// Gives a hint to rows that predate the column.
    ///
    /// An ordinary scan skips a file it has already seen unchanged, so without
    /// this an existing catalogue would carry no hints at all until someone
    /// asked for a full scan — and the feature would be inert on exactly the
    /// installations that have favourites worth keeping. `pid IS NULL` in the
    /// predicate makes it a one-time cost: the second scan finds nothing to do.
    pub async fn backfill_track_pids(&self, hints: &[(Uuid, Uuid)]) -> Result<(), sqlx::Error> {
        if hints.is_empty() {
            return Ok(());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        for (track_id, pid) in hints {
            sqlx::query("UPDATE track SET pid = ? WHERE id = ? AND pid IS NULL")
                .bind(pid.to_string())
                .bind(track_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }
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
pub(super) struct TrackIdentity<'a> {
    input: &'a CatalogTrackInput,
    album_artist: &'a str,
    folder: &'a str,
    disc_number: String,
    track_number: String,
}

impl<'a> TrackIdentity<'a> {
    pub(super) fn new(input: &'a CatalogTrackInput, album_artist: &'a str) -> Self {
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

/// The credits a file's tags imply.
///
/// Derived here rather than by the scanner because this is where identity is
/// written — and now also where the relocation hint is, which has to hash the
/// same values the row is built from.
pub(super) fn track_credits(input: &CatalogTrackInput) -> Vec<crate::library::tags::Credit> {
    crate::library::tags::credits(&crate::library::tags::RawCredits {
        artist: input.artist.iter().cloned().collect(),
        artists: input.artists.clone(),
        album_artist: input.album_artist.iter().cloned().collect(),
        album_artists: input.album_artists.clone(),
        sort_artist: input.sort_artist.clone(),
        sort_album_artist: input.sort_album_artist.clone(),
        is_compilation: input.is_compilation,
        roles: input.roles.clone(),
        performer_pairs: input.performer_pairs.clone(),
    })
}

/// The album keeps the joined string the file wrote as its display name, and
/// hangs off the first credited artist.
pub(super) fn album_artist_display(
    input: &CatalogTrackInput,
    credits: &[crate::library::tags::Credit],
) -> Option<String> {
    let names = credit_names(credits, crate::library::tags::Role::AlbumArtist);
    input
        .album_artist
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| (names.len() == 1).then(|| names[0].clone()))
        .or_else(|| (!names.is_empty()).then(|| names.join("; ")))
}

/// The relocation hint for one file: the track spec evaluated over its tags.
///
/// Never an identity. Track ids stay drawn at random — six tables cascade off
/// them, so moving one would delete play history rather than orphan it. This
/// only tells a scan that the file it is looking at is probably the one whose
/// path disappeared, when neither the path nor the content hash can say so.
pub fn track_pid(
    specs: &crate::library::pid::PidSpecs,
    library_id: Uuid,
    input: &CatalogTrackInput,
) -> Uuid {
    let credits = track_credits(input);
    let album_artist = album_artist_display(input, &credits).unwrap_or_default();
    specs.track_id(library_id, &TrackIdentity::new(input, &album_artist))
}

impl crate::library::pid::PidSource for TrackIdentity<'_> {
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

/// The namespace a remap stages through. A UUID holds no colon, so a staged
/// value cannot be mistaken for an identifier and no identifier for it.
const REMAP_STAGE: &str = "pid-remap:";

/// Moves one artist identifier onto another, in both tables that hold one.
///
/// Spelled out per table rather than looped: sqlx takes static SQL only, which
/// is what keeps every query in this crate injection-proof by construction.
async fn move_artist_user_data(
    tx: &mut Transaction<'_, Sqlite>,
    from: &str,
    to: &str,
) -> Result<u64, sqlx::Error> {
    let mut moved = sqlx::query(
        "UPDATE OR IGNORE user_star SET entity_id = ? \
         WHERE entity_type = 'artist' AND entity_id = ?",
    )
    .bind(to)
    .bind(from)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    sqlx::query("DELETE FROM user_star WHERE entity_type = 'artist' AND entity_id = ?")
        .bind(from)
        .execute(&mut **tx)
        .await?;
    moved += sqlx::query(
        "UPDATE OR IGNORE user_rating SET entity_id = ? \
         WHERE entity_type = 'artist' AND entity_id = ?",
    )
    .bind(to)
    .bind(from)
    .execute(&mut **tx)
    .await?
    .rows_affected();
    sqlx::query("DELETE FROM user_rating WHERE entity_type = 'artist' AND entity_id = ?")
        .bind(from)
        .execute(&mut **tx)
        .await?;
    Ok(moved)
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
        pid: row.try_get("pid")?,
    })
}
