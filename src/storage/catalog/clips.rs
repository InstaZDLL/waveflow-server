//! A track's music video as the scan recorded it (RFC-029 decision 3): read
//! once per scan, then the rows that changed written back.

use sqlx::Row;
use uuid::Uuid;

use crate::storage::database::Database;

use super::parse_uuid;

/// The link a track has to its clip: the file, relative to the library root,
/// and what the next scan compares to take it as confirmed without opening it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackClip {
    pub relative_path: String,
    /// The file's extension, lower-cased: `mp4`, `m4v`, `mov`, `webm`, `mkv`.
    pub format: String,
    /// `sibling`, `clips_dir` or `itself`.
    pub origin: &'static str,
    pub byte_size: i64,
    pub modified_at: i64,
}

/// One available track of a library and the link it has now, if any.
#[derive(Debug, Clone)]
pub struct TrackClipFacts {
    pub track_id: Uuid,
    pub clip: Option<TrackClip>,
}

/// Rows written per transaction, so a library of many clips does not hold the
/// writer gate for its whole pass.
const CLIP_BATCH: usize = 200;

fn origin(value: &str) -> Result<&'static str, sqlx::Error> {
    match value {
        "sibling" => Ok("sibling"),
        "clips_dir" => Ok("clips_dir"),
        "itself" => Ok("itself"),
        other => Err(sqlx::Error::Decode(
            format!("unknown clip origin {other:?}").into(),
        )),
    }
}

impl Database {
    /// Every available track of the library by its path, with the link it has.
    /// One query per scan, as `track_skip_facts` is.
    pub async fn track_clip_facts(
        &self,
        library_id: Uuid,
    ) -> Result<std::collections::HashMap<String, TrackClipFacts>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT t.id, t.relative_path, c.relative_path AS clip_path, c.format, c.origin, \
                    c.byte_size, c.modified_at \
             FROM track t LEFT JOIN track_clip c ON c.track_id = t.id \
             WHERE t.library_id = ? AND t.is_available = 1",
        )
        .bind(library_id.to_string())
        .fetch_all(self.pool())
        .await?;
        let mut facts = std::collections::HashMap::with_capacity(rows.len());
        for row in rows {
            let clip = match row.try_get::<Option<String>, _>("clip_path")? {
                Some(relative_path) => Some(TrackClip {
                    relative_path,
                    format: row.try_get("format")?,
                    origin: origin(&row.try_get::<String, _>("origin")?)?,
                    byte_size: row.try_get("byte_size")?,
                    modified_at: row.try_get("modified_at")?,
                }),
                None => None,
            };
            facts.insert(
                row.try_get("relative_path")?,
                TrackClipFacts {
                    track_id: parse_uuid(row.try_get("id")?)?,
                    clip,
                },
            );
        }
        Ok(facts)
    }

    /// Writes the links a scan's pass changed: a link set or replaced, or, for
    /// `None`, removed. `library_id` is read from the track in the writing
    /// transaction, never taken from the caller, as `track_canvas` does it; a
    /// track deleted since the pass read it writes nothing.
    pub async fn apply_track_clips(
        &self,
        changes: &[(Uuid, Option<TrackClip>)],
    ) -> Result<(), sqlx::Error> {
        for batch in changes.chunks(CLIP_BATCH) {
            let writer = self.writer_guard().await;
            let mut tx = self.begin_write(&writer).await?;
            for (track_id, clip) in batch {
                match clip {
                    Some(clip) => {
                        sqlx::query(
                            "INSERT INTO track_clip \
                               (track_id, library_id, relative_path, format, origin, byte_size, \
                                modified_at) \
                             SELECT id, library_id, ?, ?, ?, ?, ? FROM track WHERE id = ? \
                             ON CONFLICT (track_id) DO UPDATE SET \
                               relative_path = excluded.relative_path, \
                               format = excluded.format, origin = excluded.origin, \
                               byte_size = excluded.byte_size, \
                               modified_at = excluded.modified_at",
                        )
                        .bind(&clip.relative_path)
                        .bind(&clip.format)
                        .bind(clip.origin)
                        .bind(clip.byte_size)
                        .bind(clip.modified_at)
                        .bind(track_id.to_string())
                        .execute(&mut *tx)
                        .await?;
                    }
                    None => {
                        sqlx::query("DELETE FROM track_clip WHERE track_id = ?")
                            .bind(track_id.to_string())
                            .execute(&mut *tx)
                            .await?;
                    }
                }
            }
            tx.commit().await?;
        }
        Ok(())
    }
}
