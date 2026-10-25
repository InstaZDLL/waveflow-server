//! Artist listings and detail.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    /// Artists visible to the user, paginated, each with its album count.
    pub async fn list_artists(
        &self,
        user_id: Uuid,
        library_id: Option<Uuid>,
        page: BrowsePage,
    ) -> Result<Vec<ArtistSummary>, ServiceError> {
        let library = library_id.map(|id| id.to_string());
        Ok(sqlx::query(concat!(
            artist_select!(album_count),
            " AND (? IS NULL OR ar.library_id=?) \
              ORDER BY ar.name COLLATE NOCASE, ar.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(library.as_deref())
        .bind(library.as_deref())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_summary_from_row)
        .collect::<Result<Vec<_>, _>>()?)
    }

    /// One artist with the albums it is credited on as album artist.
    pub async fn artist(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
    ) -> Result<ArtistDetail, ServiceError> {
        let summary = self.artist_summary(user_id, artist_id).await?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND EXISTS (SELECT 1 FROM album_participant ap \
                 WHERE ap.album_id=al.id AND ap.artist_id=? AND ap.role='albumartist') \
              ORDER BY al.year NULLS LAST, al.title COLLATE NOCASE, al.id"
        ))
        .bind(user_id.to_string())
        .bind(artist_id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        let (banner_hash, image_choice) = self.artist_image_choices(artist_id).await?;
        Ok(ArtistDetail {
            artist: summary.artist,
            album_count: summary.album_count,
            albums,
            appears_on: None,
            enrichment: None,
            banner_hash,
            image_choice,
        })
    }

    /// The albums an artist appears on without being one of their artists
    /// (RFC-022 decision 3), in the order [`Self::artist`] lists its own.
    ///
    /// A credit in the track's `artist` role, on an available track: a
    /// composer, producer or performer credit is a different relation, and a
    /// page listing every record someone engineered would be a credits page,
    /// not a discography. An album the artist is album artist of is never
    /// listed here, however many of its tracks also name them.
    ///
    /// The caller has resolved the artist for this user already, so an artist
    /// in another library never reaches this query; `album_select!` still
    /// binds the user first and passes every album through `library_member`.
    pub(super) async fn artist_appearances(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
    ) -> Result<Vec<AlbumItem>, ServiceError> {
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND al.id IN (SELECT t.album_id FROM track_participant tp \
                 JOIN track t ON t.id=tp.track_id \
                 WHERE tp.artist_id=? AND tp.role='artist' \
                   AND t.is_available=1 AND t.album_id IS NOT NULL) \
              AND NOT EXISTS (SELECT 1 FROM album_participant ap \
                 WHERE ap.album_id=al.id AND ap.artist_id=? AND ap.role='albumartist') \
              ORDER BY al.year NULLS LAST, al.title COLLATE NOCASE, al.id"
        ))
        .bind(user_id.to_string())
        .bind(artist_id.to_string())
        .bind(artist_id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        Ok(albums)
    }

    /// One artist the user can see, or [`ServiceError::NotFound`] — for one
    /// that does not exist and one in a library they are not a member of
    /// alike.
    pub(super) async fn artist_summary(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
    ) -> Result<ArtistSummary, ServiceError> {
        sqlx::query(concat!(artist_select!(album_count), " AND ar.id=?"))
            .bind(user_id.to_string())
            .bind(artist_id.to_string())
            .fetch_optional(self.db.pool())
            .await?
            .map(artist_summary_from_row)
            .transpose()?
            .ok_or(ServiceError::NotFound)
    }

    /// The account's most played tracks of one artist, counted from local
    /// history.
    ///
    /// The only part of an artist page WaveFlow can answer without a remote
    /// source: `play_event` already holds what this account played and when,
    /// and `song_select!` already projects it as `play_count`. What was missing
    /// was a way to ask the question per artist.
    ///
    /// **Played at least once, or absent.** A most-played list padded with
    /// tracks nobody has played is not a most-played list; it is the artist's
    /// discography in an arbitrary order wearing that name. An account with no
    /// history gets an empty list, which is the true answer.
    ///
    /// The counting is per account, like every other figure in the
    /// projections — `play_count` is correlated on `m.user_id`, so two members
    /// of one library see their own listening rather than the instance's.
    pub async fn artist_top_songs(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
        limit: i64,
    ) -> Result<Vec<SongItem>, ServiceError> {
        if limit <= 0 || limit > MAX_BROWSE_LIMIT {
            return Err(ServiceError::Invalid);
        }
        let mut connection = self.db.pool().acquire().await?;
        // Runs first and for its refusal: an artist in another account's
        // library answers as one that does not exist, rather than as an empty
        // list that would confirm the id.
        self.authorize_entity_on(&mut connection, user_id, "artist", artist_id)
            .await?;
        self.top_songs_on(&mut connection, user_id, &[artist_id], limit)
            .await
    }

    /// The same list, asked for by name — which is all `getTopSongs` sends.
    ///
    /// Matching is on `artist.canonical_name`, the one folding rule the rest of
    /// the catalogue already uses, so a client echoing a name back at the
    /// server finds the artist it was handed whatever the punctuation did on
    /// the way.
    ///
    /// A name can name more than one artist — one per library — and every one
    /// the account can reach contributes. Resolving to a single row would
    /// silently answer for one library while the caller asked about a name.
    /// An unknown name is an empty list rather than a refusal: the caller sent
    /// a string, not an identifier, so there is no existence to blur and
    /// nothing for an error code to say.
    pub async fn artist_top_songs_by_name(
        &self,
        user_id: Uuid,
        name: &str,
        limit: i64,
    ) -> Result<Vec<SongItem>, ServiceError> {
        if limit <= 0 || limit > MAX_BROWSE_LIMIT {
            return Err(ServiceError::Invalid);
        }
        let canonical = waveflow_core::scanner::canonical_name(name);
        let mut connection = self.db.pool().acquire().await?;
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT ar.id FROM artist ar \
             JOIN library_member m ON m.library_id=ar.library_id \
             WHERE m.user_id=? AND ar.canonical_name=? ORDER BY ar.id",
        )
        .bind(user_id.to_string())
        .bind(&canonical)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|id| {
            Uuid::parse_str(&id)
                .map_err(|error| ServiceError::Database(sqlx::Error::Decode(error.into())))
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.top_songs_on(&mut connection, user_id, &ids, limit)
            .await
    }

    /// What both forms of the question share. Which tracks are "this
    /// artist's" is [`artist_credit_clause!`], shared with the similar-songs
    /// draw so the definition exists once.
    async fn top_songs_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        artist_ids: &[Uuid],
        limit: i64,
    ) -> Result<Vec<SongItem>, ServiceError> {
        // No library narrowing: an artist id carries its own library, and a
        // name resolves only inside the ones this account reaches.
        let folders = folder_filter(&[]);
        let ids = serde_json::to_string(artist_ids).map_err(|_| ServiceError::Invalid)?;
        // The credit filter sits *inside* the projection and the play filter
        // outside it, and the split is not cosmetic. `play_count` is a
        // correlated count over `play_event` in the projection, so it is
        // computed for every row the inner query emits — and the wrapper can
        // only filter after that. `EXPLAIN QUERY PLAN` on both arrangements,
        // measured rather than assumed: either way the walk is over the
        // library's available tracks, but with the credit test inside, the
        // per-track work before filtering is one covering-index probe of
        // `track_participant`, and the count over the history runs for the
        // artist's own tracks alone. With it outside, the count runs for every
        // visible track — the same shape of degradation
        // `play_event_user_submission_track_idx` was added to keep away from
        // album listings. `play_count` itself cannot move inside: SQLite does
        // not accept a result alias in `WHERE`, which is what the wrapper is
        // for.
        let mut songs = sqlx::query(concat!(
            "SELECT * FROM (",
            song_select!(),
            song_folder_clause!(),
            artist_credit_clause!("SELECT value FROM json_each(?)"),
            ") AS s WHERE play_count > 0 \
             ORDER BY play_count DESC, title COLLATE NOCASE, id LIMIT ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(&ids)
        .bind(&ids)
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(connection, user_id, &mut songs).await?;
        Ok(songs)
    }
}
