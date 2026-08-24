//! Album listings and detail.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

impl DomainServices {
    /// Albums visible to the user, ordered, filtered and paged entirely in SQL.
    ///
    /// See [`AlbumOrder`] for why the ten orderings live here rather than in the
    /// Subsonic facade. Every mode reads a single static literal — sqlx only
    /// accepts static SQL, so the composition stays injection-proof by
    /// construction and the user id is always the first bind.
    pub async fn list_albums(
        &self,
        user_id: Uuid,
        query: &AlbumListQuery,
    ) -> Result<Vec<AlbumItem>, ServiceError> {
        let folders = (!query.library_ids.is_empty()).then(|| {
            serde_json::to_string(&query.library_ids).expect("UUID list serialization cannot fail")
        });
        // `byGenre` without a genre is a malformed request, not an empty one:
        // answering with the whole catalogue would drop the filter in silence.
        // Matching is on the canonical form, so "Hip-Hop" and "hip hop" are the
        // same genre — the facade previously compared display strings with
        // `eq_ignore_ascii_case`, which they are not.
        let genre = match query.order {
            AlbumOrder::ByGenre => Some(waveflow_core::scanner::canonical_name(
                query.genre.as_deref().ok_or(ServiceError::Invalid)?,
            )),
            _ => None,
        };
        // An absent bound is unbounded, and a reversed range is how Subsonic
        // asks for descending years.
        let from = query.from_year.unwrap_or(i64::MIN);
        let to = query.to_year.unwrap_or(i64::MAX);
        let sql = match (query.order, from <= to) {
            (AlbumOrder::AlphabeticalByName, _) => concat!(
                album_scope!(),
                " ORDER BY title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::AlphabeticalByArtist, _) => concat!(
                album_scope!(),
                " ORDER BY COALESCE(album_artist_name, '') COLLATE NOCASE, \
                   title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::Newest, _) => concat!(
                album_scope!(),
                " ORDER BY created_at DESC, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::Highest, _) => concat!(
                album_scope!(),
                " WHERE user_rating > 0 \
                  ORDER BY user_rating DESC, title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::Frequent, _) => concat!(
                album_scope!(),
                " WHERE play_count > 0 \
                  ORDER BY play_count DESC, title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::Recent, _) => concat!(
                album_scope!(),
                " WHERE last_played_at IS NOT NULL \
                  ORDER BY last_played_at DESC, title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::Starred, _) => concat!(
                album_scope!(),
                " WHERE starred_at IS NOT NULL \
                  ORDER BY starred_at DESC, title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::Random, _) => {
                concat!(album_scope!(), " ORDER BY RANDOM() LIMIT ? OFFSET ?")
            }
            (AlbumOrder::ByYear, true) => concat!(
                album_scope!(),
                " WHERE year IS NOT NULL AND year BETWEEN ? AND ? \
                  ORDER BY year, title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::ByYear, false) => concat!(
                album_scope!(),
                " WHERE year IS NOT NULL AND year BETWEEN ? AND ? \
                  ORDER BY year DESC, title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
            (AlbumOrder::ByGenre, _) => concat!(
                album_scope!(),
                " WHERE EXISTS (SELECT 1 FROM track t2 \
                    JOIN track_genre tg ON tg.track_id=t2.id \
                    JOIN genre g ON g.id=tg.genre_id \
                    WHERE t2.album_id=a.id AND t2.is_available=1 AND g.canonical_name=?) \
                  ORDER BY title COLLATE NOCASE, id LIMIT ? OFFSET ?"
            ),
        };
        let mut statement = sqlx::query(sql)
            .bind(user_id.to_string())
            .bind(folders.as_deref())
            .bind(folders.as_deref());
        if let Some(genre) = genre {
            statement = statement.bind(genre);
        }
        if query.order == AlbumOrder::ByYear {
            statement = statement.bind(from.min(to)).bind(from.max(to));
        }
        let mut albums = statement
            .bind(query.page.limit)
            .bind(query.page.offset)
            .fetch_all(self.db.pool())
            .await?
            .into_iter()
            .map(album_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        Ok(albums)
    }

    /// One album with its tracks in sleeve order. Returns [`ServiceError::NotFound`]
    /// both when the album does not exist and when it belongs to a library the
    /// user cannot see, so the surface never leaks another tenant's catalogue.
    pub async fn album(&self, user_id: Uuid, album_id: Uuid) -> Result<AlbumDetail, ServiceError> {
        let mut album = vec![sqlx::query(concat!(album_select!(), " AND al.id=?"))
            .bind(user_id.to_string())
            .bind(album_id.to_string())
            .fetch_optional(self.db.pool())
            .await?
            .map(album_from_row)
            .transpose()?
            .ok_or(ServiceError::NotFound)?];
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut album).await?;
        let album = album.remove(0);
        let mut songs = sqlx::query(concat!(
            song_select!(),
            // SQLite orders NULL first, which would put an untagged track ahead
            // of track 1. Incomplete disc/track tags are common in real
            // libraries, so unnumbered tracks sort to the end instead.
            " AND t.album_id=? \
              ORDER BY t.disc_number NULLS LAST, t.track_number NULLS LAST, \
                       t.title COLLATE NOCASE, t.id"
        ))
        .bind(user_id.to_string())
        .bind(album_id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(AlbumDetail { album, songs })
    }
}
