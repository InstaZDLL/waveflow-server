//! Lyrics uploaded to a track, and the one order every reader chooses by
//! (RFC-012 decision 6).
//!
//! Uploads live in `track_lyrics_upload`, beside the scanner's
//! `track_lyrics`, and nothing here writes a file. The native lyrics route,
//! the properties panel and the download all read [`DomainServices::track_lyrics`],
//! so the order is decided once. The Subsonic façade keeps reading
//! [`DomainServices::lyrics`], which knows nothing of uploads: its contract is
//! frozen.

use std::str::FromStr;

use sqlx::Row;
use uuid::Uuid;

use super::{DomainServices, ServiceError};
use crate::auth::authentication::now_ms;
use crate::library::lyrics::{self, LyricsList, StructuredLyrics, UploadFormat};

/// The sidecar's cap, so an upload is held to what a scan would read.
pub const MAX_UPLOAD_BYTES: usize = 1024 * 1024;

/// One sheet, ready to be ranked.
struct Candidate {
    rank: u8,
    tie: u8,
    position: i64,
    sheet: StructuredLyrics,
}

/// A lyrics file to hand over as a download.
#[derive(Debug)]
pub struct LyricsFile {
    /// The name it is saved under, extension included.
    pub name: String,
    pub content: String,
}

impl DomainServices {
    /// A track's lyrics as the native API answers them: every sheet, uploaded
    /// ones included, in the order of RFC-012 decision 6, empty ones left
    /// out — so the first entry is always the sheet to show.
    ///
    /// 1. an uploaded `.ttml`;
    /// 2. an uploaded `.lrc` with timings;
    /// 3. a scanned synced sheet, a sidecar before embedded lyrics;
    /// 4. an uploaded `.txt`, then an uploaded `.lrc` without timings;
    /// 5. scanned plain text, a sidecar before embedded lyrics.
    pub async fn track_lyrics(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<LyricsList, ServiceError> {
        let track = sqlx::query(
            "SELECT t.title, t.artist_display FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE m.user_id=? AND t.id=? AND t.is_available=1",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        let display_title: String = track.try_get("title")?;
        let display_artist: Option<String> = track.try_get("artist_display")?;
        let sheet = |lang: String, synced: bool, lines, source: &str| StructuredLyrics {
            display_artist: display_artist.clone(),
            display_title: display_title.clone(),
            lang,
            synced,
            lines,
            source: Some(source.to_owned()),
        };

        let mut candidates = Vec::new();
        let scanned = sqlx::query(
            "SELECT source, lang, synced, content, position FROM track_lyrics \
             WHERE track_id=? ORDER BY position",
        )
        .bind(track_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        for row in scanned {
            let source: String = row.try_get("source")?;
            let synced = row.try_get::<i64, _>("synced")? != 0;
            let content: String = row.try_get("content")?;
            let lines = lyrics::lines(&content, synced);
            if !lyrics::has_text(&lines) {
                continue;
            }
            candidates.push(Candidate {
                rank: if synced { 2 } else { 4 },
                tie: u8::from(source == "embedded"),
                position: row.try_get("position")?,
                sheet: sheet(row.try_get("lang")?, synced, lines, &source),
            });
        }

        let uploads =
            sqlx::query("SELECT format, content FROM track_lyrics_upload WHERE track_id=?")
                .bind(track_id.to_string())
                .fetch_all(self.db.pool())
                .await?;
        for row in uploads {
            let Some(format) = UploadFormat::parse(row.try_get::<&str, _>("format")?) else {
                continue;
            };
            let content: String = row.try_get("content")?;
            // Read again rather than trusted: a row an earlier version stored
            // with nothing in it is skipped like any empty source.
            let Some(parsed) = lyrics::parse_upload(format, &content) else {
                continue;
            };
            let (rank, tie) = match (format, parsed.synced) {
                (UploadFormat::Ttml, _) => (0, 0),
                (UploadFormat::Lrc, true) => (1, 0),
                (UploadFormat::Txt, _) => (3, 0),
                (UploadFormat::Lrc, false) => (3, 1),
            };
            candidates.push(Candidate {
                rank,
                tie,
                position: 0,
                sheet: sheet(parsed.lang, parsed.synced, parsed.lines, format.source()),
            });
        }

        candidates.sort_by_key(|candidate| (candidate.rank, candidate.tie, candidate.position));
        // Last, and only into an empty answer: LRCLIB never stands beside a
        // sheet of the track's own (RFC-024 decision 2).
        if candidates.is_empty() {
            if let Some(sheet) = self.remote_sheet(user_id, track_id, true).await? {
                candidates.push(Candidate {
                    rank: 6,
                    tie: 0,
                    position: 0,
                    sheet,
                });
            }
        }
        Ok(LyricsList {
            track_id,
            structured_lyrics: candidates
                .into_iter()
                .map(|candidate| candidate.sheet)
                .collect(),
        })
    }

    /// Store an uploaded sheet, replacing any earlier upload in the same
    /// format, and answer the track's lyrics as they now read.
    ///
    /// Gated like a metadata correction — owner and manager — and refused with
    /// `Invalid` when the file holds nothing usable, so an empty sheet never
    /// stands in front of a real one. The bytes are decoded through
    /// `waveflow_core::text_encoding`, so a sheet saved in GB18030 or
    /// Shift_JIS reads as its author wrote it.
    pub async fn upload_lyrics(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        format: UploadFormat,
        bytes: &[u8],
    ) -> Result<LyricsList, ServiceError> {
        if bytes.is_empty() || bytes.len() > MAX_UPLOAD_BYTES {
            return Err(ServiceError::Invalid);
        }
        let content = waveflow_core::text_encoding::decode(bytes).into_owned();
        let parsed = lyrics::parse_upload(format, &content).ok_or(ServiceError::Invalid)?;
        {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let library_id = self
                .lyrics_writer_library(&mut tx, user_id, track_id)
                .await?;
            sqlx::query(
                "INSERT INTO track_lyrics_upload \
                   (track_id, library_id, format, lang, content, uploaded_by, uploaded_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT(track_id, format) DO UPDATE SET \
                   lang=excluded.lang, content=excluded.content, \
                   uploaded_by=excluded.uploaded_by, uploaded_at=excluded.uploaded_at",
            )
            .bind(track_id.to_string())
            .bind(library_id)
            .bind(format.as_str())
            .bind(&parsed.lang)
            .bind(&content)
            .bind(user_id.to_string())
            .bind(now_ms())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
        }
        self.track_lyrics(user_id, track_id).await
    }

    /// Remove one upload; the track falls back to what the scanner found.
    /// `NotFound` when the track has no upload in that format.
    pub async fn delete_lyrics_upload(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        format: UploadFormat,
    ) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        self.lyrics_writer_library(&mut tx, user_id, track_id)
            .await?;
        let removed = sqlx::query("DELETE FROM track_lyrics_upload WHERE track_id=? AND format=?")
            .bind(track_id.to_string())
            .bind(format.as_str())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;
        if removed == 0 {
            return Err(ServiceError::NotFound);
        }
        Ok(())
    }

    /// The lyrics as a file: one upload by name when `format` is given, and
    /// otherwise the sheet [`track_lyrics`](Self::track_lyrics) puts first.
    ///
    /// An upload comes back as it was uploaded. A scanned sheet is written as
    /// `.lrc` when it is synced and `.txt` when it is not.
    pub async fn lyrics_file(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        format: Option<UploadFormat>,
    ) -> Result<LyricsFile, ServiceError> {
        let list = self.track_lyrics(user_id, track_id).await?;
        let first = list.structured_lyrics.first();
        let stem = {
            let title = first
                .map(|sheet| sheet.display_title.trim().to_owned())
                .unwrap_or_default();
            match first
                .and_then(|sheet| sheet.display_artist.as_deref())
                .map(str::trim)
                .filter(|artist| !artist.is_empty())
            {
                Some(artist) => format!("{artist} - {title}"),
                None => title,
            }
        };
        let stem = if stem.is_empty() {
            "lyrics".to_owned()
        } else {
            stem
        };

        let upload = match format {
            Some(format) => Some(format),
            None => {
                let first = first.ok_or(ServiceError::NotFound)?;
                match first.source.as_deref() {
                    Some("upload_ttml") => Some(UploadFormat::Ttml),
                    Some("upload_lrc") => Some(UploadFormat::Lrc),
                    Some("upload_txt") => Some(UploadFormat::Txt),
                    _ => {
                        let (content, extension) = if first.synced {
                            (lyrics::to_lrc(&first.lines), "lrc")
                        } else {
                            (lyrics::to_text(&first.lines), "txt")
                        };
                        return Ok(LyricsFile {
                            name: format!("{stem}.{extension}"),
                            content,
                        });
                    }
                }
            }
        };
        let upload = upload.ok_or(ServiceError::NotFound)?;
        // Membership was proved by `track_lyrics` above, and the upload row
        // carries the track's library: reading it by track id is reading
        // within what the caller may see.
        let content: String = sqlx::query_scalar(
            "SELECT content FROM track_lyrics_upload WHERE track_id=? AND format=?",
        )
        .bind(track_id.to_string())
        .bind(upload.as_str())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        Ok(LyricsFile {
            name: format!("{stem}.{}", upload.as_str()),
            content,
        })
    }

    /// The track's library, if the caller may write its metadata. `NotFound`
    /// for a track they cannot see and `Forbidden` for one they may only read,
    /// which every surface blurs into the same `404`.
    async fn lyrics_writer_library(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<String, ServiceError> {
        let row = sqlx::query(
            // Available, as the read requires: lyrics stored for a track
            // nobody can read would answer 200 here and 404 there.
            "SELECT t.library_id, m.role FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE t.id=? AND m.user_id=? AND t.is_available=1",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(ServiceError::NotFound)?;
        let role = crate::storage::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_write_metadata() {
            return Err(ServiceError::Forbidden);
        }
        Ok(row.try_get("library_id")?)
    }
}
