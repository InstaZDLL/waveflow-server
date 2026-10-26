//! How a listing is asked for: the request ceilings, the browse page, and the
//! album and song orderings.

use std::str::FromStr;

use uuid::Uuid;

use crate::services::ServiceError;

/// Upper bound on a native browse page. It matches the Subsonic contract's
/// 500-item cap so both surfaces expose the same paging ceiling.
pub const MAX_BROWSE_LIMIT: i64 = 500;
const DEFAULT_BROWSE_LIMIT: i64 = 100;
pub const MAX_HISTORY_LIMIT: i64 = 500;
/// How many identifiers one request may name when each costs a mutation of its
/// own under the writer gate — a Subsonic `star` or `scrobble` naming many —
/// or a sealed pass of its own: a share's opening mints one per track.
pub const MAX_REQUEST_IDENTIFIERS: usize = 400;
/// Upper bound on the tracks a play queue holds (RFC-022, what was still
/// open), shared by the native queue and Subsonic `savePlayQueue`.
///
/// **The largest playlist**, because the common way to fill a queue is to play
/// a playlist whole, and a queue smaller than the playlist it was filled from
/// loses its end on the next save — silently, on a client that saves the
/// queue as it plays. Four hundred, the old ceiling, cut off ordinary
/// libraries.
///
/// **Bounded under the writer gate** as a playlist is: a save deletes and
/// reinserts the list in one statement each, and checks the tracks with one
/// count rather than reading every song (`save_queue`). **And on the wire**:
/// the native route carries [`queue_body_limit`], and a `savePlayQueue` of
/// this many `id=<uuid>&` pairs, forty bytes each, is 400,000 bytes — inside
/// the Subsonic form bound, which is sized for the longer `songIdToAdd` pair
/// at this same count.
pub const MAX_QUEUE_TRACKS: usize = MAX_PLAYLIST_TRACKS;
/// A share holds what one opening can hand a pass to each of.
pub const MAX_SHARE_TRACKS: usize = MAX_REQUEST_IDENTIFIERS;
/// Upper bound on the tracks one playlist may hold.
///
/// Deliberately not [`MAX_QUEUE_TRACKS`]: that one bounds a request, and a
/// queue is written whole by every call. A playlist grows across many calls, so
/// the same number would refuse ordinary libraries. What this bounds is the
/// rewrite — `replace_playlist_tracks` deletes and reinserts the whole list on
/// every edit, under the process-wide writer gate — and ten thousand keeps that
/// bounded while sitting far above any playlist a person curates by hand.
pub const MAX_PLAYLIST_TRACKS: usize = 10_000;

/// The body ceiling the playlist import route carries.
///
/// Derived from [`MAX_PLAYLIST_TRACKS`] rather than configured, like the upload
/// negotiation limit and for the same reason: a file holding the most tracks a
/// playlist may hold has to fit, and raising that ceiling must not quietly
/// start refusing the files it has just permitted. Half a kibibyte per entry
/// covers a path and an `#EXTINF` line with room to spare — a deep tree with
/// long names is nowhere near it — and the envelope covers the header lines.
pub fn playlist_import_body_limit() -> usize {
    MAX_PLAYLIST_TRACKS.saturating_mul(512).saturating_add(4096)
}

/// The body ceiling the native queue route carries, derived from
/// [`MAX_QUEUE_TRACKS`] like the import's from the playlist ceiling: a JSON
/// string of a UUID with its quotes and comma is thirty-nine bytes, so
/// sixty-four covers any spacing a client adds, and the envelope covers the
/// other fields.
pub fn queue_body_limit() -> usize {
    MAX_QUEUE_TRACKS.saturating_mul(64).saturating_add(4096)
}

/// Offset/limit pair validated once, at the HTTP boundary, so the SQL layer can
/// bind it without re-checking bounds.
#[derive(Debug, Clone, Copy)]
pub struct BrowsePage {
    pub(in crate::services) offset: i64,
    pub(in crate::services) limit: i64,
}

impl BrowsePage {
    pub fn new(offset: Option<i64>, limit: Option<i64>) -> Result<Self, ServiceError> {
        let offset = offset.unwrap_or(0);
        let limit = limit.unwrap_or(DEFAULT_BROWSE_LIMIT);
        if offset < 0 || limit <= 0 || limit > MAX_BROWSE_LIMIT {
            return Err(ServiceError::Invalid);
        }
        Ok(Self { offset, limit })
    }
}

impl Default for BrowsePage {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: DEFAULT_BROWSE_LIMIT,
        }
    }
}

/// How an album listing is ordered and filtered.
///
/// This is the single implementation of the ten Subsonic `getAlbumList2` modes,
/// and it lives in the domain services rather than in the facade for two
/// reasons. The facade used to sort in Rust over [`DomainServices::catalog_snapshot`](crate::services::DomainServices::catalog_snapshot),
/// which materialises every folder, artist, album *and track* the tenant can
/// see on each call — `byGenre` then rescanned every track once per album, and
/// `songCount` needed the whole track list just to be counted. And the native
/// API had no ordering at all, so the web client could not ask for "recently
/// added" without paging the entire catalogue itself.
///
/// The variant names are the Subsonic `type` values verbatim, so the facade
/// stays a parameter adapter with no vocabulary of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlbumOrder {
    #[default]
    AlphabeticalByName,
    AlphabeticalByArtist,
    Newest,
    Highest,
    Frequent,
    Recent,
    Starred,
    Random,
    ByYear,
    ByGenre,
}

impl FromStr for AlbumOrder {
    type Err = ServiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "alphabeticalByName" => Self::AlphabeticalByName,
            "alphabeticalByArtist" => Self::AlphabeticalByArtist,
            "newest" => Self::Newest,
            "highest" => Self::Highest,
            "frequent" => Self::Frequent,
            "recent" => Self::Recent,
            "starred" => Self::Starred,
            "random" => Self::Random,
            "byYear" => Self::ByYear,
            "byGenre" => Self::ByGenre,
            _ => return Err(ServiceError::Invalid),
        })
    }
}

/// One album listing request.
#[derive(Debug, Clone, Default)]
pub struct AlbumListQuery {
    /// Restrict to these libraries. Empty means every library the user can see.
    /// It is a set because Subsonic sends repeated `musicFolderId` values.
    pub library_ids: Vec<Uuid>,
    pub order: AlbumOrder,
    /// Required by [`AlbumOrder::ByGenre`], ignored otherwise.
    pub genre: Option<String>,
    /// Bounds for [`AlbumOrder::ByYear`], inclusive. Supplying them reversed is
    /// how Subsonic asks for descending years, and that is preserved here.
    pub from_year: Option<i64>,
    pub to_year: Option<i64>,
    pub page: BrowsePage,
}

/// How a song listing is ordered.
///
/// The same vocabulary as [`AlbumOrder`], so one catalogue does not answer to
/// two spellings, plus `alphabeticalByAlbum`, which an album listing has no
/// use for and a song listing does: it is sleeve order across the catalogue.
///
/// **A missing tag sorts last here, where an album listing sorts it first.**
/// `list_albums` orders on `COALESCE(album_artist_name, '')`, so an album
/// with no credited artist leads the list; these orderings say `NULLS LAST`.
/// The divergence is deliberate and one-way: `list_albums` is the single
/// implementation behind `getAlbumList2`, whose ordering four clients were
/// replayed against, so correcting it there is an observable wire change under
/// a frozen contract. Correcting it here costs nothing, because nothing has
/// read this route yet.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SongOrder {
    #[default]
    AlphabeticalByName,
    AlphabeticalByArtist,
    AlphabeticalByAlbum,
    Newest,
    Highest,
    Frequent,
    Recent,
    Starred,
    /// A fresh draw per request, so it **does not page**: asking for an
    /// offset gives a new sample rather than the next slice, which can repeat
    /// rows already seen and skip others. Left answering rather than refused,
    /// because [`AlbumOrder::Random`] answers the same way and sits behind
    /// `getAlbumList2?type=random` — refusing here alone would give one
    /// catalogue two rules for one question. A caller wanting a bounded draw
    /// has `/api/v2/songs/random`, which takes a limit and no offset at all.
    Random,
    ByYear,
    ByGenre,
}

impl FromStr for SongOrder {
    type Err = ServiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "alphabeticalByName" => Self::AlphabeticalByName,
            "alphabeticalByArtist" => Self::AlphabeticalByArtist,
            "alphabeticalByAlbum" => Self::AlphabeticalByAlbum,
            "newest" => Self::Newest,
            "highest" => Self::Highest,
            "frequent" => Self::Frequent,
            "recent" => Self::Recent,
            "starred" => Self::Starred,
            "random" => Self::Random,
            "byYear" => Self::ByYear,
            "byGenre" => Self::ByGenre,
            _ => return Err(ServiceError::Invalid),
        })
    }
}

/// One song listing request.
#[derive(Debug, Clone, Default)]
pub struct SongListQuery {
    /// Restrict to these libraries. Empty means every library the user can see.
    pub library_ids: Vec<Uuid>,
    pub order: SongOrder,
    /// Required by [`SongOrder::ByGenre`], ignored otherwise — the same rule
    /// [`AlbumListQuery::genre`] follows.
    pub genre: Option<String>,
    /// Bounds for [`SongOrder::ByYear`], inclusive. Supplied reversed, they ask
    /// for descending years, as they do for albums.
    pub from_year: Option<i64>,
    pub to_year: Option<i64>,
    pub page: BrowsePage,
}
