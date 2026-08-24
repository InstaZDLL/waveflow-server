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
        let summary = sqlx::query(concat!(artist_select!(album_count), " AND ar.id=?"))
            .bind(user_id.to_string())
            .bind(artist_id.to_string())
            .fetch_optional(self.db.pool())
            .await?
            .map(artist_summary_from_row)
            .transpose()?
            .ok_or(ServiceError::NotFound)?;
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
        Ok(ArtistDetail {
            artist: summary.artist,
            album_count: summary.album_count,
            albums,
        })
    }
}
