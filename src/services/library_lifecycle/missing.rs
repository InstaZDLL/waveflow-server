//! Listing the tracks a library's scans could no longer find, and the ones
//! already purged.

use super::*;

/// One page of a library's missing tracks with what purging each would cost:
/// those not purged, oldest missing first, when `grace` is `None`; those
/// purged and still restorable, soonest deleted first, when it is the grace
/// window in milliseconds.
async fn list_missing_on(
    connection: &mut SqliteConnection,
    library_id: Uuid,
    page: BrowsePage,
    grace: Option<i64>,
) -> Result<UnavailableTracks, ServiceError> {
    let purged = grace.is_some();
    // A purge past its window is due for deletion, not restorable: it is
    // listed nowhere until the finishing pass takes it.
    let restorable_after = grace.map(|grace| now_ms().saturating_sub(grace));
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM track WHERE library_id = ? AND is_available = 0 \
           AND (purged_at IS NOT NULL) = ? AND (? IS NULL OR purged_at > ?)",
    )
    .bind(library_id.to_string())
    .bind(purged)
    .bind(restorable_after)
    .bind(restorable_after)
    .fetch_one(&mut *connection)
    .await?;
    let rows = sqlx::query(
        "SELECT t.id, t.relative_path, COALESCE(ovr.title, t.title) AS title, \
                t.artist_display, t.album_title, t.duration_ms, \
                COALESCE(t.unavailable_since, t.updated_at) AS since, t.purged_at, \
                (SELECT COUNT(*) FROM playlist_track pt WHERE pt.track_id = t.id) \
                  AS playlist_entries, \
                (SELECT COUNT(*) FROM user_star us \
                  WHERE us.entity_type = 'track' AND us.entity_id = t.id) AS favorites, \
                (SELECT COUNT(*) FROM play_event pe WHERE pe.track_id = t.id) AS plays \
         FROM track t LEFT JOIN track_override ovr ON ovr.track_id = t.id \
         WHERE t.library_id = ? AND t.is_available = 0 AND (t.purged_at IS NOT NULL) = ? \
           AND (? IS NULL OR t.purged_at > ?) \
         ORDER BY t.purged_at, since, t.relative_path, t.id LIMIT ? OFFSET ?",
    )
    .bind(library_id.to_string())
    .bind(purged)
    .bind(restorable_after)
    .bind(restorable_after)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&mut *connection)
    .await?;
    let tracks = rows
        .into_iter()
        .map(|row| {
            let purged_at: Option<i64> = row.try_get("purged_at")?;
            Ok(UnavailableTrack {
                id: parse_uuid(row.try_get("id")?)?,
                relative_path: row.try_get("relative_path")?,
                title: row.try_get("title")?,
                artist: row.try_get("artist_display")?,
                album: row.try_get("album_title")?,
                duration_ms: row.try_get("duration_ms")?,
                unavailable_since: row.try_get("since")?,
                playlist_entries: row.try_get("playlist_entries")?,
                favorites: row.try_get("favorites")?,
                plays: row.try_get("plays")?,
                purged_at,
                deleted_after: purged_at
                    .zip(grace)
                    .map(|(at, grace)| at.saturating_add(grace)),
            })
        })
        .collect::<Result<Vec<_>, ServiceError>>()?;
    Ok(UnavailableTracks { total, tracks })
}

impl DomainServices {
    /// One page of the tracks a library's scans can no longer find, oldest
    /// first, with what purging each would cost (RFC-021 decision 4).
    pub async fn unavailable_tracks(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        page: BrowsePage,
    ) -> Result<UnavailableTracks, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        if !standing_on(&mut connection, actor, library_id)
            .await?
            .curates()
        {
            return Err(ServiceError::Forbidden);
        }
        list_missing_on(&mut connection, library_id, page, None).await
    }

    /// One page of a library's purged tracks still within their grace window,
    /// soonest deleted first, for the people who may purge them and so may
    /// take the purge back.
    pub async fn purged_tracks(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        page: BrowsePage,
    ) -> Result<UnavailableTracks, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        if !standing_on(&mut connection, actor, library_id)
            .await?
            .curates()
        {
            return Err(ServiceError::Forbidden);
        }
        let grace = i64::try_from(self.purge_grace.as_millis()).unwrap_or(i64::MAX);
        list_missing_on(&mut connection, library_id, page, Some(grace)).await
    }
}
