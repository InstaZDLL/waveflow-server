//! The relations a projected row cannot carry, loaded in one batch per
//! listing: an album's credits, genres and disc titles, a song's credits,
//! genres and contributors.

use super::*;

/// Fills in the album relations OpenSubsonic expects on `AlbumID3`.
///
/// Both are derived from the album's own available tracks rather than stored:
/// an album has no genre or credit of its own in the schema, it has the union
/// of what its files carry. Loaded in one batch per relation like the song
/// relations, because an album listing is up to five hundred rows and a query
/// each would be a query per row.
///
/// Tenancy is re-checked in the query. The batch is keyed by album id alone, so
/// the `library_member` join is what stops an id from another account resolving
/// to real names.
pub(super) async fn attach_album_relations(
    connection: &mut SqliteConnection,
    user_id: Uuid,
    albums: &mut [AlbumItem],
) -> Result<(), sqlx::Error> {
    if albums.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(&albums.iter().map(|album| album.id).collect::<Vec<_>>())
        .expect("UUID list serialization cannot fail");
    let mut artists: HashMap<Uuid, Vec<ArtistRef>> = HashMap::new();
    for row in sqlx::query(
        // The album's own credits, which are its album artists — not the union
        // of its tracks' credits, which is what this used to answer. An album
        // with a guest on one track was reporting the guest as one of its
        // artists; the reference reports the two the album is credited to, and
        // leaves the guest to the track that names them.
        "SELECT ap.album_id, ar.id, ar.name \
         FROM album_participant ap \
         JOIN artist ar ON ar.id=ap.artist_id \
         JOIN library_member m ON m.library_id=ap.library_id \
         WHERE m.user_id=? AND ap.role='albumartist' \
           AND ap.album_id IN (SELECT value FROM json_each(?)) \
         ORDER BY ap.album_id, ap.position, ar.name COLLATE NOCASE, ar.id",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        artists
            .entry(parse_uuid(row.try_get("album_id")?)?)
            .or_default()
            .push(ArtistRef {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            });
    }
    let mut genres: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in sqlx::query(
        // Grouped on the canonical name for the same reason `list_genres` is:
        // otherwise one album spelling "Hip-Hop" on some tracks and "Hip Hop"
        // on others reports two genres.
        "SELECT t.album_id, MIN(g.name) AS name FROM track t \
         JOIN track_genre tg ON tg.track_id=t.id \
         JOIN genre g ON g.id=tg.genre_id \
         JOIN library_member m ON m.library_id=t.library_id \
         WHERE m.user_id=? AND t.is_available=1 \
           AND t.album_id IN (SELECT value FROM json_each(?)) \
         GROUP BY t.album_id, g.canonical_name \
         ORDER BY t.album_id, name COLLATE NOCASE",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        genres
            .entry(parse_uuid(row.try_get("album_id")?)?)
            .or_default()
            .push(row.try_get("name")?);
    }
    let mut disc_titles: HashMap<Uuid, Vec<DiscTitle>> = HashMap::new();
    for row in sqlx::query(
        // One title per disc, and the first spelling in disc order when the
        // tracks of one disc disagree — the same `MIN` the genres use, for the
        // same reason: an album must not report a disc twice because two of
        // its files were tagged by different hands.
        "SELECT t.album_id, t.disc_number, MIN(t.disc_subtitle) AS title FROM track t \
         JOIN library_member m ON m.library_id=t.library_id \
         WHERE m.user_id=? AND t.is_available=1 AND t.disc_subtitle IS NOT NULL \
           AND t.disc_number IS NOT NULL \
           AND t.album_id IN (SELECT value FROM json_each(?)) \
         GROUP BY t.album_id, t.disc_number \
         ORDER BY t.album_id, t.disc_number",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        disc_titles
            .entry(parse_uuid(row.try_get("album_id")?)?)
            .or_default()
            .push(DiscTitle {
                disc: row.try_get("disc_number")?,
                title: row.try_get("title")?,
            });
    }
    for album in albums {
        album.artists = artists.remove(&album.id).unwrap_or_default();
        album.genres = genres.remove(&album.id).unwrap_or_default();
        album.disc_titles = disc_titles.remove(&album.id).unwrap_or_default();
    }
    Ok(())
}

/// Fills in the relations a single projected row cannot carry.
///
/// `song_select!` collapses credited artists and genres into the display strings
/// the tags happened to contain; the structured form lives in `track_artist` and
/// `track_genre`, ordered and deduplicated by the scanner. Reading them per song
/// would be two queries per row on every listing, so both are fetched once for
/// the whole batch and distributed by track id.
///
/// Tenancy is re-checked here rather than inherited from the caller: the batch
/// is keyed by track id alone, and a join that trusted those ids would be the
/// one place in the read path where membership is not proven.
pub(super) async fn attach_song_relations(
    connection: &mut SqliteConnection,
    user_id: Uuid,
    songs: &mut [SongItem],
) -> Result<(), sqlx::Error> {
    if songs.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(&songs.iter().map(|song| song.id).collect::<Vec<_>>())
        .expect("UUID list serialization cannot fail");
    let mut artists: HashMap<Uuid, Vec<ArtistRef>> = HashMap::new();
    for row in sqlx::query(
        "SELECT tp.track_id, ar.id, ar.name FROM track_participant tp \
         JOIN artist ar ON ar.id=tp.artist_id \
         JOIN library_member m ON m.library_id=tp.library_id \
         WHERE m.user_id=? AND tp.role='artist' \
           AND tp.track_id IN (SELECT value FROM json_each(?)) \
         ORDER BY tp.track_id, tp.position",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        artists
            .entry(parse_uuid(row.try_get("track_id")?)?)
            .or_default()
            .push(ArtistRef {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            });
    }
    let mut genres: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in sqlx::query(
        // `track_genre` has no position column, so the order is the genre name.
        // It has to be deterministic: a client diffing two responses would
        // otherwise see a change that is not one.
        "SELECT tg.track_id, g.name FROM track_genre tg \
         JOIN genre g ON g.id=tg.genre_id \
         JOIN library_member m ON m.library_id=tg.library_id \
         WHERE m.user_id=? AND tg.track_id IN (SELECT value FROM json_each(?)) \
         ORDER BY tg.track_id, g.name COLLATE NOCASE, g.id",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        genres
            .entry(parse_uuid(row.try_get("track_id")?)?)
            .or_default()
            .push(row.try_get("name")?);
    }
    // Everything credited on the track that is neither its artist nor its
    // album artist. Ordered by role then position so two responses for one
    // track are byte-identical — the reference emits these in map-iteration
    // order and answers differently on every request.
    let mut contributors: HashMap<Uuid, Vec<Contributor>> = HashMap::new();
    for row in sqlx::query(
        "SELECT tp.track_id, tp.role, tp.sub_role, ar.id, ar.name \
         FROM track_participant tp \
         JOIN artist ar ON ar.id=tp.artist_id \
         JOIN library_member m ON m.library_id=tp.library_id \
         WHERE m.user_id=? AND tp.role NOT IN ('artist', 'albumartist') \
           AND tp.track_id IN (SELECT value FROM json_each(?)) \
         ORDER BY tp.track_id, tp.role, tp.position, tp.sub_role",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        let sub_role: String = row.try_get("sub_role")?;
        contributors
            .entry(parse_uuid(row.try_get("track_id")?)?)
            .or_default()
            .push(Contributor {
                role: row.try_get("role")?,
                sub_role: (!sub_role.is_empty()).then_some(sub_role),
                artist: ArtistRef {
                    id: parse_uuid(row.try_get("id")?)?,
                    name: row.try_get("name")?,
                },
            });
    }
    // The album's credit, which is not the track's: a guest appearance names
    // the guest while the album still belongs under its album artists. Keyed
    // on the album, so every track of one album answers the same list.
    let album_ids = serde_json::to_string(
        &songs
            .iter()
            .filter_map(|song| song.album_id)
            .collect::<Vec<_>>(),
    )
    .expect("UUID list serialization cannot fail");
    let mut album_artists: HashMap<Uuid, Vec<ArtistRef>> = HashMap::new();
    for row in sqlx::query(
        "SELECT ap.album_id, ar.id, ar.name FROM album_participant ap \
         JOIN artist ar ON ar.id=ap.artist_id \
         JOIN library_member m ON m.library_id=ap.library_id \
         WHERE m.user_id=? AND ap.role='albumartist' \
           AND ap.album_id IN (SELECT value FROM json_each(?)) \
         ORDER BY ap.album_id, ap.position, ar.name COLLATE NOCASE, ar.id",
    )
    .bind(user_id.to_string())
    .bind(&album_ids)
    .fetch_all(&mut *connection)
    .await?
    {
        album_artists
            .entry(parse_uuid(row.try_get("album_id")?)?)
            .or_default()
            .push(ArtistRef {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            });
    }
    for song in songs {
        song.artists = artists.remove(&song.id).unwrap_or_default();
        song.genres = genres.remove(&song.id).unwrap_or_default();
        song.contributors = contributors.remove(&song.id).unwrap_or_default();
        song.album_artists = song
            .album_id
            .and_then(|album| album_artists.get(&album).cloned())
            .unwrap_or_default();
    }
    Ok(())
}

pub(super) async fn fetch_songs(
    db: &Database,
    user_id: Uuid,
    folder_filter: Option<&str>,
    id: Option<Uuid>,
) -> Result<Vec<SongItem>, sqlx::Error> {
    let id = id.map(|id| id.to_string());
    let mut songs = sqlx::query(concat!(
        song_select!(),
        " AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
           AND (? IS NULL OR t.id=?) ORDER BY t.title COLLATE NOCASE"
    ))
    .bind(user_id.to_string())
    .bind(folder_filter)
    .bind(folder_filter)
    .bind(id.as_deref())
    .bind(id.as_deref())
    .fetch_all(db.pool())
    .await?
    .into_iter()
    .map(song_from_row)
    .collect::<Result<Vec<_>, _>>()?;
    attach_song_relations(&mut *db.pool().acquire().await?, user_id, &mut songs).await?;
    Ok(songs)
}
