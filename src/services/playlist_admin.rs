//! An administrator's view of another account's playlists: what Subsonic's
//! `getPlaylists?username=` asks for, and RFC-020 left ignored.
//!
//! **Read-only, and read through the administrator's own libraries.** Seeing
//! that a playlist exists, its name and its owner is what the parameter is
//! for; the tracks in it are still resolved through the reader's
//! `library_member` rows by the projection every read uses, so a track in a
//! library the administrator is not a member of is neither shown nor counted.
//! Every write stays the owner's.

use super::playlists::playlist_from_row;
use super::*;

/// The columns `playlist_from_row` reads, as the playlist reads select them.
macro_rules! administered_columns {
    () => {
        "SELECT p.id, p.owner_user_id, a.username AS owner_username, p.name, p.comment, \
                p.public, p.created_at, p.updated_at \
         FROM playlist p JOIN account a ON a.id=p.owner_user_id \
         WHERE EXISTS (SELECT 1 FROM account admin \
                       WHERE admin.id=? AND admin.role='admin' AND admin.disabled=0) "
    };
}

impl DomainServices {
    /// Every playlist `username` owns, private ones included, for an active
    /// administrator. Anyone else, and a name no account carries, is answered
    /// not-found alike: the answer must not say which accounts exist to
    /// someone not entitled to list them.
    pub async fn administered_playlists(
        &self,
        admin_id: Uuid,
        username: &str,
    ) -> Result<Vec<PlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        let owner: Option<String> = sqlx::query_scalar(
            "SELECT target.id FROM account target \
             WHERE target.username=? COLLATE NOCASE \
               AND EXISTS (SELECT 1 FROM account admin \
                           WHERE admin.id=? AND admin.role='admin' AND admin.disabled=0)",
        )
        .bind(username.trim())
        .bind(admin_id.to_string())
        .fetch_optional(&mut *connection)
        .await?;
        let owner = owner.ok_or(ServiceError::NotFound)?;
        let rows = sqlx::query(concat!(
            administered_columns!(),
            "AND p.owner_user_id=? ORDER BY p.updated_at DESC, p.id"
        ))
        .bind(admin_id.to_string())
        .bind(&owner)
        .fetch_all(&mut *connection)
        .await?;
        let mut playlists = Vec::with_capacity(rows.len());
        for row in rows {
            let id = parse_uuid(row.try_get("id")?)?;
            let songs = self
                .administered_songs_on(&mut connection, admin_id, id)
                .await?;
            playlists.push(playlist_from_row(&row, id, songs)?);
        }
        Ok(playlists)
    }

    /// One playlist of any account, for an active administrator: the playlist
    /// `administered_playlists` listed must open. Not-found to anyone else.
    pub async fn administered_playlist(
        &self,
        admin_id: Uuid,
        id: Uuid,
    ) -> Result<PlaylistItem, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        let row = sqlx::query(concat!(administered_columns!(), "AND p.id=?"))
            .bind(admin_id.to_string())
            .bind(id.to_string())
            .fetch_optional(&mut *connection)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let songs = self
            .administered_songs_on(&mut connection, admin_id, id)
            .await?;
        playlist_from_row(&row, id, songs)
    }

    /// A playlist's tracks in order, as the administrator may read them.
    async fn administered_songs_on(
        &self,
        connection: &mut SqliteConnection,
        admin_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<SongItem>, ServiceError> {
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT track_id FROM playlist_track WHERE playlist_id=? ORDER BY position",
        )
        .bind(playlist_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
        self.songs_by_ids_lenient_on(connection, admin_id, &ids)
            .await
    }
}
