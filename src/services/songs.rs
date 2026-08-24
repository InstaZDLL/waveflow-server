//! Song listings, genres, starred sets and lyrics.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    /// Genres visible to the user, with the size of what each holds.
    ///
    /// Grouping is by `genre.canonical_name`, so one genre spelled differently
    /// across two libraries — or differing only in case — is a single row. The
    /// facade previously grouped the raw `genre_display` fragments, which
    /// listed "Rock" and "rock" as two genres with split counts.
    pub async fn list_genres(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
    ) -> Result<Vec<GenreItem>, ServiceError> {
        let folders = (!library_ids.is_empty()).then(|| {
            serde_json::to_string(library_ids).expect("UUID list serialization cannot fail")
        });
        Ok(sqlx::query(
            "SELECT MIN(g.name) AS name, COUNT(DISTINCT t.id) AS song_count, \
                    COUNT(DISTINCT t.album_id) AS album_count \
             FROM genre g JOIN library_member m ON m.library_id=g.library_id \
             JOIN track_genre tg ON tg.genre_id=g.id \
             JOIN track t ON t.id=tg.track_id AND t.is_available=1 \
             WHERE m.user_id=? AND (? IS NULL OR g.library_id IN (SELECT value FROM json_each(?))) \
             GROUP BY g.canonical_name ORDER BY name COLLATE NOCASE",
        )
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(|row| {
            Ok(GenreItem {
                name: row.try_get("name")?,
                song_count: row.try_get("song_count")?,
                album_count: row.try_get("album_count")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }

    /// Songs of one genre, ordered and paged in SQL.
    ///
    /// Matching is on `genre.canonical_name`, the same key `list_genres` groups
    /// by and `byGenre` filters on. It was `eq_ignore_ascii_case` against the
    /// joined display string, which folds case but not punctuation or spacing:
    /// `getGenres` answered one row for "Hip-Hop" and "Hip Hop", and asking for
    /// that row returned only the tracks spelled the way the caller happened to
    /// send. A client showed a genre it had just been given, and it was empty.
    pub async fn songs_by_genre(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
        genre: &str,
        page: BrowsePage,
    ) -> Result<Vec<SongItem>, ServiceError> {
        let folders = folder_filter(library_ids);
        let canonical = waveflow_core::scanner::canonical_name(genre);
        let mut songs = sqlx::query(concat!(
            song_select!(),
            song_folder_clause!(),
            song_genre_clause!(),
            " ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(&canonical)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(songs)
    }

    /// The available tracks of one library that belong to no album.
    ///
    /// A track without an album has no album id to be the `parent` of its
    /// Subsonic `child`, so it names its library instead. That was a
    /// dead end until now: browsing to that identifier listed the library's
    /// artists and nothing else, so a track reachable by search was reachable
    /// by no amount of browsing. Answering here is what makes the `parent`
    /// it already advertised true.
    ///
    /// `getMusicDirectory` has no offset to page a folder with, so the caller
    /// asks for a ceiling instead of a page: everything up to `limit`, in one
    /// answer. A library that holds more album-less tracks than that would
    /// build an unbounded response out of a request that cannot say how much
    /// it wants, so the ceiling is what keeps the answer finite — and the
    /// caller says so in the log rather than truncating in silence. The artist
    /// list this tail follows is still bounded only by the library.
    pub async fn songs_without_album(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        limit: i64,
    ) -> Result<Vec<SongItem>, ServiceError> {
        if limit <= 0 {
            return Err(ServiceError::Invalid);
        }
        let mut songs = sqlx::query(concat!(
            song_select!(),
            " AND t.library_id=? AND t.album_id IS NULL \
              ORDER BY t.title COLLATE NOCASE, t.id LIMIT ?"
        ))
        .bind(user_id.to_string())
        .bind(library_id.to_string())
        .bind(limit)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(songs)
    }

    /// A random selection, drawn in SQL rather than by shuffling the catalogue.
    ///
    /// The facade used to read every visible track, filter in Rust and shuffle
    /// the result to answer with ten. `ORDER BY RANDOM() LIMIT` asks SQLite for
    /// the same thing without materialising the rest, and the genre filter
    /// matches the canonical name like every other genre predicate.
    ///
    /// A reversed year range is how Subsonic asks for one, so the bounds are
    /// normalised rather than rejected.
    pub async fn random_songs(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
        genre: Option<&str>,
        from_year: Option<i64>,
        to_year: Option<i64>,
        limit: i64,
    ) -> Result<Vec<SongItem>, ServiceError> {
        if limit <= 0 || limit > MAX_BROWSE_LIMIT {
            return Err(ServiceError::Invalid);
        }
        let folders = folder_filter(library_ids);
        let canonical = genre.map(waveflow_core::scanner::canonical_name);
        let from = from_year.unwrap_or(i64::MIN);
        let to = to_year.unwrap_or(i64::MAX);
        let bounded = from_year.is_some() || to_year.is_some();
        let sql = match canonical.is_some() {
            true => concat!(
                song_select!(),
                song_folder_clause!(),
                song_genre_clause!(),
                song_year_clause!(),
                " ORDER BY RANDOM() LIMIT ?"
            ),
            false => concat!(
                song_select!(),
                song_folder_clause!(),
                song_year_clause!(),
                " ORDER BY RANDOM() LIMIT ?"
            ),
        };
        let mut statement = sqlx::query(sql)
            .bind(user_id.to_string())
            .bind(folders.as_deref())
            .bind(folders.as_deref());
        if let Some(canonical) = canonical.as_deref() {
            statement = statement.bind(canonical);
        }
        let mut songs = statement
            .bind(bounded)
            .bind(from.min(to))
            .bind(from.max(to))
            .bind(limit)
            .fetch_all(self.db.pool())
            .await?
            .into_iter()
            .map(song_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(songs)
    }

    /// Everything the account has starred, most recent first.
    ///
    /// The three projections already `LEFT JOIN user_star`, so this is the same
    /// read with the join made mandatory. The facade used to load the whole
    /// catalogue and look each starred id up inside it, which cost a full
    /// catalogue read to answer a list that is usually short.
    pub async fn starred(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
    ) -> Result<StarredCatalog, ServiceError> {
        let folders = folder_filter(library_ids);
        // One connection for the three projections and both relation
        // batches: five acquisitions from the pool answered the same
        // question, and spreading them risked five different snapshots.
        let mut connection = self.db.pool().acquire().await?;
        let artists = sqlx::query(concat!(
            artist_select!(album_count),
            " AND us.starred_at IS NOT NULL \
              AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY us.starred_at DESC, ar.id"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(artist_summary_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND us.starred_at IS NOT NULL \
              AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY us.starred_at DESC, al.id"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut connection, user_id, &mut albums).await?;
        let mut songs = sqlx::query(concat!(
            song_select!(),
            " AND us.starred_at IS NOT NULL",
            song_folder_clause!(),
            " ORDER BY us.starred_at DESC, t.id"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut connection, user_id, &mut songs).await?;
        Ok(StarredCatalog {
            artists,
            albums,
            songs,
        })
    }

    pub async fn songs_by_ids(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.songs_by_ids_on(&mut connection, user_id, ids).await
    }

    /// Lyrics for one visible, available track. A visible track with no lyrics
    /// returns an empty list; an unknown or foreign track is blurred as not
    /// found, matching the rest of the catalogue API.
    pub async fn lyrics(&self, user_id: Uuid, track_id: Uuid) -> Result<LyricsList, ServiceError> {
        let rows = sqlx::query(
            "SELECT t.id, t.title, t.artist_display, tl.lang, tl.synced, tl.content \
             FROM track t JOIN library_member m ON m.library_id=t.library_id \
             LEFT JOIN track_lyrics tl ON tl.track_id=t.id AND tl.library_id=t.library_id \
             WHERE m.user_id=? AND t.id=? AND t.is_available=1 \
             ORDER BY tl.position",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        lyrics_list_from_rows(track_id, rows)
    }

    /// Legacy Subsonic lookup by metadata. Matching stays tenant-scoped and
    /// deterministic; it is intentionally exact because fuzzy catalogue
    /// reconciliation is outside the v2 contract.
    pub async fn lyrics_by_metadata(
        &self,
        user_id: Uuid,
        artist: Option<&str>,
        title: Option<&str>,
    ) -> Result<Option<LyricsList>, ServiceError> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT t.id FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE m.user_id=? AND t.is_available=1 \
               AND (? IS NULL OR t.artist_display = ? COLLATE NOCASE) \
               AND (? IS NULL OR t.title = ? COLLATE NOCASE) \
               AND EXISTS (SELECT 1 FROM track_lyrics tl WHERE tl.track_id=t.id) \
             ORDER BY t.title COLLATE NOCASE, t.id LIMIT 1",
        )
        .bind(user_id.to_string())
        .bind(artist)
        .bind(artist)
        .bind(title)
        .bind(title)
        .fetch_optional(self.db.pool())
        .await?;
        let track_id = row
            .map(|id| {
                Uuid::parse_str(&id)
                    .map_err(|error| ServiceError::Database(sqlx::Error::Decode(error.into())))
            })
            .transpose()?;
        match track_id {
            Some(track_id) => self.lyrics(user_id, track_id).await.map(Some),
            None => Ok(None),
        }
    }

    pub(super) async fn songs_by_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        let songs = self
            .songs_by_ids_lenient_on(connection, user_id, ids)
            .await?;
        if songs.len() == ids.len() {
            Ok(songs)
        } else {
            Err(ServiceError::NotFound)
        }
    }

    pub(super) async fn songs_by_ids_lenient_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids_json = serde_json::to_string(ids).map_err(|_| ServiceError::Invalid)?;
        let rows = sqlx::query(concat!(
            song_select!(),
            " AND t.id IN (SELECT value FROM json_each(?))"
        ))
        .bind(user_id.to_string())
        .bind(ids_json)
        .fetch_all(&mut *connection)
        .await?;
        let mut resolved = rows
            .into_iter()
            .map(song_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(connection, user_id, &mut resolved).await?;
        let available = resolved
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        Ok(ids
            .iter()
            .filter_map(|id| available.get(id).cloned())
            .collect())
    }
}
