//! Track reads for an account: listing, searching, browsing and streaming,
//! and the bounds a search query is held to.

use std::path::PathBuf;

use serde::Serialize;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::database::Database;

use super::parse_uuid;

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
    /// Whether, and how far, a link in `relative_path` may be followed when
    /// the file is opened (RFC-021, links outside the root).
    pub link_policy: crate::link_roots::LinkPolicy,
    /// The track's ReplayGain tags, for a transcode asked to apply them.
    pub replay_gain: crate::replay_gain::ReplayGainTags,
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

    /// The file behind `track_id`, for an account that may play it.
    ///
    /// **The account must not be disabled** (RFC-023 decision 4). A bearer
    /// token of a disabled account is refused before it gets here, but a
    /// stream ticket is sealed, not looked up, and lives an hour, and a share
    /// streams as its owner: this is the one read all three redeem through, so
    /// it is where a disabled account stops playing.
    pub async fn stream_track_for_user(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<Option<StreamTrack>, sqlx::Error> {
        let row = sqlx::query(
            "SELECT t.id, t.library_id, l.root_path, t.relative_path, t.full_hash, \
               t.codec, t.bitrate, t.duration_ms, t.is_available, \
               t.replay_gain_track_gain, t.replay_gain_track_peak, \
               t.replay_gain_album_gain, t.replay_gain_album_peak, \
               l.follow_links, (SELECT json_group_array(o.root_path) FROM library o WHERE o.id <> l.id) AS other_roots FROM track t \
             JOIN library l ON l.id = t.library_id \
             JOIN library_member m ON m.library_id = t.library_id \
             JOIN account a ON a.id = m.user_id \
             WHERE t.id = ? AND m.user_id = ? AND a.disabled = 0",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.pool())
        .await?;
        row.map(stream_track_from_row).transpose()
    }
}

/// The most characters of a search query that are read.
pub const MAX_SEARCH_QUERY_CHARS: usize = 256;

/// The most terms of a search query that are matched.
pub const MAX_SEARCH_TERMS: usize = 16;

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
///
/// **Bounded** (RFC-023 decision 5): the first [`MAX_SEARCH_QUERY_CHARS`]
/// characters, and of those the first [`MAX_SEARCH_TERMS`] terms. Each term is
/// one more phrase FTS5 intersects, for each of the three kinds a search
/// answers, and nothing bounded either but the request body — some seven
/// hundred kilobytes through a Subsonic form. Past the bounds the query is
/// narrowed, not refused, so no client is answered an error it never had: a
/// search that long names what it wants well before its end.
pub(crate) fn fts_prefix_query(query: &str) -> Option<String> {
    let query = match query.char_indices().nth(MAX_SEARCH_QUERY_CHARS) {
        Some((end, _)) => &query[..end],
        None => query,
    };
    let mut terms = query
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .take(MAX_SEARCH_TERMS)
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
        link_policy: crate::link_roots::LinkPolicy::from_columns(
            row.try_get("follow_links")?,
            row.try_get::<Option<String>, _>("other_roots")?.as_deref(),
        ),
        replay_gain: crate::replay_gain::ReplayGainTags {
            track_gain: row.try_get("replay_gain_track_gain")?,
            track_peak: row.try_get("replay_gain_track_peak")?,
            album_gain: row.try_get("replay_gain_album_gain")?,
            album_peak: row.try_get("replay_gain_album_peak")?,
        },
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
