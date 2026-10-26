//! Playlists and their track lists.
//!
//! Split out of `services.rs`; see [`super`] for the shared projections.

use super::*;

/// The columns a playlist read answers, with its owner's name joined in.
///
/// Every read names the owner because a reader may be shown playlists that are
/// not its own (RFC-020 decision 2), and `owner` is what a Subsonic client
/// reads to decide whether it may edit one.
macro_rules! playlist_columns {
    () => {
        "SELECT p.id, p.owner_user_id, a.username AS owner_username, p.name, p.comment, \
                p.public, p.created_at, p.updated_at \
         FROM playlist p JOIN account a ON a.id=p.owner_user_id "
    };
}

/// Which playlists a read reaches.
///
/// **Visible** is the reader's own and every other account's public one, the
/// owner's account still enabled — a disabled account publishes nothing. Every
/// write reaches **owned** only, so a public playlist stays read-only to
/// anyone but its owner, and the refusal is the same not-found a missing one
/// answers. The predicate is written once per reach, in SQL, rather than
/// checked after the read: tenancy lives in the queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reach {
    Owned,
    Visible,
}

mod edit;
mod files;
mod reads;

/// A playlist read through `playlist_columns!`, with the songs already
/// resolved for whoever is reading it.
pub(super) fn playlist_from_row(
    row: &sqlx::sqlite::SqliteRow,
    id: Uuid,
    songs: Vec<SongItem>,
) -> Result<PlaylistItem, ServiceError> {
    Ok(PlaylistItem {
        id,
        owner_id: parse_uuid(row.try_get("owner_user_id")?)?,
        owner_username: row.try_get("owner_username")?,
        name: row.try_get("name")?,
        comment: row.try_get("comment")?,
        public: row.try_get::<i64, _>("public")? != 0,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        songs,
    })
}

/// Every track a playlist holds, hidden ones included, in its order.
///
/// Behind the same reach as a read, the wider of the two: a writer has already
/// proved ownership by the time it asks, and asking here again costs a join.
pub(super) async fn playlist_track_ids_on(
    connection: &mut SqliteConnection,
    user_id: Uuid,
    playlist_id: Uuid,
) -> Result<Vec<Uuid>, ServiceError> {
    sqlx::query_scalar::<_, String>(
        "SELECT pt.track_id FROM playlist_track pt \
         JOIN playlist p ON p.id=pt.playlist_id \
         JOIN account a ON a.id=p.owner_user_id \
         WHERE (p.owner_user_id=? OR (p.public=1 AND a.disabled=0)) AND p.id=? \
         ORDER BY pt.position",
    )
    .bind(user_id.to_string())
    .bind(playlist_id.to_string())
    .fetch_all(&mut *connection)
    .await?
    .into_iter()
    .map(parse_uuid)
    .collect::<Result<Vec<_>, _>>()
    .map_err(Into::into)
}
