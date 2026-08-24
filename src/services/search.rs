//! Directory browsing and search.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    /// The whole visible catalogue, each kind ordered and paged in SQL.
    ///
    /// Subsonic clients send the literal `""` to `search3` as the documented
    /// match-all query, and page through it to build their initial library.
    /// FTS5 has no expression meaning "everything", so this is not a search at
    /// all — it is three ordinary listings under the search response. It used
    /// to read the entire catalogue and slice it in Rust, once per page, which
    /// made a client's first synchronization quadratic in the library.
    ///
    /// A page beyond the end is an empty list rather than an error: that is how
    /// a client learns it has reached the end.
    pub async fn browse_all(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
        artists: BrowsePage,
        albums: BrowsePage,
        songs: BrowsePage,
    ) -> Result<CatalogSearch, ServiceError> {
        let folders = folder_filter(library_ids);
        let artists = sqlx::query(concat!(
            artist_select!(),
            " AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY ar.name COLLATE NOCASE, ar.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(artists.limit)
        .bind(artists.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY al.title COLLATE NOCASE, al.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(albums.limit)
        .bind(albums.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        let mut songs = sqlx::query(concat!(
            song_select!(),
            song_folder_clause!(),
            " ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(songs.limit)
        .bind(songs.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(CatalogSearch {
            artists,
            albums,
            songs,
        })
    }

    /// Full-text search across the user's visible catalogue. Tracks are matched
    /// through the FTS5 index built in M1, which folds case and diacritics, so
    /// "echo" finds "Écho". Albums and artists are derived from the same index
    /// rather than a second scan, keeping one source of truth for relevance.
    ///
    /// Each kind is paged independently, as `search3` has always allowed:
    /// a client that has read every matching song should be able to ask for
    /// the next page of songs without re-reading the artists beside them.
    pub async fn search(
        &self,
        user_id: Uuid,
        query: &str,
        artists: BrowsePage,
        albums: BrowsePage,
        songs: BrowsePage,
    ) -> Result<SearchResult, ServiceError> {
        // Prefix on the trailing term, like the Subsonic surface: a client
        // querying on each keystroke would otherwise get nothing until the word
        // is complete — "ech" returned zero results while "echo" returned the
        // album. Native clients type incrementally just as Subsonic ones do.
        let Some(fts) = crate::catalog::fts_prefix_query(query) else {
            return Ok(SearchResult {
                artists: Vec::new(),
                albums: Vec::new(),
                songs: Vec::new(),
            });
        };
        let mut songs = sqlx::query(concat!(
            song_select!(),
            " AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
              ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(songs.limit)
        .bind(songs.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND al.id IN (SELECT t.album_id FROM track t \
                WHERE t.album_id IS NOT NULL \
                  AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?)) \
              ORDER BY al.title COLLATE NOCASE, al.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(albums.limit)
        .bind(albums.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        let artists = sqlx::query(concat!(
            artist_select!(),
            " AND ar.id IN (SELECT artist_id FROM artist_fts WHERE artist_fts MATCH ?) \
              ORDER BY ar.name COLLATE NOCASE, ar.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(artists.limit)
        .bind(artists.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(SearchResult {
            artists,
            albums,
            songs,
        })
    }
}
