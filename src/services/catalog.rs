//! Music folders, catalogue overviews and artwork.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    /// The libraries one account can reach.
    ///
    /// `getMusicFolders` needs nothing else, and used to read the whole
    /// catalogue to answer with a handful of names.
    pub async fn music_folders(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
    ) -> Result<Vec<MusicFolderItem>, ServiceError> {
        let folder_filter = folder_filter(folder_ids);
        Ok(sqlx::query(
            "SELECT l.id, l.name FROM library l JOIN library_member m ON m.library_id=l.id \
             WHERE m.user_id=? AND (? IS NULL OR l.id IN (SELECT value FROM json_each(?))) \
             ORDER BY l.name COLLATE NOCASE",
        )
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(|row| {
            Ok(MusicFolderItem {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }

    /// Folders, artists and albums, without the tracks.
    ///
    /// Most of what browses the catalogue never looks at a track: an index of
    /// artists, an artist's albums, a folder's contents. Those used to read
    /// every visible track anyway, because one snapshot served every browse
    /// method, and the track read is by far the largest of the three — and
    /// since the OpenSubsonic fields landed it carries two relation loads of
    /// its own.
    pub async fn catalog_overview(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
    ) -> Result<CatalogOverview, ServiceError> {
        let folder_filter = folder_filter(folder_ids);
        let folders = self.music_folders(user_id, folder_ids).await?;
        let artists = sqlx::query(concat!(
            artist_select!(album_count),
            // Only artists an album is credited to. A composer with no album
            // of their own is reachable by identifier and by search, but does
            // not belong in an index of the library's artists — which is what
            // the reference answers, and what `getArtists` means.
            " AND EXISTS (SELECT 1 FROM artist_role_stats ars \
                  WHERE ars.artist_id=ar.id AND ars.role='albumartist') \
                AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY ar.name COLLATE NOCASE"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_summary_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY al.title COLLATE NOCASE"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        Ok(CatalogOverview {
            folders,
            artists,
            albums,
        })
    }

    /// The overview plus every visible track.
    ///
    /// **No route calls this.** Every browse method that used to now asks for
    /// what it renders, and nothing should reach for this again: it is the
    /// shape that made one album page read a tenant's whole catalogue.
    ///
    /// It survives as a fixture. The integration suite builds ids from it —
    /// "give me an album of this account so I can ask for it" — which is a
    /// legitimate use of a full read in a test with three tracks in it, and
    /// not one in a request.
    pub async fn catalog_snapshot(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
    ) -> Result<CatalogSnapshot, ServiceError> {
        let overview = self.catalog_overview(user_id, folder_ids).await?;
        let songs = fetch_songs(
            &self.db,
            user_id,
            folder_filter(folder_ids).as_deref(),
            None,
        )
        .await?;
        Ok(CatalogSnapshot {
            folders: overview.folders,
            artists: overview.artists,
            albums: overview.albums,
            songs,
        })
    }

    /// Backs Subsonic `search3` with the FTS5 index instead of materialising the
    /// whole catalogue and filtering it in memory.
    ///
    /// `track_fts` indexes title, album, artists and genres per track, so
    /// selecting matching tracks and deriving their albums and artists covers
    /// the same ground the in-memory pass did — a matching album title reaches
    /// its own tracks through the `album` column.
    ///
    /// Its tokenizer folds case *and* diacritics, so "echo" now finds "Écho",
    /// which the previous lowercase substring test did not. What it gives up is
    /// matching inside a word: "cho" no longer finds "Echo". The trailing term
    /// is treated as a prefix so search-as-you-type still works.
    /// Paged in SQL, one page per kind.
    ///
    /// The three pages are independent because `search3` has always let a
    /// client page songs past the end of the artists. Slicing a full result in
    /// the renderer, which is what this used to leave it to do, read the whole
    /// matching catalogue to answer for twenty rows of it.
    pub async fn catalog_search(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
        query: &str,
        artist_page: BrowsePage,
        album_page: BrowsePage,
        song_page: BrowsePage,
    ) -> Result<CatalogSearch, ServiceError> {
        let Some(fts) = crate::catalog::fts_prefix_query(query) else {
            return Ok(CatalogSearch {
                artists: Vec::new(),
                albums: Vec::new(),
                songs: Vec::new(),
            });
        };
        let folders = folder_filter(folder_ids);
        let folder_filter = folders.as_deref();

        let mut songs = sqlx::query(concat!(
            song_select!(),
            " AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
               AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
             ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .bind(song_page.limit)
        .bind(song_page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;

        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
               AND al.id IN (SELECT t.album_id FROM track t WHERE t.album_id IS NOT NULL \
                 AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?)) \
             ORDER BY al.title COLLATE NOCASE, al.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .bind(album_page.limit)
        .bind(album_page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;

        let artists = sqlx::query(concat!(
            artist_select!(),
            " AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
                AND ar.id IN (SELECT artist_id FROM artist_fts WHERE artist_fts MATCH ?) \
              ORDER BY ar.name COLLATE NOCASE, ar.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .bind(artist_page.limit)
        .bind(artist_page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;

        Ok(CatalogSearch {
            artists,
            albums,
            songs,
        })
    }

    pub async fn artwork_for_user(
        &self,
        user_id: Uuid,
        id: &str,
    ) -> Result<Option<(String, String)>, ServiceError> {
        let row = sqlx::query(
            "SELECT a.hash, a.format FROM artwork a WHERE a.hash=? AND EXISTS ( \
               SELECT 1 FROM track t JOIN library_member m ON m.library_id=t.library_id WHERE t.artwork_hash=a.hash AND m.user_id=? \
               UNION SELECT 1 FROM album al JOIN library_member m ON m.library_id=al.library_id WHERE al.artwork_hash=a.hash AND m.user_id=? \
               UNION SELECT 1 FROM artist ar JOIN library_member m ON m.library_id=ar.library_id WHERE ar.artwork_hash=a.hash AND m.user_id=? \
             ) UNION ALL SELECT a.hash, a.format FROM track t JOIN library_member m ON m.library_id=t.library_id JOIN artwork a ON a.hash=t.artwork_hash WHERE t.id=? AND m.user_id=? \
             UNION ALL SELECT a.hash, a.format FROM album al JOIN library_member m ON m.library_id=al.library_id JOIN artwork a ON a.hash=al.artwork_hash WHERE al.id=? AND m.user_id=? \
             UNION ALL SELECT a.hash, a.format FROM artist ar JOIN library_member m ON m.library_id=ar.library_id JOIN artwork a ON a.hash=ar.artwork_hash WHERE ar.id=? AND m.user_id=? LIMIT 1",
        )
        .bind(id).bind(user_id.to_string()).bind(user_id.to_string()).bind(user_id.to_string())
        .bind(id).bind(user_id.to_string()).bind(id).bind(user_id.to_string()).bind(id).bind(user_id.to_string())
        .fetch_optional(self.db.pool()).await?;
        row.map(|row| Ok::<_, sqlx::Error>((row.try_get("hash")?, row.try_get("format")?)))
            .transpose()
            .map_err(Into::into)
    }
}
