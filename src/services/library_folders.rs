//! Filed folders: a library sorted by its owner and managers, in the database
//! only (RFC-012 decisions 2 and 3).
//!
//! A folder is a place, not a collection: a track sits in at most one, and
//! moving it takes it out of the first. Nothing here touches a file. Every
//! write is gated like a metadata correction — owner and manager — and runs in
//! one transaction under the writer gate, so the caps and the checks it makes
//! hold against a racing write. Any member reads.

use std::collections::BTreeSet;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sqlx::{Row, Sqlite, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    attach_song_relations, song_from_row, BrowsePage, DomainServices, ServiceError, SongItem,
};
use crate::authentication::now_ms;
use crate::database::LibraryRole;

/// The deepest a folder may sit: the root is 0, a folder at the root is 1.
pub const MAX_FOLDER_DEPTH: i64 = 16;
/// The most folders one library holds — what keeps the tree one bounded
/// answer.
pub const MAX_FOLDERS_PER_LIBRARY: i64 = 5_000;
/// The most tracks one placement request files.
pub const MAX_PLACEMENTS: usize = 500;
const MAX_NAME_CHARS: usize = 120;
/// Past any depth the caps allow; it only keeps a walk finite.
const WALK_LIMIT: i64 = 64;

/// A filed folder, without its tracks.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FiledFolder {
    pub id: Uuid,
    /// `null` at the root.
    pub parent_id: Option<Uuid>,
    pub name: String,
    /// The available tracks filed directly in it.
    pub track_count: i64,
}

/// A library's whole filed tree, as one flat list a client draws from.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FiledTree {
    /// Every folder, siblings by name in natural order: numbers by value,
    /// letters case-folded.
    pub folders: Vec<FiledFolder>,
    /// The available tracks filed nowhere: the root's **Unfiled** entry.
    pub unfiled_track_count: i64,
}

/// What deleting a folder did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FolderRemoved {
    /// The tracks that were filed in it or under it, and are now unfiled.
    pub unfiled: i64,
}

/// What a placement did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Placed {
    /// The tracks filed, or unfiled when the folder was `null`.
    pub placed: i64,
}

/// A rename, a move, or both. `parent_id` absent leaves the folder where it
/// is; `null` moves it to the root.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct FolderPatch {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "super::present")]
    #[schema(value_type = Option<Uuid>, nullable)]
    pub parent_id: Option<Option<Uuid>>,
}

/// A name as stored, and the key siblings are compared by. `None` when it is
/// not a name: empty once trimmed, longer than 120 characters, or holding a
/// control character. A `/` is allowed — no path is built from it.
fn folder_name(name: &str) -> Option<(String, String)> {
    let name = name.trim();
    let length = name.chars().count();
    if length == 0 || length > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return None;
    }
    Some((name.to_owned(), name.to_lowercase()))
}

/// The library a folder belongs to and the caller's role in it; `NotFound`
/// for a folder in no library the caller is a member of.
async fn folder_access(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: Uuid,
    folder_id: Uuid,
) -> Result<(String, LibraryRole), ServiceError> {
    let row = sqlx::query(
        "SELECT f.library_id, m.role FROM library_folder f \
         JOIN library_member m ON m.library_id=f.library_id \
         WHERE m.user_id=? AND f.id=?",
    )
    .bind(user_id.to_string())
    .bind(folder_id.to_string())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ServiceError::NotFound)?;
    let role = LibraryRole::from_str(row.try_get::<&str, _>("role")?)
        .map_err(|_| ServiceError::Invalid)?;
    Ok((row.try_get("library_id")?, role))
}

/// The caller's role in a library; `NotFound` for one they are not in.
async fn library_role(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: Uuid,
    library_id: Uuid,
) -> Result<LibraryRole, ServiceError> {
    let role: String =
        sqlx::query_scalar("SELECT role FROM library_member WHERE user_id=? AND library_id=?")
            .bind(user_id.to_string())
            .bind(library_id.to_string())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(ServiceError::NotFound)?;
    LibraryRole::from_str(&role).map_err(|_| ServiceError::Invalid)
}

fn may_file(role: LibraryRole) -> Result<(), ServiceError> {
    if role.may_write_metadata() {
        Ok(())
    } else {
        Err(ServiceError::Forbidden)
    }
}

/// How deep a folder sits: 1 at the root.
async fn depth_of(tx: &mut Transaction<'_, Sqlite>, folder_id: &str) -> Result<i64, ServiceError> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE up(id, parent_id, depth) AS ( \
           SELECT id, parent_id, 1 FROM library_folder WHERE id=? \
           UNION ALL \
           SELECT f.id, f.parent_id, up.depth + 1 FROM library_folder f \
           JOIN up ON f.id=up.parent_id WHERE up.depth < ?) \
         SELECT COALESCE(MAX(depth), 0) FROM up",
    )
    .bind(folder_id)
    .bind(WALK_LIMIT)
    .fetch_one(&mut **tx)
    .await?)
}

/// A folder's sub-tree: its height, the folder itself counting 1, and whether
/// `candidate` is in it.
async fn subtree(
    tx: &mut Transaction<'_, Sqlite>,
    folder_id: &str,
    candidate: Option<&str>,
) -> Result<(i64, bool), ServiceError> {
    let row = sqlx::query(
        "WITH RECURSIVE down(id, level) AS ( \
           SELECT id, 1 FROM library_folder WHERE id=? \
           UNION ALL \
           SELECT f.id, down.level + 1 FROM library_folder f \
           JOIN down ON f.parent_id=down.id WHERE down.level < ?) \
         SELECT COALESCE(MAX(level), 0) AS height, \
                COALESCE(SUM(id = ?), 0) AS holds FROM down",
    )
    .bind(folder_id)
    .bind(WALK_LIMIT)
    .bind(candidate)
    .fetch_one(&mut **tx)
    .await?;
    Ok((row.try_get("height")?, row.try_get::<i64, _>("holds")? > 0))
}

/// `Conflict` when a sibling under `parent` already answers to `key`.
async fn sibling_free(
    tx: &mut Transaction<'_, Sqlite>,
    library_id: &str,
    parent: Option<&str>,
    key: &str,
    except: Option<&str>,
) -> Result<(), ServiceError> {
    let taken: Option<String> = sqlx::query_scalar(
        "SELECT id FROM library_folder \
         WHERE library_id=? AND ifnull(parent_id, '')=ifnull(?, '') AND name_key=? \
           AND (? IS NULL OR id<>?)",
    )
    .bind(library_id)
    .bind(parent)
    .bind(key)
    .bind(except)
    .bind(except)
    .fetch_optional(&mut **tx)
    .await?;
    if taken.is_some() {
        Err(ServiceError::Conflict)
    } else {
        Ok(())
    }
}

/// `NotFound` unless `folder_id` is a folder of `library_id`.
async fn folder_in_library(
    tx: &mut Transaction<'_, Sqlite>,
    library_id: &str,
    folder_id: &str,
) -> Result<(), ServiceError> {
    sqlx::query_scalar::<_, String>("SELECT id FROM library_folder WHERE id=? AND library_id=?")
        .bind(folder_id)
        .bind(library_id)
        .fetch_optional(&mut **tx)
        .await?
        .map(|_| ())
        .ok_or(ServiceError::NotFound)
}

async fn folder_row(
    tx: &mut Transaction<'_, Sqlite>,
    folder_id: &str,
) -> Result<FiledFolder, ServiceError> {
    let row = sqlx::query(
        "SELECT f.id, f.parent_id, f.name, \
           (SELECT COUNT(*) FROM track_placement p JOIN track t ON t.id=p.track_id \
            WHERE p.folder_id=f.id AND t.is_available=1) AS track_count \
         FROM library_folder f WHERE f.id=?",
    )
    .bind(folder_id)
    .fetch_one(&mut **tx)
    .await?;
    filed_folder(&row)
}

fn filed_folder(row: &sqlx::sqlite::SqliteRow) -> Result<FiledFolder, ServiceError> {
    let id: String = row.try_get("id")?;
    let parent: Option<String> = row.try_get("parent_id")?;
    Ok(FiledFolder {
        id: Uuid::parse_str(&id).map_err(|_| ServiceError::Invalid)?,
        parent_id: parent
            .map(|parent| Uuid::parse_str(&parent))
            .transpose()
            .map_err(|_| ServiceError::Invalid)?,
        name: row.try_get("name")?,
        track_count: row.try_get("track_count")?,
    })
}

impl DomainServices {
    /// A library's whole filed tree, flat: bounded by the folder cap, so it
    /// is one answer and never a page. Any member reads it.
    pub async fn filed_tree(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<FiledTree, ServiceError> {
        if self
            .db
            .library_for_user(user_id, library_id)
            .await?
            .is_none()
        {
            return Err(ServiceError::NotFound);
        }
        let rows = sqlx::query(
            "SELECT f.id, f.parent_id, f.name, \
               (SELECT COUNT(*) FROM track_placement p JOIN track t ON t.id=p.track_id \
                WHERE p.folder_id=f.id AND t.is_available=1) AS track_count \
             FROM library_folder f WHERE f.library_id=? \
             ORDER BY f.name COLLATE natural_order, f.id",
        )
        .bind(library_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let folders = rows
            .iter()
            .map(filed_folder)
            .collect::<Result<Vec<_>, _>>()?;
        let unfiled_track_count = sqlx::query_scalar(
            "SELECT COUNT(*) FROM track t WHERE t.library_id=? AND t.is_available=1 \
             AND NOT EXISTS (SELECT 1 FROM track_placement p WHERE p.track_id=t.id)",
        )
        .bind(library_id.to_string())
        .fetch_one(self.db.pool())
        .await?;
        Ok(FiledTree {
            folders,
            unfiled_track_count,
        })
    }

    /// Create a folder at the root, or under `parent_id` in the same library.
    ///
    /// `Invalid` for a name that is not one, past the depth cap, or past the
    /// library's folder cap; `Conflict` for a sibling of the same name;
    /// `NotFound` for a parent elsewhere. All checked in the transaction that
    /// writes, under the writer gate.
    pub async fn create_folder(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        name: &str,
        parent_id: Option<Uuid>,
    ) -> Result<FiledFolder, ServiceError> {
        let (name, key) = folder_name(name).ok_or(ServiceError::Invalid)?;
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        may_file(library_role(&mut tx, user_id, library_id).await?)?;
        let library = library_id.to_string();
        let parent = parent_id.map(|parent| parent.to_string());
        if let Some(parent) = parent.as_deref() {
            folder_in_library(&mut tx, &library, parent).await?;
            if depth_of(&mut tx, parent).await? >= MAX_FOLDER_DEPTH {
                return Err(ServiceError::Invalid);
            }
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM library_folder WHERE library_id=?")
                .bind(&library)
                .fetch_one(&mut *tx)
                .await?;
        if count >= MAX_FOLDERS_PER_LIBRARY {
            return Err(ServiceError::Invalid);
        }
        sibling_free(&mut tx, &library, parent.as_deref(), &key, None).await?;
        let id = Uuid::new_v4().to_string();
        let now = now_ms();
        sqlx::query(
            "INSERT INTO library_folder \
               (id, library_id, parent_id, name, name_key, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&library)
        .bind(parent.as_deref())
        .bind(&name)
        .bind(&key)
        .bind(user_id.to_string())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let folder = folder_row(&mut tx, &id).await?;
        tx.commit().await?;
        Ok(folder)
    }

    /// Rename a folder, move it, or both.
    ///
    /// A move is checked for the whole sub-tree it carries: the new parent's
    /// depth plus the sub-tree's height must stay within the cap, and a move
    /// under itself or one of its descendants is refused. Both answer
    /// `Invalid`, and neither is repaired.
    pub async fn update_folder(
        &self,
        user_id: Uuid,
        folder_id: Uuid,
        patch: FolderPatch,
    ) -> Result<FiledFolder, ServiceError> {
        let renamed = match patch.name.as_deref() {
            Some(name) => Some(folder_name(name).ok_or(ServiceError::Invalid)?),
            None => None,
        };
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let (library, role) = folder_access(&mut tx, user_id, folder_id).await?;
        may_file(role)?;
        let id = folder_id.to_string();
        let current = sqlx::query("SELECT parent_id, name_key FROM library_folder WHERE id=?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
        let current_parent: Option<String> = current.try_get("parent_id")?;
        let current_key: String = current.try_get("name_key")?;

        let parent = match patch.parent_id {
            None => current_parent,
            Some(target) => {
                let target = target.map(|target| target.to_string());
                let (height, holds) = subtree(&mut tx, &id, target.as_deref()).await?;
                let base = match target.as_deref() {
                    Some(target) => {
                        folder_in_library(&mut tx, &library, target).await?;
                        if holds {
                            return Err(ServiceError::Invalid);
                        }
                        depth_of(&mut tx, target).await?
                    }
                    None => 0,
                };
                if base + height > MAX_FOLDER_DEPTH {
                    return Err(ServiceError::Invalid);
                }
                target
            }
        };
        let key = renamed
            .as_ref()
            .map_or(current_key.as_str(), |(_, key)| key.as_str());
        sibling_free(&mut tx, &library, parent.as_deref(), key, Some(&id)).await?;
        sqlx::query(
            "UPDATE library_folder SET parent_id=?, \
               name=COALESCE(?, name), name_key=COALESCE(?, name_key), updated_at=? \
             WHERE id=?",
        )
        .bind(parent.as_deref())
        .bind(renamed.as_ref().map(|(name, _)| name.as_str()))
        .bind(renamed.as_ref().map(|(_, key)| key.as_str()))
        .bind(now_ms())
        .bind(&id)
        .execute(&mut *tx)
        .await?;
        let folder = folder_row(&mut tx, &id).await?;
        tx.commit().await?;
        Ok(folder)
    }

    /// Delete a folder and its sub-folders. Every track filed in or under it
    /// becomes unfiled; nothing is filed elsewhere and nothing on disk moves.
    pub async fn delete_folder(
        &self,
        user_id: Uuid,
        folder_id: Uuid,
    ) -> Result<FolderRemoved, ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let (_, role) = folder_access(&mut tx, user_id, folder_id).await?;
        may_file(role)?;
        let id = folder_id.to_string();
        let unfiled: i64 = sqlx::query_scalar(
            "WITH RECURSIVE down(id, level) AS ( \
               SELECT id, 1 FROM library_folder WHERE id=? \
               UNION ALL \
               SELECT f.id, down.level + 1 FROM library_folder f \
               JOIN down ON f.parent_id=down.id WHERE down.level < ?) \
             SELECT COUNT(*) FROM track_placement WHERE folder_id IN (SELECT id FROM down)",
        )
        .bind(&id)
        .bind(WALK_LIMIT)
        .fetch_one(&mut *tx)
        .await?;
        // The sub-folders and the placements go by their foreign keys.
        sqlx::query("DELETE FROM library_folder WHERE id=?")
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(FolderRemoved { unfiled })
    }

    /// The tracks filed directly in a folder, or with `folder_id` `None` the
    /// library's unfiled ones. Sorted by artist, album, disc and track, a
    /// missing tag last. Any member reads; `NotFound` for anyone else.
    pub async fn folder_tracks(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        folder_id: Option<Uuid>,
        page: BrowsePage,
    ) -> Result<Vec<SongItem>, ServiceError> {
        macro_rules! by_sleeve {
            () => {
                " ORDER BY t.artist_display COLLATE NOCASE NULLS LAST, \
                   t.album_title COLLATE NOCASE NULLS LAST, \
                   COALESCE(ovr.disc_number, t.disc_number) NULLS LAST, \
                   COALESCE(ovr.track_number, t.track_number) NULLS LAST, \
                   COALESCE(ovr.title, t.title) COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
            };
        }
        let rows = match folder_id {
            Some(folder) => {
                let library: Option<String> = sqlx::query_scalar(
                    "SELECT f.library_id FROM library_folder f \
                     JOIN library_member m ON m.library_id=f.library_id \
                     WHERE m.user_id=? AND f.id=? AND f.library_id=?",
                )
                .bind(user_id.to_string())
                .bind(folder.to_string())
                .bind(library_id.to_string())
                .fetch_optional(self.db.pool())
                .await?;
                if library.is_none() {
                    return Err(ServiceError::NotFound);
                }
                sqlx::query(concat!(
                    song_select!(),
                    " AND t.id IN (SELECT track_id FROM track_placement WHERE folder_id=?)",
                    by_sleeve!()
                ))
                .bind(user_id.to_string())
                .bind(folder.to_string())
                .bind(page.limit)
                .bind(page.offset)
                .fetch_all(self.db.pool())
                .await?
            }
            None => {
                if self
                    .db
                    .library_for_user(user_id, library_id)
                    .await?
                    .is_none()
                {
                    return Err(ServiceError::NotFound);
                }
                sqlx::query(concat!(
                    song_select!(),
                    " AND t.library_id=? \
                      AND NOT EXISTS (SELECT 1 FROM track_placement p WHERE p.track_id=t.id)",
                    by_sleeve!()
                ))
                .bind(user_id.to_string())
                .bind(library_id.to_string())
                .bind(page.limit)
                .bind(page.offset)
                .fetch_all(self.db.pool())
                .await?
            }
        };
        let mut songs = rows
            .into_iter()
            .map(song_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(songs)
    }

    /// File up to 500 tracks into one folder, or unfile them with `None`.
    ///
    /// All or nothing: the folder and every track are checked before anything
    /// is written, in one transaction under the writer gate. A track that is
    /// not an available track of this library, or a folder of another, answers
    /// `NotFound`; more than 500 answers `Invalid`. Filing a track that sits
    /// in another folder moves it.
    pub async fn place_tracks(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        track_ids: &[Uuid],
        folder_id: Option<Uuid>,
    ) -> Result<Placed, ServiceError> {
        if track_ids.len() > MAX_PLACEMENTS {
            return Err(ServiceError::Invalid);
        }
        let tracks: BTreeSet<String> = track_ids.iter().map(Uuid::to_string).collect();
        let listed = serde_json::to_string(&tracks).map_err(|_| ServiceError::Invalid)?;
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        may_file(library_role(&mut tx, user_id, library_id).await?)?;
        let library = library_id.to_string();
        if let Some(folder) = folder_id {
            folder_in_library(&mut tx, &library, &folder.to_string()).await?;
        }
        let found: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM track WHERE library_id=? AND is_available=1 \
             AND id IN (SELECT value FROM json_each(?))",
        )
        .bind(&library)
        .bind(&listed)
        .fetch_one(&mut *tx)
        .await?;
        if found != tracks.len() as i64 {
            return Err(ServiceError::NotFound);
        }
        match folder_id {
            Some(folder) => {
                sqlx::query(
                    "INSERT INTO track_placement \
                       (track_id, library_id, folder_id, placed_by, placed_at) \
                     SELECT value, ?, ?, ?, ? FROM json_each(?) WHERE true \
                     ON CONFLICT(track_id) DO UPDATE SET \
                       folder_id=excluded.folder_id, placed_by=excluded.placed_by, \
                       placed_at=excluded.placed_at",
                )
                .bind(&library)
                .bind(folder.to_string())
                .bind(user_id.to_string())
                .bind(now_ms())
                .bind(&listed)
                .execute(&mut *tx)
                .await?;
            }
            None => {
                sqlx::query(
                    "DELETE FROM track_placement \
                     WHERE library_id=? AND track_id IN (SELECT value FROM json_each(?))",
                )
                .bind(&library)
                .bind(&listed)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(Placed {
            placed: tracks.len() as i64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_name_is_trimmed_bounded_and_folded() {
        assert_eq!(
            folder_name("  Été / 2024 "),
            Some(("Été / 2024".into(), "été / 2024".into()))
        );
        assert_eq!(
            folder_name(&"é".repeat(120)).map(|(name, _)| name.chars().count()),
            Some(120)
        );
        for refused in ["", "   ", "tab\there", "line\nbreak"] {
            assert!(folder_name(refused).is_none(), "{refused:?}");
        }
        assert!(folder_name(&"é".repeat(121)).is_none());
    }
}
