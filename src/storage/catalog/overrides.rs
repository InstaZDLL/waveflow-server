//! Track corrections as a scan sees them: reading a track's corrected lists,
//! and writing the rows they imply.

use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use super::apply::{upsert_artist, upsert_genre};

/// The display string a correction implies, or the file's when there is none.
///
/// An empty list is a correction meaning "nobody", and joins to nothing — which
/// has to become absence rather than an empty string, or a track answers with a
/// blank name instead of with no name.
pub(crate) fn display_of(corrected: Option<&[String]>, scanned: Option<&str>) -> Option<String> {
    match corrected {
        None => scanned.map(str::to_owned),
        Some([]) => None,
        Some(names) => Some(names.join("; ")),
    }
}

/// The corrections a scan has to respect while re-deriving a track.
///
/// The title is here as well as the two lists, and not because the track row
/// needs it — the projection reads the correction over that row. The full-text
/// index does not have a projection: it is a copy, rebuilt from the file by
/// every full scan, so a correction that never reached it would leave the track
/// findable only under the name it was corrected away from.
#[derive(Debug, Clone, Default)]
pub(crate) struct TrackOverrideLists {
    pub title: Option<String>,
    pub artists: Option<Vec<String>>,
    pub genres: Option<Vec<String>>,
}

impl TrackOverrideLists {
    fn decode(raw: Option<String>) -> Result<Option<Vec<String>>, sqlx::Error> {
        raw.map(|value| {
            serde_json::from_str::<Vec<String>>(&value)
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))
        })
        .transpose()
    }
}

/// Reads a track's list corrections, if it carries any.
pub(crate) async fn track_override_lists(
    tx: &mut Transaction<'_, Sqlite>,
    track_id: Uuid,
) -> Result<TrackOverrideLists, sqlx::Error> {
    let row = sqlx::query("SELECT title, artists, genres FROM track_override WHERE track_id = ?")
        .bind(track_id.to_string())
        .fetch_optional(&mut **tx)
        .await?;
    let Some(row) = row else {
        return Ok(TrackOverrideLists::default());
    };
    Ok(TrackOverrideLists {
        title: row.try_get("title")?,
        artists: TrackOverrideLists::decode(row.try_get("artists")?)?,
        genres: TrackOverrideLists::decode(row.try_get("genres")?)?,
    })
}

/// Writes the rows an explicit list of artists or genres implies.
///
/// Deliberately not the tag mapper's path. That exists to guess where one name
/// ends in a string a tagger wrote however it liked; a correction arrives
/// already separated, so parsing it again would only reintroduce the ambiguity
/// it was made to settle.
///
/// Only the `artist` role is rewritten. A composer or a conductor comes from
/// the file and is untouched by someone correcting who performed the track,
/// which is why the delete is narrowed rather than wholesale.
pub(crate) async fn apply_track_override_lists(
    tx: &mut Transaction<'_, Sqlite>,
    pid: &crate::library::pid::PidSpecs,
    library_id: Uuid,
    track_id: Uuid,
    lists: &TrackOverrideLists,
    now: i64,
) -> Result<(), sqlx::Error> {
    if let Some(names) = &lists.artists {
        sqlx::query("DELETE FROM track_participant WHERE track_id = ? AND role = 'artist'")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        for (position, name) in names.iter().enumerate() {
            let artist_id = upsert_artist(tx, pid, library_id, name, now).await?;
            sqlx::query(
                "INSERT INTO track_participant \
                   (track_id, artist_id, library_id, role, sub_role, position, sort_name) \
                 VALUES (?, ?, ?, 'artist', '', ?, NULL) \
                 ON CONFLICT (track_id, artist_id, role, sub_role) DO NOTHING",
            )
            .bind(track_id.to_string())
            .bind(artist_id.to_string())
            .bind(library_id.to_string())
            .bind(position as i64)
            .execute(&mut **tx)
            .await?;
        }
    }
    if let Some(names) = &lists.genres {
        sqlx::query("DELETE FROM track_genre WHERE track_id = ?")
            .bind(track_id.to_string())
            .execute(&mut **tx)
            .await?;
        for name in names {
            let genre_id = upsert_genre(tx, library_id, name, now).await?;
            // A correction may repeat a genre as the tags may: one row.
            sqlx::query(
                "INSERT INTO track_genre (track_id, genre_id, library_id) VALUES (?, ?, ?) \
                 ON CONFLICT (track_id, genre_id) DO NOTHING",
            )
            .bind(track_id.to_string())
            .bind(genre_id.to_string())
            .bind(library_id.to_string())
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}
