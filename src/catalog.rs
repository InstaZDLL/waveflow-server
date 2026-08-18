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

    pub async fn start_scan_job(&self, scan_id: Uuid, total: i64) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let now = now_ms();
        sqlx::query(
            "UPDATE scan_job SET status = 'running', total_files = ?, started_at = ? WHERE id = ?",
        )
        .bind(total)
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
                Self::apply_catalog_track_in_transaction(&mut tx, library_id, scan_id, apply, now)
                    .await?,
            );
        }
        tx.commit().await?;
        Ok(outcomes)
    }

    async fn apply_catalog_track_in_transaction(
        tx: &mut Transaction<'_, Sqlite>,
        library_id: Uuid,
        scan_id: Uuid,
        apply: &CatalogApply,
        now: i64,
    ) -> Result<ApplyOutcome, sqlx::Error> {
        let input = &apply.input;
        let existing_id = apply.existing_id;
        let moved = apply.moved;
        let track_id = existing_id.unwrap_or_else(Uuid::new_v4);
        let artwork_hash = upsert_artwork(tx, input.artwork.as_ref(), now).await?;
        let artist_names = split_values(input.artist.as_deref());
        let mut artist_ids = Vec::with_capacity(artist_names.len());
        for artist in &artist_names {
            artist_ids.push(upsert_artist(tx, library_id, artist, now).await?);
        }
        let album_artist = input
            .album_artist
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(str::to_owned)
            .or_else(|| input.is_compilation.then(|| "Various Artists".to_owned()))
            .or_else(|| artist_names.first().cloned());
        let album_artist_id = match album_artist.as_deref() {
            Some(name) => Some(upsert_artist(tx, library_id, name, now).await?),
            None => None,
        };
        let album_id = upsert_album(
            tx,
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
               codec, musical_key, tag_rating, lyrics_hash, is_available, last_seen_scan_id, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?) \
             ON CONFLICT (id) DO UPDATE SET album_id=excluded.album_id, artwork_hash=excluded.artwork_hash, \
               relative_path=excluded.relative_path, file_size=excluded.file_size, \
               file_modified_at=excluded.file_modified_at, quick_hash=excluded.quick_hash, \
               full_hash=excluded.full_hash, title=excluded.title, album_title=excluded.album_title, \
               artist_display=excluded.artist_display, genre_display=excluded.genre_display, year=excluded.year, \
               track_number=excluded.track_number, disc_number=excluded.disc_number, duration_ms=excluded.duration_ms, \
               bitrate=excluded.bitrate, sample_rate=excluded.sample_rate, channels=excluded.channels, \
               bit_depth=excluded.bit_depth, codec=excluded.codec, musical_key=excluded.musical_key, \
               tag_rating=excluded.tag_rating, lyrics_hash=excluded.lyrics_hash, is_available=1, last_seen_scan_id=excluded.last_seen_scan_id, \
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

        sqlx::query("DELETE FROM track_artist WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        for (position, artist_id) in artist_ids.iter().enumerate() {
            sqlx::query("INSERT INTO track_artist (track_id, artist_id, library_id, position) VALUES (?, ?, ?, ?)")
                .bind(track_id.to_string()).bind(artist_id.to_string()).bind(library_id.to_string())
                .bind(position as i64).execute(&mut **tx).await?;
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
        sqlx::query("INSERT INTO track_fts (track_id, library_id, title, album, artists, genres) VALUES (?, ?, ?, ?, ?, ?)")
            .bind(track_id.to_string()).bind(library_id.to_string()).bind(&input.title)
            .bind(input.album.as_deref()).bind(input.artist.as_deref()).bind(input.genre.as_deref())
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
            (SELECT ta.artist_id FROM track_artist ta WHERE ta.track_id=t.id AND ta.position=0 \
             ORDER BY ta.position LIMIT 1) AS artist_id, \
            t.genre_display, t.duration_ms, t.codec, t.artwork_hash, t.full_hash, t.is_available FROM track t \
            JOIN library_member m ON m.library_id=t.library_id JOIN track_fts f ON f.track_id=t.id \
            WHERE m.user_id=? AND t.library_id=? AND track_fts MATCH ? ORDER BY rank, t.id LIMIT ? OFFSET ?")
            .bind(user.to_string()).bind(library.to_string()).bind(fts_query).bind(limit).bind(offset).fetch_all(db.pool()).await?
    } else {
        sqlx::query("SELECT t.id, t.library_id, t.relative_path, t.title, t.album_title, t.artist_display, \
            (SELECT ta.artist_id FROM track_artist ta WHERE ta.track_id=t.id AND ta.position=0 \
             ORDER BY ta.position LIMIT 1) AS artist_id, \
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

async fn upsert_artist(
    tx: &mut Transaction<'_, Sqlite>,
    library: Uuid,
    name: &str,
    now: i64,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let canonical = waveflow_core::scanner::canonical_name(name);
    let value: String = sqlx::query_scalar("INSERT INTO artist (id, library_id, name, canonical_name, created_at, updated_at) \
        VALUES (?, ?, ?, ?, ?, ?) ON CONFLICT (library_id, canonical_name) DO UPDATE SET name=excluded.name, updated_at=excluded.updated_at RETURNING id")
        .bind(id.to_string()).bind(library.to_string()).bind(name.trim()).bind(canonical).bind(now).bind(now)
        .fetch_one(&mut **tx).await?;
    parse_uuid(value)
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
async fn upsert_album(
    tx: &mut Transaction<'_, Sqlite>,
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
    let identity = format!(
        "{}:{}",
        canonical,
        album_artist_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "none".into())
    );
    let id = Uuid::new_v4();
    let value: String = sqlx::query_scalar("INSERT INTO album (id, library_id, title, canonical_title, identity_key, album_artist_id, album_artist_name, is_compilation, year, artwork_hash, created_at, updated_at) \
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (library_id, identity_key) DO UPDATE SET title=excluded.title, album_artist_name=excluded.album_artist_name, \
        is_compilation=excluded.is_compilation, year=COALESCE(excluded.year, album.year), artwork_hash=COALESCE(excluded.artwork_hash, album.artwork_hash), updated_at=excluded.updated_at RETURNING id")
        .bind(id.to_string()).bind(library.to_string()).bind(title.trim()).bind(canonical).bind(identity)
        .bind(album_artist_id.map(|id| id.to_string())).bind(album_artist).bind(i64::from(input.is_compilation))
        .bind(input.year).bind(artwork).bind(now).bind(now).fetch_one(&mut **tx).await?;
    parse_uuid(value).map(Some)
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
