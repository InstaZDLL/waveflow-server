//! The one static statement that evaluates every rule set, and the macros it
//! is assembled from.

/// A text comparison on one column, by the bound condition `c`. ASCII case is
/// folded, as `NOCASE` folds it everywhere else in the catalogue.
macro_rules! smart_text_test {
    ($column:literal) => {
        concat!(
            "(CASE c.o \
               WHEN 'eq' THEN lower(COALESCE(",
            $column,
            ", '')) = lower(c.v) \
               WHEN 'ne' THEN lower(COALESCE(",
            $column,
            ", '')) <> lower(c.v) \
               WHEN 'has' THEN instr(lower(COALESCE(",
            $column,
            ", '')), lower(c.v)) > 0 \
               WHEN 'hasnt' THEN instr(lower(COALESCE(",
            $column,
            ", '')), lower(c.v)) = 0 \
               WHEN 'sw' THEN substr(lower(COALESCE(",
            $column,
            ", '')), 1, length(c.v)) = lower(c.v) \
               WHEN 'ew' THEN length(COALESCE(",
            $column,
            ", '')) >= length(c.v) \
                 AND substr(lower(COALESCE(",
            $column,
            ", '')), -length(c.v)) = lower(c.v) \
               ELSE 0 END)"
        )
    };
}

/// A text comparison against a set of names, any one of which may match. The
/// negative operators hold when *none* does: "artist is not X" on a track
/// crediting X and Y is false.
///
/// `$from` is a `FROM … WHERE …` naming the set for the row `s`; `$name` and
/// `$canonical` its display and canonical columns.
macro_rules! smart_set_test {
    ($from:literal, $name:literal, $canonical:literal) => {
        concat!(
            "(CASE WHEN c.o IN ('ne', 'hasnt') THEN NOT EXISTS (SELECT 1 ",
            $from,
            " AND ",
            smart_set_match!($name, $canonical),
            ") ELSE EXISTS (SELECT 1 ",
            $from,
            " AND ",
            smart_set_match!($name, $canonical),
            ") END)"
        )
    };
}

macro_rules! smart_set_match {
    ($name:literal, $canonical:literal) => {
        concat!(
            "(CASE WHEN c.o IN ('eq', 'ne') THEN ",
            $canonical,
            " = c.k \
               WHEN c.o IN ('has', 'hasnt') THEN instr(lower(",
            $name,
            "), lower(c.v)) > 0 \
               WHEN c.o = 'sw' THEN substr(lower(",
            $name,
            "), 1, length(c.v)) = lower(c.v) \
               WHEN c.o = 'ew' THEN length(",
            $name,
            ") >= length(c.v) \
                 AND substr(lower(",
            $name,
            "), -length(c.v)) = lower(c.v) \
               ELSE 0 END)"
        )
    };
}

/// A numeric comparison on one column. A missing value fails every test but
/// `ne`, which is what "year is not 1999" means of an undated track.
macro_rules! smart_number_test {
    ($column:literal) => {
        concat!(
            "(CASE c.o \
               WHEN 'eq' THEN ",
            $column,
            " = c.n \
               WHEN 'ne' THEN ",
            $column,
            " IS NOT c.n \
               WHEN 'gt' THEN ",
            $column,
            " > c.n \
               WHEN 'lt' THEN ",
            $column,
            " < c.n \
               WHEN 'bt' THEN ",
            $column,
            " BETWEEN c.n AND c.m \
               ELSE 0 END)"
        )
    };
}

/// A moment comparison. "Not in the last 30 days" and "before" hold for a track
/// never played, which is what they mean of one.
macro_rules! smart_date_test {
    ($column:literal) => {
        concat!(
            "(CASE c.o \
               WHEN 'ge' THEN ",
            $column,
            " >= c.n \
               WHEN 'lt' THEN COALESCE(",
            $column,
            ", 0) < c.n \
               ELSE 0 END)"
        )
    };
}

/// Whether the bound condition `c` holds for the row `s`. Every branch is
/// chosen by comparing `c.f` with a literal; an unknown field matches nothing,
/// and cannot be reached anyway, because the parse refused it.
macro_rules! smart_predicate {
    () => {
        concat!(
            "COALESCE(CASE c.f \
               WHEN 'title' THEN ",
            smart_text_test!("s.title"),
            " \
               WHEN 'album' THEN ",
            smart_text_test!("s.album_title"),
            " \
               WHEN 'artist' THEN ",
            smart_set_test!(
                "FROM track_participant tp JOIN artist ar ON ar.id=tp.artist_id \
                 WHERE tp.track_id=s.id AND tp.role='artist'",
                "ar.name",
                "ar.canonical_name"
            ),
            " WHEN 'album_artist' THEN ",
            smart_set_test!(
                "FROM album_participant ap JOIN artist ar ON ar.id=ap.artist_id \
                 WHERE ap.album_id=s.album_id AND ap.role='albumartist'",
                "ar.name",
                "ar.canonical_name"
            ),
            " WHEN 'genre' THEN ",
            smart_set_test!(
                "FROM track_genre tg JOIN genre g ON g.id=tg.genre_id \
                 WHERE tg.track_id=s.id",
                "g.name",
                "g.canonical_name"
            ),
            " WHEN 'year' THEN ",
            smart_number_test!("s.year"),
            " \
               WHEN 'rating' THEN ",
            smart_number_test!("COALESCE(s.user_rating, 0)"),
            " \
               WHEN 'play_count' THEN ",
            smart_number_test!("s.play_count"),
            " \
               WHEN 'duration' THEN ",
            smart_number_test!("(s.duration_ms / 1000)"),
            " \
               WHEN 'bitrate' THEN ",
            smart_number_test!("s.bitrate"),
            " \
               WHEN 'last_played' THEN ",
            smart_date_test!("s.last_played_at"),
            " \
               WHEN 'date_added' THEN ",
            smart_date_test!("s.created_at"),
            " \
               WHEN 'starred' THEN ((s.starred_at IS NOT NULL) = (c.n = 1)) \
               WHEN 'format' THEN (CASE c.o \
                   WHEN 'eq' THEN lower(s.relative_path) LIKE '%.' || c.v \
                   ELSE lower(s.relative_path) NOT LIKE '%.' || c.v END) \
               WHEN 'library' THEN (CASE c.o \
                   WHEN 'eq' THEN s.library_id = c.v ELSE s.library_id <> c.v END) \
               ELSE 0 END, 0)"
        )
    };
}

/// One `ORDER BY` term per sort key and direction, numbered as
/// [`SmartSort::ALL`] numbers them. Only the term whose code is bound sorts;
/// the others are `NULL` for every row and change nothing. `NULLS LAST` on
/// each, so a track missing the value sorts after every track that has it,
/// whichever way the list runs.
macro_rules! smart_order_terms {
    () => {
        concat!(
            "CASE WHEN p.sort_key = 99 THEN random() END, \
             CASE WHEN p.sort_key = 1 THEN s.title END COLLATE NOCASE ASC NULLS LAST, \
             CASE WHEN p.sort_key = 2 THEN s.title END COLLATE NOCASE DESC NULLS LAST, \
             CASE WHEN p.sort_key = 3 THEN s.artist_display END COLLATE NOCASE ASC NULLS LAST, \
             CASE WHEN p.sort_key = 4 THEN s.artist_display END COLLATE NOCASE DESC NULLS LAST, \
             CASE WHEN p.sort_key = 5 THEN s.album_title END COLLATE NOCASE ASC NULLS LAST, \
             CASE WHEN p.sort_key = 6 THEN s.album_title END COLLATE NOCASE DESC NULLS LAST, \
             CASE WHEN p.sort_key = 7 THEN s.year END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 8 THEN s.year END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 9 THEN COALESCE(s.user_rating, 0) END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 10 THEN COALESCE(s.user_rating, 0) END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 11 THEN s.play_count END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 12 THEN s.play_count END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 13 THEN s.last_played_at END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 14 THEN s.last_played_at END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 15 THEN s.created_at END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 16 THEN s.created_at END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 17 THEN s.duration_ms END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 18 THEN s.duration_ms END DESC NULLS LAST, \
             CASE WHEN p.sort_key = 19 THEN s.bitrate END ASC NULLS LAST, \
             CASE WHEN p.sort_key = 20 THEN s.bitrate END DESC NULLS LAST"
        )
    };
}

/// The evaluation, whole. Binds, in order:
///
/// 1. the owner's account id (`p.uid`, read again as a second tenancy check);
/// 2. the connective, `all` or `any`;
/// 3. the bound conditions, one JSON document;
/// 4. how many conditions it holds;
/// 5. the sort code;
/// 6. the owner's account id again, for `song_select!`;
/// 7. the limit.
///
/// No other value reaches it, and none of those is ever SQL text.
pub(super) const EVALUATE: &str = concat!(
    "WITH p AS (SELECT ? AS uid, ? AS mode, ? AS conditions, ? AS cnt, ? AS sort_key), \
     cond AS (SELECT json_extract(value, '$.f') AS f, json_extract(value, '$.o') AS o, \
                     json_extract(value, '$.v') AS v, json_extract(value, '$.k') AS k, \
                     json_extract(value, '$.n') AS n, json_extract(value, '$.m') AS m \
              FROM json_each((SELECT conditions FROM p))) \
     SELECT s.id FROM p, (",
    song_select!(),
    ") AS s \
     WHERE s.library_id IN (SELECT lm.library_id FROM library_member lm WHERE lm.user_id = p.uid) \
       AND (p.cnt = 0 \
        OR (p.mode = 'all' AND NOT EXISTS (SELECT 1 FROM cond c WHERE NOT ",
    smart_predicate!(),
    ")) \
        OR (p.mode = 'any' AND EXISTS (SELECT 1 FROM cond c WHERE ",
    smart_predicate!(),
    "))) \
     ORDER BY ",
    smart_order_terms!(),
    ", s.title COLLATE NOCASE, s.id LIMIT ?"
);
