//! The tenant-filtered SQL projections every catalogue read starts from, and
//! the library list a scoped one binds.
//!
//! Declared first in `services/mod.rs`, under `#[macro_use]`: a
//! `macro_rules!` is visible only textually after its definition, and every
//! sibling module composes these.

use uuid::Uuid;

/// Tenant-filtered projections shared by the Subsonic facade and the native
/// browse endpoints. Each expands to a literal ending at `WHERE m.user_id=?` so
/// callers `concat!` their own predicates onto it — sqlx only accepts static SQL,
/// which keeps these compositions injection-proof by construction. The first
/// bind is always the user id.
macro_rules! song_select {
    () => {
        "SELECT t.id, t.library_id, t.album_id, \
                COALESCE(ovr.title, t.title) AS title, t.album_title, t.artist_display, \
                (SELECT tp.artist_id FROM track_participant tp \
                  WHERE tp.track_id=t.id AND tp.role='artist' \
                  ORDER BY tp.position LIMIT 1) AS artist_id, \
                t.genre_display, COALESCE(ovr.year, t.year) AS year, \
                COALESCE(ovr.track_number, t.track_number) AS track_number, \
                COALESCE(ovr.disc_number, t.disc_number) AS disc_number, t.duration_ms, t.bitrate, \
                t.codec, t.relative_path, t.file_size, t.artwork_hash, t.full_hash, t.created_at, \
                us.starred_at, ur.rating AS user_rating, \
                t.sample_rate, t.channels, t.bit_depth, \
                (SELECT COUNT(*) FROM play_event pe \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pe.track_id=t.id) \
                 AS play_count, \
                (SELECT MAX(pe.played_at) FROM play_event pe \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pe.track_id=t.id) \
                 AS last_played_at, \
                COALESCE(ovr.musicbrainz_recording_id, t.musicbrainz_recording_id) \
                  AS musicbrainz_recording_id, \
                t.replay_gain_track_gain, t.replay_gain_track_peak, \
                t.replay_gain_album_gain, t.replay_gain_album_peak, t.bpm, \
                COALESCE(ovr.sort_title, t.sort_title) AS sort_title, \
                COALESCE(ovr.comment, t.comment) AS comment, t.isrc, t.moods, t.explicit_status, \
                alb.album_artist_name, alb.album_artist_id \
         FROM track t JOIN library_member m ON m.library_id=t.library_id \
         LEFT JOIN album alb ON alb.id=t.album_id \
         LEFT JOIN track_override ovr ON ovr.track_id=t.id \
         LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='track' AND us.entity_id=t.id \
         LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='track' AND ur.entity_id=t.id \
         WHERE m.user_id=? AND t.is_available=1"
    };
}

/// Narrows [`song_select!`] to an optional set of libraries. Binds the JSON
/// library list twice, as the album and artist scopes do.
macro_rules! song_folder_clause {
    () => {
        " AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?)))"
    };
}

/// Restricts [`song_select!`] to one genre, matched on `genre.canonical_name`
/// so case, punctuation and spacing fold exactly as they do in `getGenres` and
/// in the `byGenre` album filter. Binds the canonical name once.
macro_rules! song_genre_clause {
    () => {
        " AND t.id IN (SELECT tg.track_id FROM track_genre tg \
            JOIN genre g ON g.id=tg.genre_id WHERE g.canonical_name=?)"
    };
}

/// An optional inclusive year range. Binds a flag and the two bounds, so one
/// literal serves both the filtered and the unfiltered request rather than
/// forking the statement.
macro_rules! song_year_clause {
    () => {
        " AND (? = 0 OR (t.year IS NOT NULL AND t.year BETWEEN ? AND ?))"
    };
}

/// Restricts [`song_select!`] to an artist's tracks, given a subquery naming
/// the artist ids; binds whatever that subquery binds, twice.
///
/// Two credits count, and they are the two an artist page shows: a track that
/// names the artist in its own `artist` role, and a track whose album is
/// credited to the artist as album artist. The second is read from
/// `album_participant`, the same relation `DomainServices::artist` lists the
/// albums from — a classical record whose files credit performers per track
/// would otherwise have a page full of albums and no tracks.
macro_rules! artist_credit_clause {
    ($artist_ids:literal) => {
        concat!(
            " AND (EXISTS (SELECT 1 FROM track_participant tp \
                     WHERE tp.track_id=t.id AND tp.role='artist' \
                       AND tp.artist_id IN (",
            $artist_ids,
            ")) \
                OR EXISTS (SELECT 1 FROM album_participant ap \
                     WHERE ap.album_id=t.album_id AND ap.role='albumartist' \
                       AND ap.artist_id IN (",
            $artist_ids,
            ")))"
        )
    };
}

/// The song projection, wrapped so an ordering can name its own output.
///
/// Mirrors [`album_scope!`], and for the same reason: `song_select!` computes
/// `play_count`, `last_played_at`, `starred_at` and `user_rating` as output
/// columns, and an `ORDER BY` placed outside the subquery can name them.
/// Inside, each correlated subquery would have to be written twice — once in
/// the projection and once in the ordering — and the two copies would drift.
///
/// It folds in [`song_folder_clause!`], so the library narrowing binds twice
/// here rather than at every call site.
macro_rules! song_scope {
    () => {
        concat!(
            "SELECT * FROM (",
            song_select!(),
            song_folder_clause!(),
            ") AS s"
        )
    };
}

macro_rules! album_select {
    () => {
        "SELECT al.id, al.library_id, al.title, al.album_artist_name, al.album_artist_id, \
                al.artwork_hash, al.year, al.is_compilation, al.musicbrainz_id, al.sort_name, \
                al.original_release_date, al.release_date, al.release_types, al.record_labels, \
                al.created_at, us.starred_at, \
                ur.rating AS user_rating, \
                (SELECT COUNT(*) FROM play_event pe JOIN track pt ON pt.id=pe.track_id \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pt.album_id=al.id) AS play_count, \
                (SELECT MAX(pe.played_at) FROM play_event pe JOIN track pt ON pt.id=pe.track_id \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pt.album_id=al.id) AS last_played_at, \
                (SELECT COUNT(*) FROM track t2 WHERE t2.album_id=al.id AND t2.is_available=1) \
                 AS song_count, \
                (SELECT COALESCE(SUM(t2.duration_ms), 0) FROM track t2 \
                 WHERE t2.album_id=al.id AND t2.is_available=1) AS duration_ms \
         FROM album al JOIN library_member m ON m.library_id=al.library_id \
         LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='album' AND us.entity_id=al.id \
         LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='album' AND ur.entity_id=al.id \
         WHERE m.user_id=?"
    };
}

/// [`album_select!`] narrowed to an optional set of libraries and wrapped so a
/// caller can filter and order on the projected aggregates — `play_count`,
/// `last_played_at`, `song_count` — instead of repeating their subqueries.
/// SQLite does not accept a result alias in `WHERE`, hence the wrapper rather
/// than a longer predicate list. Binds are the user id, then the JSON library
/// list twice.
macro_rules! album_scope {
    () => {
        concat!(
            "SELECT * FROM (",
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?)))) AS a"
        )
    };
}

/// The artist projection, in the two shapes the catalogue reads it.
///
/// `artist_select!()` stops at the columns `ArtistItem` carries;
/// `artist_select!(album_count)` adds the count `ArtistSummary` needs, so a
/// browse that never renders it does not pay a correlated subquery per artist.
/// Both expand from the same column list on purpose: the browses that wanted
/// the short shape used to spell it out by hand, and one of those copies fell
/// a column behind the day this list gained one — the browse reading it then
/// failed on a column the query never selected, which nothing reading the
/// macro could have predicted.
macro_rules! artist_select {
    () => {
        artist_select!(@columns "")
    };
    (album_count) => {
        artist_select!(
            @columns ", COALESCE((SELECT ars.album_count FROM artist_role_stats ars \
                                   WHERE ars.artist_id=ar.id AND ars.role='albumartist'), 0) \
                      AS album_count"
        )
    };
    (@columns $extra:expr) => {
        concat!(
            // The effective portrait (RFC-013 decision 3): a hand-chosen one
            // first, then whatever the files or a provider gave the artist.
            "SELECT ar.id, ar.library_id, ar.name, \
                    COALESCE((SELECT c.artwork_hash FROM artist_image_choice c \
                               WHERE c.artist_id=ar.id AND c.kind='portrait'), \
                             ar.artwork_hash) AS artwork_hash, \
                    ar.musicbrainz_id, \
                    ar.sort_name, us.starred_at, ur.rating AS user_rating, \
                    (SELECT group_concat(role) FROM \
                       (SELECT ars.role FROM artist_role_stats ars \
                         WHERE ars.artist_id=ar.id ORDER BY ars.role)) AS roles",
            $extra,
            " FROM artist ar JOIN library_member m ON m.library_id=ar.library_id \
              LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='artist' AND us.entity_id=ar.id \
              LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='artist' AND ur.entity_id=ar.id \
              WHERE m.user_id=?"
        )
    };
}

/// The JSON library list a scoped projection binds, or `None` for "every
/// library the account can reach". Built once here because every scoped
/// query binds the same value twice and a second spelling of it would be a
/// second chance to get the empty case wrong.
pub(super) fn folder_filter(library_ids: &[Uuid]) -> Option<String> {
    (!library_ids.is_empty())
        .then(|| serde_json::to_string(library_ids).expect("UUID list serialization cannot fail"))
}
