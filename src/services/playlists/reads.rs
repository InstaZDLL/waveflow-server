//! Reading playlists: an account's own, the ones it may see, and one by id.

use super::*;

impl DomainServices {
    /// This account's own playlists. What the native list and the sync
    /// snapshot answer: both are about what the account holds and may edit.
    pub async fn playlists(&self, user_id: Uuid) -> Result<Vec<PlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlists_on(&mut connection, user_id).await
    }

    /// This account's playlists, then every other account's public ones.
    ///
    /// What Subsonic's `getPlaylists` means — "all playlists a user is allowed
    /// to play" — and what the native list answers when asked for it. The
    /// tracks of another account's playlist are read through **this**
    /// account's libraries: one it cannot reach is not shown and not counted.
    pub async fn visible_playlists(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<PlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.read_playlists_on(&mut connection, user_id, Reach::Visible)
            .await
    }

    pub(in crate::services) async fn playlists_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<PlaylistItem>, ServiceError> {
        self.read_playlists_on(connection, user_id, Reach::Owned)
            .await
    }

    async fn read_playlists_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        reach: Reach,
    ) -> Result<Vec<PlaylistItem>, ServiceError> {
        let rows = match reach {
            Reach::Owned => {
                sqlx::query(concat!(
                    playlist_columns!(),
                    "WHERE p.owner_user_id=? ORDER BY p.updated_at DESC, p.id"
                ))
                .bind(user_id.to_string())
                .fetch_all(&mut *connection)
                .await?
            }
            // The reader's own first, as before, then the others newest first.
            Reach::Visible => {
                sqlx::query(concat!(
                    playlist_columns!(),
                    "WHERE p.owner_user_id=? OR (p.public=1 AND a.disabled=0) \
                     ORDER BY p.owner_user_id<>?, p.updated_at DESC, p.id"
                ))
                .bind(user_id.to_string())
                .bind(user_id.to_string())
                .fetch_all(&mut *connection)
                .await?
            }
        };
        // One query for every playlist's track list and one for the songs they
        // name, rather than two per playlist: an account with fifty playlists
        // was costing a hundred round trips to answer `getPlaylists`.
        let playlist_ids = rows
            .iter()
            .map(|row| parse_uuid(row.try_get("id")?))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ordered: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        if !playlist_ids.is_empty() {
            let ids_json =
                serde_json::to_string(&playlist_ids).map_err(|_| ServiceError::Invalid)?;
            // The visibility predicate again, rather than trusting the ids:
            // the list above is the reach, and this read must not be wider.
            for row in sqlx::query(
                "SELECT pt.playlist_id, pt.track_id FROM playlist_track pt \
                 JOIN playlist p ON p.id=pt.playlist_id \
                 JOIN account a ON a.id=p.owner_user_id \
                 WHERE (p.owner_user_id=? OR (p.public=1 AND a.disabled=0)) \
                   AND p.id IN (SELECT value FROM json_each(?)) \
                 ORDER BY pt.playlist_id, pt.position",
            )
            .bind(user_id.to_string())
            .bind(ids_json)
            .fetch_all(&mut *connection)
            .await?
            {
                ordered
                    .entry(parse_uuid(row.try_get("playlist_id")?)?)
                    .or_default()
                    .push(parse_uuid(row.try_get("track_id")?)?);
            }
        }
        let mut union: Vec<Uuid> = ordered.values().flatten().copied().collect();
        union.sort_unstable();
        union.dedup();
        // Resolved once for every playlist at once. The per-playlist order and
        // the dropping of a track this account cannot see are reapplied below,
        // exactly as `songs_by_ids_lenient_on` would have applied them.
        let visible = self
            .songs_by_ids_lenient_on(connection, user_id, &union)
            .await?
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        let mut result = Vec::with_capacity(rows.len());
        for (row, id) in rows.into_iter().zip(playlist_ids) {
            let songs = ordered
                .get(&id)
                .into_iter()
                .flatten()
                .filter_map(|track| visible.get(track).cloned())
                .collect();
            result.push(playlist_from_row(&row, id, songs)?);
        }
        Ok(result)
    }

    /// One playlist this account may read: its own, or another account's
    /// public one, its tracks read through this account's libraries.
    pub async fn playlist(&self, user_id: Uuid, id: Uuid) -> Result<PlaylistItem, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlist_on(&mut connection, user_id, id, Reach::Visible)
            .await
    }

    pub(in crate::services) async fn playlist_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        id: Uuid,
        reach: Reach,
    ) -> Result<PlaylistItem, ServiceError> {
        let row = match reach {
            Reach::Owned => {
                sqlx::query(concat!(
                    playlist_columns!(),
                    "WHERE p.owner_user_id=? AND p.id=?"
                ))
                .bind(user_id.to_string())
                .bind(id.to_string())
                .fetch_optional(&mut *connection)
                .await?
            }
            Reach::Visible => {
                sqlx::query(concat!(
                    playlist_columns!(),
                    "WHERE (p.owner_user_id=? OR (p.public=1 AND a.disabled=0)) AND p.id=?"
                ))
                .bind(user_id.to_string())
                .bind(id.to_string())
                .fetch_optional(&mut *connection)
                .await?
            }
        }
        .ok_or(ServiceError::NotFound)?;
        let ids = playlist_track_ids_on(connection, user_id, id).await?;
        let songs = self
            .songs_by_ids_lenient_on(connection, user_id, &ids)
            .await?;
        playlist_from_row(&row, id, songs)
    }
}
