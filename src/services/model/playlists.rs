//! Playlists, what an import made of a playlist file, and what an update may
//! clear.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::catalog::SongItem;
use crate::library::playlist_file::TextEncoding;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PlaylistItem {
    pub id: Uuid,
    /// Whose playlist it is. A reader may be shown another account's public
    /// playlist beside its own (RFC-020 decision 2), and only the owner may
    /// change one.
    pub owner_id: Uuid,
    pub owner_username: String,
    pub name: String,
    pub comment: Option<String>,
    pub public: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub songs: Vec<SongItem>,
}

/// What an import made of a playlist file, and what it could not.
///
/// The unresolved list is the point of this type. An import that silently
/// produced a shorter playlist would be worse than one that refused: the
/// listener has no way to know a third of a collection stayed behind. So every
/// entry the catalogue could not answer for comes back with its line, its text
/// as written and why — and a caller can ask for the whole answer *before*
/// anything is created, which is what `dry_run` is for.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PlaylistImport {
    /// The playlist as created, or `None` for a dry run.
    pub playlist: Option<PlaylistItem>,
    /// The name used, from the request or from the file's own `#PLAYLIST`.
    pub name: String,
    /// How the bytes were read as text, by the encoding's WHATWG name in lower
    /// case: `utf-8`, `utf-16le`, `windows-1252`, `shift_jis`, `gbk`… A file
    /// that announced itself as UTF-8 and came back anything else has been
    /// read, not mangled in silence.
    #[schema(value_type = String, example = "utf-8")]
    pub encoding: TextEncoding,
    /// Targets the file named, resolved or not.
    pub entries: usize,
    /// Tracks the playlist holds, or would hold.
    pub resolved: usize,
    pub unresolved: Vec<UnresolvedEntry>,
}

/// One target the import could not turn into a track.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UnresolvedEntry {
    /// One-based line in the file, so the report can be read against it.
    pub line: usize,
    /// The target as written.
    pub entry: String,
    /// Its `#EXTINF` text, when the file carried one.
    pub title: Option<String>,
    pub reason: UnresolvedReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UnresolvedReason {
    /// No path and no credit in this catalogue matched.
    NoMatch,
    /// Several tracks matched and nothing in the entry chooses between them.
    /// Importing the first would be a guess wearing the look of a result.
    Ambiguous,
    /// The target is not a path into a library — a web radio, most often.
    NotAPath,
    /// It resolved to a track the playlist already holds.
    ///
    /// No longer produced: a playlist holds a track as many times as it is
    /// listed (RFC-020 decision 1), so a file naming one twice is imported
    /// with both copies. Kept in the schema so that a client generated against
    /// it still reads every value it was told to expect.
    Duplicate,
}

/// Optional fields a playlist update may blank out. See [`ShareClear`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PlaylistClear {
    pub comment: bool,
    /// Drop the existing track list before applying `add`, which turns an
    /// update into a replacement. Subsonic's `createPlaylist` needs it: given a
    /// `playlistId`, its `songId` values are the whole playlist rather than
    /// additions to it. The native surface does not expose it.
    pub tracks: bool,
}
