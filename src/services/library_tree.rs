//! The disk tree of a library: one level at a time, read-only (RFC-012
//! decision 1).
//!
//! The tree is derived from `track.relative_path`, which the scanner stores
//! with `/` on every platform. Nothing here lists a directory: a folder exists
//! because an available track's path runs through it, so a folder holding no
//! indexed audio does not appear, and what appears is what the last scan saw.
//! The library's root path is never read, let alone returned.

use serde::Serialize;
use sqlx::Row;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{
    attach_song_relations, song_from_row, BrowsePage, DomainServices, ServiceError, SongItem,
};
use crate::services::uploads::MANAGED_DIR;

/// The longest `path` a request may name. Far past any real tree, and short
/// enough that a path made of nothing but separators cannot weigh on a query.
const MAX_TREE_PATH_CHARS: usize = 4096;

/// A sub-folder in a listing.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DiskFolder {
    /// Its last segment.
    pub name: String,
    /// The address to ask for its own listing with: relative to the library,
    /// segments joined by `/`.
    pub path: String,
    /// The available tracks directly in it — the count RFC-012 gives a folder.
    pub track_count: i64,
    /// The available tracks anywhere beneath it, its own included: an artist's
    /// folder holds albums, not tracks, and would otherwise read as empty.
    pub nested_track_count: i64,
}

/// One level of the disk tree.
///
/// A page runs over the folders first, then the tracks, so `offset` and
/// `limit` page the two as one list. The totals say how long each part is.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DiskListing {
    /// The folder listed: empty for the library's root.
    pub path: String,
    pub folder_total: i64,
    pub track_total: i64,
    pub folders: Vec<DiskFolder>,
    pub tracks: Vec<DiskTrack>,
}

/// A track in a listing, under the name its file has.
///
/// The name rides beside the song rather than in it: `SongItem` keeps
/// `relative_path` to itself because every listing and the Subsonic façade
/// share it, and a path is shown where it is asked for (RFC-012, "Paths are
/// shown to members, and only to them").
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DiskTrack {
    /// The file name, extension included.
    pub name: String,
    pub song: SongItem,
}

/// A requested path, checked and split into the bounds the queries use.
struct TreePath {
    path: String,
    /// What every path beneath it starts with, `/` included; empty at the root.
    prefix: String,
    /// The first path past all of them: the folder's own path with `0`, the
    /// character after `/`, in place of the separator. Byte order is SQLite's
    /// `BINARY` collation, so `prefix <= p < upper` is exactly "starts with
    /// `prefix`", and it is answered from the `(library_id, relative_path)`
    /// index. `None` at the root, which bounds nothing.
    upper: Option<String>,
}

impl TreePath {
    /// `None` for anything that is not a plain relative path: a leading or
    /// trailing separator, an empty segment, `.` or `..`, a backslash. The
    /// path never reaches the file system, so this guards no traversal; it
    /// keeps one folder from having several spellings.
    fn parse(path: &str) -> Option<Self> {
        if path.is_empty() {
            return Some(Self {
                path: String::new(),
                prefix: String::new(),
                upper: None,
            });
        }
        if path.chars().count() > MAX_TREE_PATH_CHARS
            || path.contains(['\\', '\0'])
            || path
                .split('/')
                .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        {
            return None;
        }
        Some(Self {
            path: path.to_owned(),
            prefix: format!("{path}/"),
            upper: Some(format!("{path}0")),
        })
    }

    /// Where the part below the folder starts, as SQLite's `substr` counts:
    /// in characters, from one.
    fn rest_from(&self) -> i64 {
        self.prefix.chars().count() as i64 + 1
    }

    fn child(&self, name: &str) -> String {
        format!("{}{name}", self.prefix)
    }
}

impl DomainServices {
    /// One level of a library's disk tree: the sub-folders of `path`, with
    /// their counts, then the tracks directly in it.
    ///
    /// Any member reads it, listeners included (RFC-012 decision 3): every one
    /// of them already reads each track's `relative_path`, and this is only
    /// those paths, grouped. `NotFound` for a library the caller is not a
    /// member of, and for a folder no available track runs through; the root
    /// of an empty library is an empty listing. `Invalid` for a malformed path.
    ///
    /// Folders sort by name, case-insensitively. Tracks sort by file name,
    /// except in the received folder, whose file names are hashes: there they
    /// sort by artist then title, a missing artist last.
    pub async fn disk_tree(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        path: &str,
        page: BrowsePage,
    ) -> Result<DiskListing, ServiceError> {
        let tree = TreePath::parse(path).ok_or(ServiceError::Invalid)?;
        if self
            .db
            .library_for_user(user_id, library_id)
            .await?
            .is_none()
        {
            return Err(ServiceError::NotFound);
        }

        // One pass over the paths beneath the folder, grouped by their first
        // segment; the paths with no segment left over are the folder's own
        // tracks, and gather under a NULL name.
        let groups = sqlx::query(
            "WITH beneath AS ( \
               SELECT substr(t.relative_path, ?) AS rest FROM track t \
               JOIN library_member m ON m.library_id=t.library_id \
               WHERE m.user_id=? AND t.library_id=? AND t.is_available=1 \
                 AND t.relative_path >= ? AND (? IS NULL OR t.relative_path < ?)) \
             SELECT CASE WHEN instr(rest, '/') > 0 \
                      THEN substr(rest, 1, instr(rest, '/') - 1) END AS name, \
                    COUNT(*) AS nested, \
                    SUM(instr(rest, '/') > 0 \
                        AND instr(substr(rest, instr(rest, '/') + 1), '/') = 0) AS direct \
             FROM beneath GROUP BY name \
             ORDER BY name IS NOT NULL, name COLLATE NOCASE, name",
        )
        .bind(tree.rest_from())
        .bind(user_id.to_string())
        .bind(library_id.to_string())
        .bind(&tree.prefix)
        .bind(tree.upper.as_deref())
        .bind(tree.upper.as_deref())
        .fetch_all(self.db.pool())
        .await?;

        let mut track_total = 0;
        let mut folders = Vec::with_capacity(groups.len());
        for row in groups {
            let nested: i64 = row.try_get("nested")?;
            match row.try_get::<Option<String>, _>("name")? {
                None => track_total = nested,
                Some(name) => folders.push(DiskFolder {
                    path: tree.child(&name),
                    name,
                    track_count: row.try_get("direct")?,
                    nested_track_count: nested,
                }),
            }
        }
        if !tree.path.is_empty() && folders.is_empty() && track_total == 0 {
            return Err(ServiceError::NotFound);
        }

        let folder_total = folders.len() as i64;
        let (offset, limit) = (page.offset, page.limit);
        let folders: Vec<DiskFolder> = folders
            .into_iter()
            .skip(offset.min(folder_total) as usize)
            .take(limit as usize)
            .collect();
        let track_limit = limit - folders.len() as i64;
        let track_offset = (offset - folder_total).max(0);

        let mut tracks = Vec::new();
        if track_limit > 0 && track_offset < track_total {
            // The tracks whose path ends in the folder: beneath it, with no
            // separator left after its prefix.
            macro_rules! directly_in {
                () => {
                    " AND t.library_id=? AND t.relative_path >= ? \
                      AND (? IS NULL OR t.relative_path < ?) \
                      AND instr(substr(t.relative_path, ?), '/') = 0"
                };
            }
            let sql = if tree.path == MANAGED_DIR {
                concat!(
                    song_select!(),
                    directly_in!(),
                    " ORDER BY t.artist_display COLLATE NOCASE NULLS LAST, \
                       COALESCE(ovr.title, t.title) COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
                )
            } else {
                concat!(
                    song_select!(),
                    directly_in!(),
                    " ORDER BY t.relative_path COLLATE NOCASE, t.relative_path LIMIT ? OFFSET ?"
                )
            };
            tracks = sqlx::query(sql)
                .bind(user_id.to_string())
                .bind(library_id.to_string())
                .bind(&tree.prefix)
                .bind(tree.upper.as_deref())
                .bind(tree.upper.as_deref())
                .bind(tree.rest_from())
                .bind(track_limit)
                .bind(track_offset)
                .fetch_all(self.db.pool())
                .await?
                .into_iter()
                .map(song_from_row)
                .collect::<Result<Vec<_>, _>>()?;
            attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut tracks)
                .await?;
        }
        let tracks = tracks
            .into_iter()
            .map(|song| DiskTrack {
                name: song
                    .relative_path
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned(),
                song,
            })
            .collect();

        Ok(DiskListing {
            path: tree.path,
            folder_total,
            track_total,
            folders,
            tracks,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_path_has_one_spelling() {
        for refused in [
            "/Artist",
            "Artist/",
            "Artist//Album",
            "./Artist",
            "Artist/..",
            "A\\B",
            "A\0B",
        ] {
            assert!(TreePath::parse(refused).is_none(), "{refused:?}");
        }
        let root = TreePath::parse("").unwrap();
        assert_eq!((root.prefix.as_str(), root.upper), ("", None));
        let album = TreePath::parse("Björk/Post").unwrap();
        assert_eq!(album.prefix, "Björk/Post/");
        assert_eq!(album.upper.as_deref(), Some("Björk/Post0"));
        // Counted in characters, as `substr` counts: the ö is one.
        assert_eq!(album.rest_from(), 12);
        assert_eq!(album.child("Disc 1"), "Björk/Post/Disc 1");
    }

    #[test]
    fn the_upper_bound_admits_exactly_the_paths_beneath() {
        let tree = TreePath::parse("A").unwrap();
        let upper = tree.upper.unwrap();
        let beneath = |path: &str| path >= tree.prefix.as_str() && path < upper.as_str();
        assert!(beneath("A/song.flac"));
        assert!(beneath("A/B/song.flac"));
        assert!(!beneath("A.flac"));
        assert!(!beneath("A-side/song.flac"));
        assert!(!beneath("AB/song.flac"));
        assert!(!beneath("A0/song.flac"));
    }
}
