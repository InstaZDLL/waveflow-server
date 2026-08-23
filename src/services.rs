//! Shared v2 domain services and tenant-filtered read models.

use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};

use serde::Serialize;
use sqlx::{Row, SqliteConnection};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    authentication::now_ms,
    database::{AccountRecord, AccountRole, ApiTokenRecord, Database},
    lyrics::{self, LyricsList, StructuredLyrics},
    security::{self, EncryptedSecret, SecretBox},
    sync::{MutationContext, MutationIntent, MutationReceipt, OperationClaim, SyncService},
};

/// Tenant-filtered projections shared by the Subsonic facade and the native
/// browse endpoints. Each expands to a literal ending at `WHERE m.user_id=?` so
/// callers `concat!` their own predicates onto it — sqlx only accepts static SQL,
/// which keeps these compositions injection-proof by construction. The first
/// bind is always the user id.
macro_rules! song_select {
    () => {
        "SELECT t.id, t.library_id, t.album_id, t.title, t.album_title, t.artist_display, \
                (SELECT tp.artist_id FROM track_participant tp WHERE tp.track_id=t.id AND tp.role='artist' AND tp.position=0 \
                 ORDER BY tp.position LIMIT 1) AS artist_id, \
                t.genre_display, t.year, t.track_number, t.disc_number, t.duration_ms, t.bitrate, \
                t.codec, t.relative_path, t.file_size, t.artwork_hash, t.full_hash, t.created_at, \
                us.starred_at, ur.rating AS user_rating, \
                t.sample_rate, t.channels, t.bit_depth, \
                (SELECT COUNT(*) FROM play_event pe \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pe.track_id=t.id) \
                 AS play_count, \
                (SELECT MAX(pe.played_at) FROM play_event pe \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pe.track_id=t.id) \
                 AS last_played_at, \
                t.musicbrainz_recording_id, t.replay_gain_track_gain, t.replay_gain_track_peak, \
                t.replay_gain_album_gain, t.replay_gain_album_peak, t.bpm, t.sort_title, \
                t.comment, t.isrc, t.moods, t.explicit_status, \
                alb.album_artist_name, alb.album_artist_id \
         FROM track t JOIN library_member m ON m.library_id=t.library_id \
         LEFT JOIN album alb ON alb.id=t.album_id \
         LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='track' AND us.entity_id=t.id \
         LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='track' AND ur.entity_id=t.id \
         WHERE m.user_id=? AND t.is_available=1"
    };
}

/// Narrows [`song_select!`] to an optional set of libraries. Binds the JSON
/// library list twice, as the album and artist scopes do.
macro_rules! song_folder_clause {
    () => {
        " AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?)))"
    };
}

/// Restricts [`song_select!`] to one genre, matched on `genre.canonical_name`
/// so case, punctuation and spacing fold exactly as they do in `getGenres` and
/// in the `byGenre` album filter. Binds the canonical name once.
macro_rules! song_genre_clause {
    () => {
        " AND t.id IN (SELECT tg.track_id FROM track_genre tg \
            JOIN genre g ON g.id=tg.genre_id WHERE g.canonical_name=?)"
    };
}

/// An optional inclusive year range. Binds a flag and the two bounds, so one
/// literal serves both the filtered and the unfiltered request rather than
/// forking the statement.
macro_rules! song_year_clause {
    () => {
        " AND (? = 0 OR (t.year IS NOT NULL AND t.year BETWEEN ? AND ?))"
    };
}

macro_rules! album_select {
    () => {
        "SELECT al.id, al.library_id, al.title, al.album_artist_name, al.album_artist_id, \
                al.artwork_hash, al.year, al.is_compilation, al.musicbrainz_id, al.sort_name, \
                al.created_at, us.starred_at, \
                ur.rating AS user_rating, \
                (SELECT COUNT(*) FROM play_event pe JOIN track pt ON pt.id=pe.track_id \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pt.album_id=al.id) AS play_count, \
                (SELECT MAX(pe.played_at) FROM play_event pe JOIN track pt ON pt.id=pe.track_id \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pt.album_id=al.id) AS last_played_at, \
                (SELECT COUNT(*) FROM track t2 WHERE t2.album_id=al.id AND t2.is_available=1) \
                 AS song_count, \
                (SELECT COALESCE(SUM(t2.duration_ms), 0) FROM track t2 \
                 WHERE t2.album_id=al.id AND t2.is_available=1) AS duration_ms \
         FROM album al JOIN library_member m ON m.library_id=al.library_id \
         LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='album' AND us.entity_id=al.id \
         LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='album' AND ur.entity_id=al.id \
         WHERE m.user_id=?"
    };
}

/// [`album_select!`] narrowed to an optional set of libraries and wrapped so a
/// caller can filter and order on the projected aggregates — `play_count`,
/// `last_played_at`, `song_count` — instead of repeating their subqueries.
/// SQLite does not accept a result alias in `WHERE`, hence the wrapper rather
/// than a longer predicate list. Binds are the user id, then the JSON library
/// list twice.
macro_rules! album_scope {
    () => {
        concat!(
            "SELECT * FROM (",
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?)))) AS a"
        )
    };
}

/// The artist projection, in the two shapes the catalogue reads it.
///
/// `artist_select!()` stops at the columns `ArtistItem` carries;
/// `artist_select!(album_count)` adds the count `ArtistSummary` needs, so a
/// browse that never renders it does not pay a correlated subquery per artist.
/// Both expand from the same column list on purpose: the browses that wanted
/// the short shape used to spell it out by hand, and one of those copies fell
/// a column behind the day this list gained one — the browse reading it then
/// failed on a column the query never selected, which nothing reading the
/// macro could have predicted.
macro_rules! artist_select {
    () => {
        artist_select!(@columns "")
    };
    (album_count) => {
        artist_select!(
            @columns ", COALESCE((SELECT ars.album_count FROM artist_role_stats ars \
                                   WHERE ars.artist_id=ar.id AND ars.role='albumartist'), 0) \
                      AS album_count"
        )
    };
    (@columns $extra:expr) => {
        concat!(
            "SELECT ar.id, ar.library_id, ar.name, ar.artwork_hash, ar.musicbrainz_id, \
                    ar.sort_name, us.starred_at, ur.rating AS user_rating",
            $extra,
            " FROM artist ar JOIN library_member m ON m.library_id=ar.library_id \
              LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='artist' AND us.entity_id=ar.id \
              LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='artist' AND ur.entity_id=ar.id \
              WHERE m.user_id=?"
        )
    };
}

pub struct SubsonicCredentialRecord {
    pub account: AccountRecord,
    pub encrypted_password: EncryptedSecret,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MusicFolderItem {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtistItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub name: String,
    pub artwork_hash: Option<String>,
    pub musicbrainz_id: Option<String>,
    /// The tagged sort form of the name, `None` when no file supplied one.
    /// The Subsonic node emits it empty in that case rather than omitting it:
    /// the field is supported, and this artist is untagged.
    pub sort_name: Option<String>,
    pub starred_at: Option<i64>,
    pub user_rating: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AlbumItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub artist_id: Option<Uuid>,
    pub artwork_hash: Option<String>,
    pub year: Option<i64>,
    pub is_compilation: bool,
    pub musicbrainz_id: Option<String>,
    /// The tagged sort form of the title, on the same terms as the artist's.
    pub sort_name: Option<String>,
    /// Every artist credited on the album's available tracks, and every
    /// genre they carry. Derived rather than stored: an album has no credit
    /// or genre of its own in the schema, only the union of its files'.
    pub artists: Vec<ArtistRef>,
    pub genres: Vec<String>,
    pub created_at: i64,
    pub starred_at: Option<i64>,
    pub user_rating: Option<i64>,
    pub play_count: i64,
    pub last_played_at: Option<i64>,
    /// Available tracks in the album, and their total duration in
    /// milliseconds. Projected here rather than derived by the caller: album
    /// listings used to compute both by loading every track of the tenant, so
    /// the counts cost a full catalogue read per request.
    pub song_count: i64,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SongItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub album_id: Option<Uuid>,
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    /// Primary credited artist, matching the first artist in `artist`.
    pub artist_id: Option<Uuid>,
    pub genre: Option<String>,
    pub year: Option<i64>,
    pub track: Option<i64>,
    pub disc: Option<i64>,
    pub duration_ms: i64,
    pub bitrate: Option<i64>,
    pub codec: Option<String>,
    pub suffix: String,
    pub size: i64,
    pub artwork_hash: Option<String>,
    /// Content fingerprint: **BLAKE3, unkeyed, hexadecimal, over the whole
    /// file** — 64 characters. A client can compute the same value locally and
    /// compare, which is the only automatic link M5 accepts (a unique full-hash
    /// match; MBID stays a suggestion to confirm).
    ///
    /// It fingerprints the *file*, not the decoded audio: two copies of one
    /// recording with different tags do not match. The algorithm is part of the
    /// contract — changing it means adding a field, never redefining this one.
    pub full_hash: String,
    pub created_at: i64,
    pub starred_at: Option<i64>,
    pub user_rating: Option<i64>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub bit_depth: Option<i64>,
    pub play_count: i64,
    pub last_played_at: Option<i64>,
    /// Every credited artist, in tag order. `artist` and `artist_id` stay the
    /// display string and the primary credit; these are the structured form
    /// `track_artist` has always held and no surface ever read.
    pub artists: Vec<ArtistRef>,
    /// Every genre of the track, from `track_genre` rather than from the
    /// semicolon-joined `genre` display string.
    pub genres: Vec<String>,
    /// The credit the album carries, which is not always the track's own: a
    /// guest appearance names the guest, and the album still belongs under
    /// the album artist.
    pub album_artist: Option<String>,
    pub album_artist_id: Option<Uuid>,
    /// The MusicBrainz recording identifier: the performance, which is what
    /// OpenSubsonic means by a song's `musicBrainzId`. RFC-004 keeps a match
    /// on it a candidate the user confirms, never an automatic link.
    pub musicbrainz_id: Option<String>,
    pub replay_gain_track_gain: Option<f64>,
    pub replay_gain_track_peak: Option<f64>,
    pub replay_gain_album_gain: Option<f64>,
    pub replay_gain_album_peak: Option<f64>,
    pub bpm: Option<i64>,
    pub sort_name: Option<String>,
    pub comment: Option<String>,
    /// Split from the tag the same way artists and genres are.
    pub isrc: Vec<String>,
    pub moods: Vec<String>,
    /// `explicit` or `clean`; the scanner stores nothing else.
    pub explicit_status: Option<String>,
}

/// One credited artist of a track. Only `id` and `name` are carried: those are
/// the required `ArtistID3` fields, and OpenSubsonic asks for no more than the
/// required ones inside a media item.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtistRef {
    pub id: Uuid,
    pub name: String,
}

/// A browse view that stops short of the tracks.
#[derive(Debug, Clone)]
pub struct CatalogOverview {
    pub folders: Vec<MusicFolderItem>,
    pub artists: Vec<ArtistItem>,
    pub albums: Vec<AlbumItem>,
}

#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    pub folders: Vec<MusicFolderItem>,
    pub artists: Vec<ArtistItem>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
}

/// Everything one account has starred, across the three entity kinds.
#[derive(Debug, Clone)]
pub struct StarredCatalog {
    pub artists: Vec<ArtistSummary>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
}

/// Result of a Subsonic `search3`, backed by the FTS5 index.
#[derive(Debug, Clone)]
pub struct CatalogSearch {
    pub artists: Vec<ArtistItem>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
}

/// Upper bound on a native browse page. It matches the Subsonic contract's
/// 500-item cap so both surfaces expose the same paging ceiling.
pub const MAX_BROWSE_LIMIT: i64 = 500;
const DEFAULT_BROWSE_LIMIT: i64 = 100;
pub const MAX_HISTORY_LIMIT: i64 = 500;
/// Fits a UUID-only queue request below the server's 16 KiB body limit while
/// also bounding the work performed under the global SQLite writer gate.
pub const MAX_QUEUE_TRACKS: usize = 400;
/// Applies the same request-size and writer-gate bound to public shares.
pub const MAX_SHARE_TRACKS: usize = MAX_QUEUE_TRACKS;

/// Offset/limit pair validated once, at the HTTP boundary, so the SQL layer can
/// bind it without re-checking bounds.
#[derive(Debug, Clone, Copy)]
pub struct BrowsePage {
    offset: i64,
    limit: i64,
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
/// reasons. The facade used to sort in Rust over [`DomainServices::catalog_snapshot`],
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

/// A genre with the size of what it holds, aggregated across the libraries the
/// user can see.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GenreItem {
    pub name: String,
    pub song_count: i64,
    pub album_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtistSummary {
    #[serde(flatten)]
    pub artist: ArtistItem,
    pub album_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AlbumDetail {
    #[serde(flatten)]
    pub album: AlbumItem,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtistDetail {
    #[serde(flatten)]
    pub artist: ArtistItem,
    /// Same field the list endpoint returns. `albums` below is unpaginated, so
    /// its length matches — but that is an unwritten guarantee, and a client
    /// should not have to depend on one.
    pub album_count: i64,
    pub albums: Vec<AlbumItem>,
}

/// Inputs for a native client's authorization request.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizationRequest<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub code_challenge: &'a str,
    pub code_challenge_method: &'a str,
    pub device_name: &'a str,
    pub state: Option<&'a str>,
    /// The scopes of the credential authorizing this grant. Recorded on the
    /// grant so the session redeemed from it inherits them, which is what
    /// keeps a session from ever being broader than what asked for it.
    pub scopes: &'a [String],
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SearchResult {
    pub artists: Vec<ArtistItem>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PlaylistItem {
    pub id: Uuid,
    pub name: String,
    pub comment: Option<String>,
    pub public: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub songs: Vec<SongItem>,
}

/// A playback position the user saved in one track.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BookmarkItem {
    pub position_ms: i64,
    pub comment: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    /// The bookmarked track, resolved through the same tenant-filtered
    /// projection every other surface reads.
    pub song: SongItem,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QueueItem {
    pub current: Option<Uuid>,
    pub position_ms: i64,
    pub changed_by: Option<String>,
    pub updated_at: i64,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RatingItem {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub rating: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HistoryItem {
    pub track_id: Uuid,
    pub submission: bool,
    pub played_at: i64,
}

/// Optional fields a share update may blank out.
///
/// `COALESCE(?, column)` cannot express this: an absent field and an explicit
/// null arrive as the same bind, so "leave it alone" and "remove it" collapse.
/// The consequence was not cosmetic — an expiry set by mistake could never be
/// lifted, and the owner's only recourse was to delete the share and mint a new
/// URL. Clearing is opt-in and named, so a client that merely omits a field can
/// never erase one by accident.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShareClear {
    pub description: bool,
    pub expires_at: bool,
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

#[derive(Debug, Clone)]
pub struct ShareItem {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// Present only in the result of a newly-created share. Persistent reads
    /// deliberately cannot recover the bearer token from its lookup hash.
    pub url_token: Option<String>,
    pub description: Option<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub visit_count: i64,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserItem {
    pub id: Uuid,
    pub username: String,
    pub role: AccountRole,
    pub disabled: bool,
    pub has_subsonic_credential: bool,
    pub folder_ids: Vec<Uuid>,
}

pub struct UserUpdate<'a> {
    pub admin: Option<bool>,
    pub disabled: Option<bool>,
    pub folder_ids: Option<&'a [Uuid]>,
    pub subsonic_password: Option<&'a str>,
    pub web_password: Option<&'a str>,
}

pub struct SyncSnapshotData {
    pub cursor: i64,
    pub playlists: Vec<PlaylistItem>,
    pub favorites: Vec<(String, Uuid, i64)>,
    pub ratings: Vec<RatingItem>,
    pub queue: Option<QueueItem>,
    pub history: Vec<HistoryItem>,
    pub shares: Vec<ShareItem>,
    pub bookmarks: Vec<BookmarkItem>,
}

#[derive(Clone)]
pub struct DomainServices {
    db: Database,
    secret_box: Arc<SecretBox>,
    sync: SyncService,
    scanner: crate::scanner::ScanManager,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("resource not found")]
    NotFound,
    #[error("operation is forbidden")]
    Forbidden,
    #[error("invalid input")]
    Invalid,
    #[error("conflict")]
    Conflict,
    #[error("service unavailable")]
    Unavailable,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Security(#[from] security::SecurityError),
}

impl From<crate::sync::SyncError> for ServiceError {
    fn from(error: crate::sync::SyncError) -> Self {
        match error {
            crate::sync::SyncError::Invalid => Self::Invalid,
            // Mutations claim operations; they never read the journal, so
            // CursorExpired cannot reach this conversion. Folded into Conflict
            // rather than given a domain variant nothing would ever construct.
            crate::sync::SyncError::Conflict | crate::sync::SyncError::CursorExpired => {
                Self::Conflict
            }
            crate::sync::SyncError::Database(error) => Self::Database(error),
        }
    }
}

impl DomainServices {
    pub fn new(
        db: Database,
        secret_box: Arc<SecretBox>,
        sync: SyncService,
        scanner: crate::scanner::ScanManager,
    ) -> Self {
        Self {
            db,
            secret_box,
            sync,
            scanner,
        }
    }

    /// Queues a rescan of one library the user can reach.
    ///
    /// The single implementation behind `POST /api/v2/libraries/{id}/scans`
    /// and the Subsonic `startScan`. Both surfaces have to answer the same
    /// question about who may scan what, so the membership check cannot sit
    /// in a handler where the two copies can drift apart.
    pub async fn start_library_scan(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<Uuid, ServiceError> {
        let library = self
            .db
            .library_for_user(user_id, library_id)
            .await?
            .ok_or(ServiceError::NotFound)?;
        // The lookup above reads the root path; it is not what authorises the
        // job. The insert tests `library_member` itself — membership and role
        // together — so an access revoked or downgraded between the two refuses
        // the job instead of queuing work the requester may no longer ask for.
        let scan_id = self
            .db
            .create_scan_job_for_user(user_id, library_id, "manual")
            .await?
            .ok_or(ServiceError::NotFound)?;
        self.scanner.spawn(scan_id, library);
        Ok(scan_id)
    }

    /// Queues a rescan of every library the user may scan, for the Subsonic
    /// `startScan`, which takes no library parameter.
    ///
    /// Libraries the account only listens to are skipped rather than attempted
    /// and reported: `startScan` names no library, so refusing the whole call
    /// because one of the account's libraries is read-only would put the
    /// scannable ones out of reach from Subsonic entirely.
    ///
    /// An account that may scan nothing therefore queues nothing and succeeds,
    /// like an account that reaches no library at all: there is no missing
    /// resource to report, and every other catalogue-wide method answers such
    /// an account with an empty result rather than an error.
    ///
    /// Best effort by design: a library whose job cannot be queued does not
    /// cancel the ones that can. Aborting on the first failure would leave
    /// the caller reading an error while half the catalogue is already
    /// rescanning, which is the worst of both answers. The error surfaces
    /// only when nothing at all could be queued.
    ///
    /// Re-queuing a library that is already scanning is deliberately allowed,
    /// exactly as calling the native endpoint twice is: [`crate::scanner::ScanManager`]
    /// serialises jobs per library and a scan converges on file content, so a
    /// redundant pass costs time and changes nothing.
    pub async fn start_visible_scans(&self, user_id: Uuid) -> Result<Vec<Uuid>, ServiceError> {
        let libraries = self.db.libraries_for_user(user_id).await?;
        let mut queued = Vec::new();
        let mut failure = None;
        for access in libraries
            .into_iter()
            .filter(|access| access.role.may_scan())
        {
            match self.start_library_scan(user_id, access.id).await {
                Ok(scan_id) => queued.push(scan_id),
                Err(error) => failure = Some(error),
            }
        }
        match failure {
            Some(error) if queued.is_empty() => Err(error),
            _ => Ok(queued),
        }
    }

    pub async fn bootstrap_admin(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Uuid, ServiceError> {
        validate_username(username)?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || security::hash_password(&password))
            .await
            .map_err(|_| ServiceError::Unavailable)??;
        self.db
            .bootstrap_admin(username, &password_hash, now_ms())
            .await?
            .ok_or(ServiceError::Conflict)
    }

    pub async fn credential_by_username(
        &self,
        username: &str,
    ) -> Result<Option<SubsonicCredentialRecord>, ServiceError> {
        let row = sqlx::query(
            "SELECT a.id, a.username, a.password_hash, a.role, a.disabled, \
                    c.password_nonce, c.password_ciphertext \
             FROM account a JOIN subsonic_credential c ON c.user_id=a.id \
             WHERE a.username=? COLLATE NOCASE AND a.disabled=0",
        )
        .bind(username)
        .fetch_optional(self.db.pool())
        .await?;
        row.map(credential_from_row).transpose().map_err(Into::into)
    }

    pub async fn credential_by_api_key(
        &self,
        api_key: &str,
    ) -> Result<Option<SubsonicCredentialRecord>, ServiceError> {
        let hash = security::token_hash(api_key);
        let row = sqlx::query(
            "SELECT a.id, a.username, a.password_hash, a.role, a.disabled, \
                    c.password_nonce, c.password_ciphertext \
             FROM account a JOIN subsonic_credential c ON c.user_id=a.id \
             WHERE c.api_key_hash=? AND a.disabled=0",
        )
        .bind(hash.as_slice())
        .fetch_optional(self.db.pool())
        .await?;
        row.map(credential_from_row).transpose().map_err(Into::into)
    }

    pub fn decrypt_subsonic_password(
        &self,
        credential: &SubsonicCredentialRecord,
    ) -> Result<Vec<u8>, ServiceError> {
        self.secret_box
            .decrypt(
                &credential.encrypted_password.nonce,
                &credential.encrypted_password.ciphertext,
            )
            .map_err(Into::into)
    }

    /// The libraries one account can reach.
    ///
    /// `getMusicFolders` needs nothing else, and used to read the whole
    /// catalogue to answer with a handful of names.
    pub async fn music_folders(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
    ) -> Result<Vec<MusicFolderItem>, ServiceError> {
        let folder_filter = folder_filter(folder_ids);
        Ok(sqlx::query(
            "SELECT l.id, l.name FROM library l JOIN library_member m ON m.library_id=l.id \
             WHERE m.user_id=? AND (? IS NULL OR l.id IN (SELECT value FROM json_each(?))) \
             ORDER BY l.name COLLATE NOCASE",
        )
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(|row| {
            Ok(MusicFolderItem {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }

    /// Folders, artists and albums, without the tracks.
    ///
    /// Most of what browses the catalogue never looks at a track: an index of
    /// artists, an artist's albums, a folder's contents. Those used to read
    /// every visible track anyway, because one snapshot served every browse
    /// method, and the track read is by far the largest of the three — and
    /// since the OpenSubsonic fields landed it carries two relation loads of
    /// its own.
    pub async fn catalog_overview(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
    ) -> Result<CatalogOverview, ServiceError> {
        let folder_filter = folder_filter(folder_ids);
        let folders = self.music_folders(user_id, folder_ids).await?;
        let artists = sqlx::query(concat!(
            artist_select!(),
            " AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY ar.name COLLATE NOCASE"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY al.title COLLATE NOCASE"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        Ok(CatalogOverview {
            folders,
            artists,
            albums,
        })
    }

    /// The overview plus every visible track.
    ///
    /// **No route calls this.** Every browse method that used to now asks for
    /// what it renders, and nothing should reach for this again: it is the
    /// shape that made one album page read a tenant's whole catalogue.
    ///
    /// It survives as a fixture. The integration suite builds ids from it —
    /// "give me an album of this account so I can ask for it" — which is a
    /// legitimate use of a full read in a test with three tracks in it, and
    /// not one in a request.
    pub async fn catalog_snapshot(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
    ) -> Result<CatalogSnapshot, ServiceError> {
        let overview = self.catalog_overview(user_id, folder_ids).await?;
        let songs = fetch_songs(
            &self.db,
            user_id,
            folder_filter(folder_ids).as_deref(),
            None,
        )
        .await?;
        Ok(CatalogSnapshot {
            folders: overview.folders,
            artists: overview.artists,
            albums: overview.albums,
            songs,
        })
    }

    /// Backs Subsonic `search3` with the FTS5 index instead of materialising the
    /// whole catalogue and filtering it in memory.
    ///
    /// `track_fts` indexes title, album, artists and genres per track, so
    /// selecting matching tracks and deriving their albums and artists covers
    /// the same ground the in-memory pass did — a matching album title reaches
    /// its own tracks through the `album` column.
    ///
    /// Its tokenizer folds case *and* diacritics, so "echo" now finds "Écho",
    /// which the previous lowercase substring test did not. What it gives up is
    /// matching inside a word: "cho" no longer finds "Echo". The trailing term
    /// is treated as a prefix so search-as-you-type still works.
    pub async fn catalog_search(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
        query: &str,
    ) -> Result<CatalogSearch, ServiceError> {
        let Some(fts) = crate::catalog::fts_prefix_query(query) else {
            return Ok(CatalogSearch {
                artists: Vec::new(),
                albums: Vec::new(),
                songs: Vec::new(),
            });
        };
        let folder_filter = (!folder_ids.is_empty()).then(|| {
            serde_json::to_string(folder_ids).expect("UUID list serialization cannot fail")
        });
        let folder_filter = folder_filter.as_deref();

        let mut songs = sqlx::query(concat!(
            song_select!(),
            " AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
               AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
             ORDER BY t.title COLLATE NOCASE, t.id"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;

        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
               AND al.id IN (SELECT t.album_id FROM track t WHERE t.album_id IS NOT NULL \
                 AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?)) \
             ORDER BY al.title COLLATE NOCASE, al.id"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;

        let artists = sqlx::query(concat!(
            artist_select!(),
            " AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
                AND ar.id IN ( \
                  SELECT al.album_artist_id FROM album al JOIN track t ON t.album_id=al.id \
                  WHERE al.album_artist_id IS NOT NULL \
                    AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
                  UNION \
                  SELECT tp.artist_id FROM track_participant tp \
                  WHERE tp.track_id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
                ) \
              ORDER BY ar.name COLLATE NOCASE"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .bind(&fts)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;

        Ok(CatalogSearch {
            artists,
            albums,
            songs,
        })
    }

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

    /// Genres visible to the user, with the size of what each holds.
    ///
    /// Grouping is by `genre.canonical_name`, so one genre spelled differently
    /// across two libraries — or differing only in case — is a single row. The
    /// facade previously grouped the raw `genre_display` fragments, which
    /// listed "Rock" and "rock" as two genres with split counts.
    pub async fn list_genres(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
    ) -> Result<Vec<GenreItem>, ServiceError> {
        let folders = (!library_ids.is_empty()).then(|| {
            serde_json::to_string(library_ids).expect("UUID list serialization cannot fail")
        });
        Ok(sqlx::query(
            "SELECT MIN(g.name) AS name, COUNT(DISTINCT t.id) AS song_count, \
                    COUNT(DISTINCT t.album_id) AS album_count \
             FROM genre g JOIN library_member m ON m.library_id=g.library_id \
             JOIN track_genre tg ON tg.genre_id=g.id \
             JOIN track t ON t.id=tg.track_id AND t.is_available=1 \
             WHERE m.user_id=? AND (? IS NULL OR g.library_id IN (SELECT value FROM json_each(?))) \
             GROUP BY g.canonical_name ORDER BY name COLLATE NOCASE",
        )
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(|row| {
            Ok(GenreItem {
                name: row.try_get("name")?,
                song_count: row.try_get("song_count")?,
                album_count: row.try_get("album_count")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?)
    }

    /// Songs of one genre, ordered and paged in SQL.
    ///
    /// Matching is on `genre.canonical_name`, the same key `list_genres` groups
    /// by and `byGenre` filters on. It was `eq_ignore_ascii_case` against the
    /// joined display string, which folds case but not punctuation or spacing:
    /// `getGenres` answered one row for "Hip-Hop" and "Hip Hop", and asking for
    /// that row returned only the tracks spelled the way the caller happened to
    /// send. A client showed a genre it had just been given, and it was empty.
    pub async fn songs_by_genre(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
        genre: &str,
        page: BrowsePage,
    ) -> Result<Vec<SongItem>, ServiceError> {
        let folders = folder_filter(library_ids);
        let canonical = waveflow_core::scanner::canonical_name(genre);
        let mut songs = sqlx::query(concat!(
            song_select!(),
            song_folder_clause!(),
            song_genre_clause!(),
            " ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(&canonical)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(songs)
    }

    /// The available tracks of one library that belong to no album.
    ///
    /// A track without an album has no album id to be the `parent` of its
    /// Subsonic `child`, so it names its library instead. That was a
    /// dead end until now: browsing to that identifier listed the library's
    /// artists and nothing else, so a track reachable by search was reachable
    /// by no amount of browsing. Answering here is what makes the `parent`
    /// it already advertised true.
    ///
    /// `getMusicDirectory` has no offset to page a folder with, so the caller
    /// asks for a ceiling instead of a page: everything up to `limit`, in one
    /// answer. A library that holds more album-less tracks than that would
    /// build an unbounded response out of a request that cannot say how much
    /// it wants, so the ceiling is what keeps the answer finite — and the
    /// caller says so in the log rather than truncating in silence. The artist
    /// list this tail follows is still bounded only by the library.
    pub async fn songs_without_album(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        limit: i64,
    ) -> Result<Vec<SongItem>, ServiceError> {
        if limit <= 0 {
            return Err(ServiceError::Invalid);
        }
        let mut songs = sqlx::query(concat!(
            song_select!(),
            " AND t.library_id=? AND t.album_id IS NULL \
              ORDER BY t.title COLLATE NOCASE, t.id LIMIT ?"
        ))
        .bind(user_id.to_string())
        .bind(library_id.to_string())
        .bind(limit)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(songs)
    }

    /// A random selection, drawn in SQL rather than by shuffling the catalogue.
    ///
    /// The facade used to read every visible track, filter in Rust and shuffle
    /// the result to answer with ten. `ORDER BY RANDOM() LIMIT` asks SQLite for
    /// the same thing without materialising the rest, and the genre filter
    /// matches the canonical name like every other genre predicate.
    ///
    /// A reversed year range is how Subsonic asks for one, so the bounds are
    /// normalised rather than rejected.
    pub async fn random_songs(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
        genre: Option<&str>,
        from_year: Option<i64>,
        to_year: Option<i64>,
        limit: i64,
    ) -> Result<Vec<SongItem>, ServiceError> {
        if limit <= 0 || limit > MAX_BROWSE_LIMIT {
            return Err(ServiceError::Invalid);
        }
        let folders = folder_filter(library_ids);
        let canonical = genre.map(waveflow_core::scanner::canonical_name);
        let from = from_year.unwrap_or(i64::MIN);
        let to = to_year.unwrap_or(i64::MAX);
        let bounded = from_year.is_some() || to_year.is_some();
        let sql = match canonical.is_some() {
            true => concat!(
                song_select!(),
                song_folder_clause!(),
                song_genre_clause!(),
                song_year_clause!(),
                " ORDER BY RANDOM() LIMIT ?"
            ),
            false => concat!(
                song_select!(),
                song_folder_clause!(),
                song_year_clause!(),
                " ORDER BY RANDOM() LIMIT ?"
            ),
        };
        let mut statement = sqlx::query(sql)
            .bind(user_id.to_string())
            .bind(folders.as_deref())
            .bind(folders.as_deref());
        if let Some(canonical) = canonical.as_deref() {
            statement = statement.bind(canonical);
        }
        let mut songs = statement
            .bind(bounded)
            .bind(from.min(to))
            .bind(from.max(to))
            .bind(limit)
            .fetch_all(self.db.pool())
            .await?
            .into_iter()
            .map(song_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(songs)
    }

    /// Everything the account has starred, most recent first.
    ///
    /// The three projections already `LEFT JOIN user_star`, so this is the same
    /// read with the join made mandatory. The facade used to load the whole
    /// catalogue and look each starred id up inside it, which cost a full
    /// catalogue read to answer a list that is usually short.
    pub async fn starred(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
    ) -> Result<StarredCatalog, ServiceError> {
        let folders = folder_filter(library_ids);
        let artists = sqlx::query(concat!(
            artist_select!(album_count),
            " AND us.starred_at IS NOT NULL \
              AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY us.starred_at DESC, ar.id"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_summary_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND us.starred_at IS NOT NULL \
              AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY us.starred_at DESC, al.id"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        let mut songs = sqlx::query(concat!(
            song_select!(),
            " AND us.starred_at IS NOT NULL",
            song_folder_clause!(),
            " ORDER BY us.starred_at DESC, t.id"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(StarredCatalog {
            artists,
            albums,
            songs,
        })
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

    /// The whole visible catalogue, each kind ordered and paged in SQL.
    ///
    /// Subsonic clients send the literal `""` to `search3` as the documented
    /// match-all query, and page through it to build their initial library.
    /// FTS5 has no expression meaning "everything", so this is not a search at
    /// all — it is three ordinary listings under the search response. It used
    /// to read the entire catalogue and slice it in Rust, once per page, which
    /// made a client's first synchronization quadratic in the library.
    ///
    /// A page beyond the end is an empty list rather than an error: that is how
    /// a client learns it has reached the end.
    pub async fn browse_all(
        &self,
        user_id: Uuid,
        library_ids: &[Uuid],
        artists: BrowsePage,
        albums: BrowsePage,
        songs: BrowsePage,
    ) -> Result<CatalogSearch, ServiceError> {
        let folders = folder_filter(library_ids);
        let artists = sqlx::query(concat!(
            artist_select!(),
            " AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY ar.name COLLATE NOCASE, ar.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(artists.limit)
        .bind(artists.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
              ORDER BY al.title COLLATE NOCASE, al.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(albums.limit)
        .bind(albums.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        let mut songs = sqlx::query(concat!(
            song_select!(),
            song_folder_clause!(),
            " ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(songs.limit)
        .bind(songs.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        Ok(CatalogSearch {
            artists,
            albums,
            songs,
        })
    }

    /// Full-text search across the user's visible catalogue. Tracks are matched
    /// through the FTS5 index built in M1, which folds case and diacritics, so
    /// "echo" finds "Écho". Albums and artists are derived from the same index
    /// rather than a second scan, keeping one source of truth for relevance.
    ///
    /// Each kind is paged independently, as `search3` has always allowed:
    /// a client that has read every matching song should be able to ask for
    /// the next page of songs without re-reading the artists beside them.
    pub async fn search(
        &self,
        user_id: Uuid,
        query: &str,
        artists: BrowsePage,
        albums: BrowsePage,
        songs: BrowsePage,
    ) -> Result<SearchResult, ServiceError> {
        // Prefix on the trailing term, like the Subsonic surface: a client
        // querying on each keystroke would otherwise get nothing until the word
        // is complete — "ech" returned zero results while "echo" returned the
        // album. Native clients type incrementally just as Subsonic ones do.
        let Some(fts) = crate::catalog::fts_prefix_query(query) else {
            return Ok(SearchResult {
                artists: Vec::new(),
                albums: Vec::new(),
                songs: Vec::new(),
            });
        };
        let mut songs = sqlx::query(concat!(
            song_select!(),
            " AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
              ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(songs.limit)
        .bind(songs.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut *self.db.pool().acquire().await?, user_id, &mut songs).await?;
        let mut albums = sqlx::query(concat!(
            album_select!(),
            " AND al.id IN (SELECT t.album_id FROM track t \
                WHERE t.album_id IS NOT NULL \
                  AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?)) \
              ORDER BY al.title COLLATE NOCASE, al.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(albums.limit)
        .bind(albums.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_album_relations(&mut *self.db.pool().acquire().await?, user_id, &mut albums).await?;
        let artists = sqlx::query(concat!(
            artist_select!(),
            " AND ar.id IN (SELECT tp.artist_id FROM track_participant tp \
                WHERE tp.track_id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?)) \
              ORDER BY ar.name COLLATE NOCASE, ar.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(artists.limit)
        .bind(artists.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(SearchResult {
            artists,
            albums,
            songs,
        })
    }

    /// Issues an authorization code for a native client.
    ///
    /// Validation, credential generation and persistence live here rather than
    /// in the handler so the grant rules hold for every surface that ever
    /// issues one, and so they can be exercised without an HTTP request.
    /// Returns the URL the consent screen must send the user agent to.
    pub async fn authorize_native_client(
        &self,
        user_id: Uuid,
        request: AuthorizationRequest<'_>,
    ) -> Result<String, ServiceError> {
        crate::oauth::validate_redirect_uri(request.redirect_uri)
            .map_err(|_| ServiceError::Invalid)?;
        crate::oauth::validate_challenge(request.code_challenge_method, request.code_challenge)
            .map_err(|_| ServiceError::Invalid)?;
        let client_id = request.client_id.trim();
        let device_name = request.device_name.trim();
        // Checked before the code exists: a name the session issuer would
        // reject must not burn a grant the client can never redeem.
        if client_id.is_empty() || device_name.is_empty() || device_name.len() > 120 {
            return Err(ServiceError::Invalid);
        }

        let code = security::generate_token("wfc_");
        let now = now_ms();
        self.db
            .create_authorization(crate::database::NewAuthorization {
                code_hash: security::token_hash(&code),
                user_id,
                client_id,
                redirect_uri: request.redirect_uri,
                code_challenge: request.code_challenge,
                device_name,
                now_ms: now,
                expires_at: now + crate::oauth::AUTHORIZATION_CODE_TTL_MS,
                scopes: request.scopes,
            })
            .await?;
        Ok(crate::oauth::redirect_with_code(
            request.redirect_uri,
            &code,
            request.state,
        ))
    }

    pub async fn songs_by_ids(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.songs_by_ids_on(&mut connection, user_id, ids).await
    }

    /// Lyrics for one visible, available track. A visible track with no lyrics
    /// returns an empty list; an unknown or foreign track is blurred as not
    /// found, matching the rest of the catalogue API.
    pub async fn lyrics(&self, user_id: Uuid, track_id: Uuid) -> Result<LyricsList, ServiceError> {
        let rows = sqlx::query(
            "SELECT t.id, t.title, t.artist_display, tl.lang, tl.synced, tl.content \
             FROM track t JOIN library_member m ON m.library_id=t.library_id \
             LEFT JOIN track_lyrics tl ON tl.track_id=t.id AND tl.library_id=t.library_id \
             WHERE m.user_id=? AND t.id=? AND t.is_available=1 \
             ORDER BY tl.position",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        lyrics_list_from_rows(track_id, rows)
    }

    /// Legacy Subsonic lookup by metadata. Matching stays tenant-scoped and
    /// deterministic; it is intentionally exact because fuzzy catalogue
    /// reconciliation is outside the v2 contract.
    pub async fn lyrics_by_metadata(
        &self,
        user_id: Uuid,
        artist: Option<&str>,
        title: Option<&str>,
    ) -> Result<Option<LyricsList>, ServiceError> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT t.id FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE m.user_id=? AND t.is_available=1 \
               AND (? IS NULL OR t.artist_display = ? COLLATE NOCASE) \
               AND (? IS NULL OR t.title = ? COLLATE NOCASE) \
               AND EXISTS (SELECT 1 FROM track_lyrics tl WHERE tl.track_id=t.id) \
             ORDER BY t.title COLLATE NOCASE, t.id LIMIT 1",
        )
        .bind(user_id.to_string())
        .bind(artist)
        .bind(artist)
        .bind(title)
        .bind(title)
        .fetch_optional(self.db.pool())
        .await?;
        let track_id = row
            .map(|id| {
                Uuid::parse_str(&id)
                    .map_err(|error| ServiceError::Database(sqlx::Error::Decode(error.into())))
            })
            .transpose()?;
        match track_id {
            Some(track_id) => self.lyrics(user_id, track_id).await.map(Some),
            None => Ok(None),
        }
    }

    pub async fn sync_snapshot(
        &self,
        user_id: Uuid,
        history_limit: i64,
    ) -> Result<SyncSnapshotData, ServiceError> {
        let mut tx = self.db.pool().begin().await?;
        // A global watermark, read inside the same transaction as the rows
        // below so nothing committed after it can be missed.
        //
        // Deliberately not this user's MAX: `changes` refuses cursors below the
        // journal's global floor, so a per-user watermark would hand an account
        // with no surviving events a cursor beneath that floor — it would
        // re-snapshot, get the same cursor, be refused again, and loop. Filtering
        // by user still happens in `changes`, so a global watermark only means
        // "everything up to here is already in this snapshot".
        let cursor = sqlx::query_scalar("SELECT COALESCE(MAX(cursor), 0) FROM sync_event")
            .fetch_one(&mut *tx)
            .await?;
        let playlists = self.playlists_on(&mut tx, user_id).await?;
        let favorites = self.starred_ids_on(&mut tx, user_id).await?;
        let ratings = self.ratings_on(&mut tx, user_id).await?;
        let queue = self.queue_on(&mut tx, user_id).await?;
        let history = self.history_on(&mut tx, user_id, history_limit).await?;
        let shares = self.shares_on(&mut tx, user_id).await?;
        let bookmarks = self.bookmarks_on(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(SyncSnapshotData {
            cursor,
            playlists,
            favorites,
            ratings,
            queue,
            history,
            shares,
            bookmarks,
        })
    }

    async fn songs_by_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        let songs = self
            .songs_by_ids_lenient_on(connection, user_id, ids)
            .await?;
        if songs.len() == ids.len() {
            Ok(songs)
        } else {
            Err(ServiceError::NotFound)
        }
    }

    async fn songs_by_ids_lenient_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids_json = serde_json::to_string(ids).map_err(|_| ServiceError::Invalid)?;
        let rows = sqlx::query(concat!(
            song_select!(),
            " AND t.id IN (SELECT value FROM json_each(?))"
        ))
        .bind(user_id.to_string())
        .bind(ids_json)
        .fetch_all(&mut *connection)
        .await?;
        let mut resolved = rows
            .into_iter()
            .map(song_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(connection, user_id, &mut resolved).await?;
        let available = resolved
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        Ok(ids
            .iter()
            .filter_map(|id| available.get(id).cloned())
            .collect())
    }

    pub async fn artwork_for_user(
        &self,
        user_id: Uuid,
        id: &str,
    ) -> Result<Option<(String, String)>, ServiceError> {
        let row = sqlx::query(
            "SELECT a.hash, a.format FROM artwork a WHERE a.hash=? AND EXISTS ( \
               SELECT 1 FROM track t JOIN library_member m ON m.library_id=t.library_id WHERE t.artwork_hash=a.hash AND m.user_id=? \
               UNION SELECT 1 FROM album al JOIN library_member m ON m.library_id=al.library_id WHERE al.artwork_hash=a.hash AND m.user_id=? \
               UNION SELECT 1 FROM artist ar JOIN library_member m ON m.library_id=ar.library_id WHERE ar.artwork_hash=a.hash AND m.user_id=? \
             ) UNION ALL SELECT a.hash, a.format FROM track t JOIN library_member m ON m.library_id=t.library_id JOIN artwork a ON a.hash=t.artwork_hash WHERE t.id=? AND m.user_id=? \
             UNION ALL SELECT a.hash, a.format FROM album al JOIN library_member m ON m.library_id=al.library_id JOIN artwork a ON a.hash=al.artwork_hash WHERE al.id=? AND m.user_id=? \
             UNION ALL SELECT a.hash, a.format FROM artist ar JOIN library_member m ON m.library_id=ar.library_id JOIN artwork a ON a.hash=ar.artwork_hash WHERE ar.id=? AND m.user_id=? LIMIT 1",
        )
        .bind(id).bind(user_id.to_string()).bind(user_id.to_string()).bind(user_id.to_string())
        .bind(id).bind(user_id.to_string()).bind(id).bind(user_id.to_string()).bind(id).bind(user_id.to_string())
        .fetch_optional(self.db.pool()).await?;
        row.map(|row| Ok::<_, sqlx::Error>((row.try_get("hash")?, row.try_get("format")?)))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn playlists(&self, user_id: Uuid) -> Result<Vec<PlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlists_on(&mut connection, user_id).await
    }

    async fn playlists_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<PlaylistItem>, ServiceError> {
        let rows = sqlx::query(
            "SELECT id, name, comment, public, created_at, updated_at FROM playlist \
             WHERE owner_user_id=? ORDER BY updated_at DESC, id",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let id = parse_uuid(row.try_get("id")?)?;
            result.push(PlaylistItem {
                id,
                name: row.try_get("name")?,
                comment: row.try_get("comment")?,
                public: row.try_get::<i64, _>("public")? != 0,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
                songs: self.playlist_songs_on(connection, user_id, id).await?,
            });
        }
        Ok(result)
    }

    pub async fn playlist(&self, user_id: Uuid, id: Uuid) -> Result<PlaylistItem, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlist_on(&mut connection, user_id, id).await
    }

    async fn playlist_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<PlaylistItem, ServiceError> {
        let row = sqlx::query(
            "SELECT id, name, comment, public, created_at, updated_at FROM playlist \
             WHERE id=? AND owner_user_id=?",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(ServiceError::NotFound)?;
        Ok(PlaylistItem {
            id,
            name: row.try_get("name")?,
            comment: row.try_get("comment")?,
            public: row.try_get::<i64, _>("public")? != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            songs: self.playlist_songs_on(connection, user_id, id).await?,
        })
    }

    async fn playlist_songs_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<SongItem>, ServiceError> {
        let ids = self
            .playlist_track_ids_on(connection, user_id, playlist_id)
            .await?;
        self.songs_by_ids_lenient_on(connection, user_id, &ids)
            .await
    }

    async fn playlist_track_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<Uuid>, ServiceError> {
        sqlx::query_scalar::<_, String>(
            "SELECT pt.track_id FROM playlist_track pt JOIN playlist p ON p.id=pt.playlist_id \
             WHERE p.id=? AND p.owner_user_id=? ORDER BY pt.position",
        )
        .bind(playlist_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    pub async fn create_playlist(
        &self,
        user_id: Uuid,
        name: &str,
        track_ids: &[Uuid],
    ) -> Result<PlaylistItem, ServiceError> {
        self.create_playlist_with_context(
            user_id,
            name,
            track_ids,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn create_playlist_with_context(
        &self,
        user_id: Uuid,
        name: &str,
        track_ids: &[Uuid],
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let intent = MutationIntent::new(
            "create",
            "playlist",
            &serde_json::json!({ "name": name.trim(), "track_ids": track_ids }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            let id = receipt.result_entity_id.ok_or(ServiceError::Conflict)?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        validate_name(name)?;
        self.songs_by_ids_on(&mut tx, user_id, track_ids).await?;
        let id = Uuid::new_v4();
        let now = now_ms();
        sqlx::query("INSERT INTO playlist (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(id.to_string()).bind(user_id.to_string()).bind(name.trim()).bind(now).bind(now)
            .execute(&mut *tx).await?;
        replace_playlist_tracks(&mut tx, id, track_ids, now).await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": name.trim(),
                    "track_ids": track_ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        add: &[Uuid],
        remove_indexes: &[usize],
        clear: PlaylistClear,
    ) -> Result<PlaylistItem, ServiceError> {
        self.update_playlist_with_context(
            user_id,
            id,
            name,
            comment,
            public,
            add,
            remove_indexes,
            clear,
            MutationContext::server_generated(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        add: &[Uuid],
        remove_indexes: &[usize],
        clear: PlaylistClear,
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let mut removes = remove_indexes.to_vec();
        removes.sort_unstable_by(|a, b| b.cmp(a));
        removes.dedup();
        let mut intent_payload = serde_json::json!({
            "name": name.map(str::trim),
            "comment": comment,
            "public": public,
            "add": add,
            "remove_indexes": &removes,
            "clear_comment": clear.comment,
        });
        // Added to the payload only when set. The intent is hashed and compared
        // on replay, so naming a new field unconditionally would change the
        // hash of every update this server version ever saw before, and turn a
        // client's retry across an upgrade into a conflict.
        if clear.tracks {
            intent_payload["clear_tracks"] = serde_json::Value::Bool(true);
        }
        let intent = MutationIntent::new("update", &format!("playlist:{id}"), &intent_payload);
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        let current = self.playlist_on(&mut tx, user_id, id).await?;
        if let Some(name) = name {
            validate_name(name)?;
        }
        self.songs_by_ids_on(&mut tx, user_id, add).await?;
        let mut ids = if clear.tracks {
            Vec::new()
        } else {
            self.playlist_track_ids_on(&mut tx, user_id, id).await?
        };
        for index in removes {
            if index >= ids.len() {
                return Err(ServiceError::Invalid);
            }
            ids.remove(index);
        }
        ids.extend_from_slice(add);
        let changed_at = now_ms();
        sqlx::query(
            "UPDATE playlist SET name=COALESCE(?, name), \
             comment=CASE WHEN ? THEN NULL ELSE COALESCE(?, comment) END, \
             public=COALESCE(?, public), updated_at=? WHERE id=? AND owner_user_id=?",
        )
        .bind(name.map(str::trim))
        .bind(clear.comment)
        .bind(comment)
        .bind(public.map(i64::from))
        .bind(changed_at)
        .bind(id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM playlist_track WHERE playlist_id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        replace_playlist_tracks(&mut tx, id, &ids, changed_at).await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": name.map(str::trim).unwrap_or(&current.name),
                    "comment": comment.or(current.comment.as_deref()),
                    "public": public.unwrap_or(current.public),
                    "track_ids": ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }

    pub async fn delete_playlist(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        self.delete_playlist_with_context(user_id, id, MutationContext::server_generated())
            .await
    }

    pub async fn delete_playlist_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent =
            MutationIntent::new("delete", &format!("playlist:{id}"), &serde_json::json!({}));
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            return Ok(());
        }
        let changed = sqlx::query("DELETE FROM playlist WHERE id=? AND owner_user_id=?")
            .bind(id.to_string())
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if changed == 0 {
            tx.rollback().await?;
            Err(ServiceError::NotFound)
        } else {
            let receipt = self
                .sync
                .complete_operation(
                    &mut tx,
                    user_id,
                    context,
                    "playlist",
                    id,
                    "delete",
                    &serde_json::json!({}),
                    Some(id),
                )
                .await?;
            tx.commit().await?;
            self.sync.publish(user_id, receipt);
            Ok(())
        }
    }

    pub async fn set_star(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        starred: bool,
    ) -> Result<(), ServiceError> {
        self.set_star_with_context(
            user_id,
            entity_type,
            entity_id,
            starred,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn set_star_with_context(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        starred: bool,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            if starred { "star" } else { "unstar" },
            &format!("{entity_type}:{entity_id}"),
            &serde_json::json!({ "starred": starred }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "favorite")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, entity_type, entity_id)
            .await?;
        if starred {
            sqlx::query("INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) VALUES (?, ?, ?, ?) ON CONFLICT DO UPDATE SET starred_at=excluded.starred_at")
                .bind(user_id.to_string()).bind(entity_type).bind(entity_id.to_string()).bind(now_ms())
                .execute(&mut *tx).await?;
        } else {
            sqlx::query("DELETE FROM user_star WHERE user_id=? AND entity_type=? AND entity_id=?")
                .bind(user_id.to_string())
                .bind(entity_type)
                .bind(entity_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "favorite",
                entity_id,
                if starred { "upsert" } else { "delete" },
                &serde_json::json!({
                    "entity_type": entity_type,
                    "entity_id": entity_id,
                    "starred": starred,
                }),
                Some(entity_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn entity_kind(
        &self,
        user_id: Uuid,
        entity_id: Uuid,
    ) -> Result<Option<&'static str>, ServiceError> {
        let kinds: Vec<String> = sqlx::query_scalar(
            "SELECT entity_type FROM (\
               SELECT 'track' AS entity_type FROM track t \
                 JOIN library_member m ON m.library_id=t.library_id \
                 WHERE t.id=? AND m.user_id=? \
               UNION ALL \
               SELECT 'album' FROM album a \
                 JOIN library_member m ON m.library_id=a.library_id \
                 WHERE a.id=? AND m.user_id=? \
               UNION ALL \
               SELECT 'artist' FROM artist ar \
                 JOIN library_member m ON m.library_id=ar.library_id \
                 WHERE ar.id=? AND m.user_id=? \
             )",
        )
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        if kinds.len() != 1 {
            return Ok(None);
        }
        Ok(match kinds[0].as_str() {
            "track" => Some("track"),
            "album" => Some("album"),
            "artist" => Some("artist"),
            _ => None,
        })
    }

    pub async fn starred_ids(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<(String, Uuid, i64)>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.starred_ids_on(&mut connection, user_id).await
    }

    async fn starred_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<(String, Uuid, i64)>, ServiceError> {
        sqlx::query(
            "SELECT s.entity_type, s.entity_id, s.starred_at FROM user_star s \
             WHERE s.user_id=? AND ( \
               (s.entity_type='track' AND EXISTS (SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) OR \
               (s.entity_type='album' AND EXISTS (SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) OR \
               (s.entity_type='artist' AND EXISTS (SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) \
             ) ORDER BY s.starred_at DESC",
        )
            .bind(user_id.to_string()).fetch_all(&mut *connection).await?
            .into_iter().map(|row| Ok((row.try_get("entity_type")?, parse_uuid(row.try_get("entity_id")?)?, row.try_get("starred_at")?)))
            .collect::<Result<Vec<_>, sqlx::Error>>().map_err(Into::into)
    }

    pub async fn ratings(&self, user_id: Uuid) -> Result<Vec<RatingItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.ratings_on(&mut connection, user_id).await
    }

    async fn ratings_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<RatingItem>, ServiceError> {
        sqlx::query(
            "SELECT r.entity_type, r.entity_id, r.rating, r.updated_at FROM user_rating r \
             WHERE r.user_id=? AND ( \
               (r.entity_type='track' AND EXISTS (SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) OR \
               (r.entity_type='album' AND EXISTS (SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) OR \
               (r.entity_type='artist' AND EXISTS (SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) \
             ) ORDER BY r.updated_at DESC, r.entity_type, r.entity_id",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok(RatingItem {
                entity_type: row.try_get("entity_type")?,
                entity_id: parse_uuid(row.try_get("entity_id")?)?,
                rating: row.try_get("rating")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
    }

    pub async fn set_rating(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        rating: i64,
    ) -> Result<(), ServiceError> {
        self.set_rating_with_context(
            user_id,
            entity_type,
            entity_id,
            rating,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn set_rating_with_context(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        rating: i64,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            "set-rating",
            &format!("{entity_type}:{entity_id}"),
            &serde_json::json!({ "rating": rating }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "rating")?;
            return Ok(());
        }
        if !(0..=5).contains(&rating) {
            return Err(ServiceError::Invalid);
        }
        self.authorize_entity_on(&mut tx, user_id, entity_type, entity_id)
            .await?;
        if rating == 0 {
            sqlx::query(
                "DELETE FROM user_rating WHERE user_id=? AND entity_type=? AND entity_id=?",
            )
            .bind(user_id.to_string())
            .bind(entity_type)
            .bind(entity_id.to_string())
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query("INSERT INTO user_rating (user_id, entity_type, entity_id, rating, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO UPDATE SET rating=excluded.rating, updated_at=excluded.updated_at")
                .bind(user_id.to_string()).bind(entity_type).bind(entity_id.to_string()).bind(rating).bind(now_ms()).execute(&mut *tx).await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "rating",
                entity_id,
                if rating == 0 { "delete" } else { "upsert" },
                &serde_json::json!({
                    "entity_type": entity_type,
                    "entity_id": entity_id,
                    "rating": rating,
                }),
                Some(entity_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn scrobble(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        submission: bool,
        time: Option<i64>,
    ) -> Result<(), ServiceError> {
        self.scrobble_with_context(
            user_id,
            track_id,
            submission,
            time,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn scrobble_with_context(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        submission: bool,
        time: Option<i64>,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            if submission {
                "scrobble"
            } else {
                "now-playing"
            },
            &format!("track:{track_id}"),
            &serde_json::json!({ "submission": submission, "time": time }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "scrobble")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, "track", track_id)
            .await?;
        let current_time = now_ms();
        let now = time.unwrap_or(current_time);
        const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
        if now < 0 || now > current_time.saturating_add(MAX_FUTURE_SKEW_MS) {
            return Err(ServiceError::Invalid);
        }
        sqlx::query(
            "INSERT INTO play_event (user_id, track_id, submission, played_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .bind(i64::from(submission))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if submission {
            sqlx::query("DELETE FROM now_playing WHERE user_id=?")
                .bind(user_id.to_string())
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("INSERT INTO now_playing (user_id, track_id, started_at, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT (user_id) DO UPDATE SET track_id=excluded.track_id, started_at=excluded.started_at, updated_at=excluded.updated_at")
                .bind(user_id.to_string()).bind(track_id.to_string()).bind(now).bind(now_ms()).execute(&mut *tx).await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "scrobble",
                track_id,
                if submission { "append" } else { "upsert" },
                &serde_json::json!({
                    "track_id": track_id,
                    "submission": submission,
                    "played_at": now,
                }),
                Some(track_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn now_playing(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<(String, SongItem, i64)>, ServiceError> {
        let rows = sqlx::query(
            "SELECT a.username, n.track_id, n.started_at FROM now_playing n \
             JOIN account a ON a.id=n.user_id WHERE a.disabled=0 ORDER BY n.started_at DESC",
        )
        .fetch_all(self.db.pool())
        .await?;
        let mut result = Vec::new();
        for row in rows {
            let id = parse_uuid(row.try_get("track_id")?)?;
            match self.songs_by_ids(user_id, &[id]).await {
                Ok(mut songs) => {
                    if let Some(song) = songs.pop() {
                        result.push((row.try_get("username")?, song, row.try_get("started_at")?));
                    }
                }
                Err(ServiceError::NotFound) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }

    pub async fn history(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<HistoryItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.history_on(&mut connection, user_id, limit).await
    }

    async fn history_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<HistoryItem>, ServiceError> {
        if !(0..=MAX_HISTORY_LIMIT).contains(&limit) {
            return Err(ServiceError::Invalid);
        }
        sqlx::query(
            "SELECT p.track_id, p.submission, p.played_at FROM play_event p \
             JOIN track t ON t.id=p.track_id JOIN library_member m ON m.library_id=t.library_id \
             WHERE p.user_id=? AND m.user_id=? ORDER BY p.played_at DESC, p.id DESC LIMIT ?",
        )
        .bind(user_id.to_string())
        .bind(user_id.to_string())
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok(HistoryItem {
                track_id: parse_uuid(row.try_get("track_id")?)?,
                submission: row.try_get::<i64, _>("submission")? != 0,
                played_at: row.try_get("played_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
    }

    pub async fn save_queue(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        current: Option<Uuid>,
        position_ms: i64,
        client: Option<&str>,
    ) -> Result<(), ServiceError> {
        self.save_queue_with_context(
            user_id,
            ids,
            current,
            position_ms,
            client,
            MutationContext::server_generated(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_queue_with_context(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        current: Option<Uuid>,
        position_ms: i64,
        client: Option<&str>,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            "save",
            &format!("queue:{user_id}"),
            &serde_json::json!({
                "track_ids": ids,
                "current": current,
                "position_ms": position_ms,
                "client": client,
            }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "queue")?;
            return Ok(());
        }
        if ids.len() > MAX_QUEUE_TRACKS {
            return Err(ServiceError::Invalid);
        }
        if position_ms < 0 {
            return Err(ServiceError::Invalid);
        }
        self.songs_by_ids_on(&mut tx, user_id, ids).await?;
        if let Some(current) = current {
            self.songs_by_ids_on(&mut tx, user_id, &[current]).await?;
        }
        sqlx::query("INSERT INTO play_queue (user_id, current_track_id, position_ms, changed_by, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (user_id) DO UPDATE SET current_track_id=excluded.current_track_id, position_ms=excluded.position_ms, changed_by=excluded.changed_by, updated_at=excluded.updated_at")
            .bind(user_id.to_string()).bind(current.map(|id| id.to_string())).bind(position_ms).bind(client).bind(now_ms()).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM play_queue_track WHERE user_id=?")
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        if !ids.is_empty() {
            let ids_json = serde_json::to_string(ids).map_err(|_| ServiceError::Invalid)?;
            sqlx::query(
                "INSERT INTO play_queue_track (user_id, track_id, position) \
                 SELECT ?, value, CAST(key AS INTEGER) FROM json_each(?)",
            )
            .bind(user_id.to_string())
            .bind(ids_json)
            .execute(&mut *tx)
            .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "queue",
                user_id,
                "upsert",
                &serde_json::json!({
                    "track_ids": ids,
                    "current": current,
                    "position_ms": position_ms,
                    "client": client,
                }),
                Some(user_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn queue(&self, user_id: Uuid) -> Result<Option<QueueItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.queue_on(&mut connection, user_id).await
    }

    async fn queue_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Option<QueueItem>, ServiceError> {
        let row = sqlx::query("SELECT current_track_id, position_ms, changed_by, updated_at FROM play_queue WHERE user_id=?")
            .bind(user_id.to_string()).fetch_optional(&mut *connection).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT track_id FROM play_queue_track WHERE user_id=? ORDER BY position",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(QueueItem {
            current: row
                .try_get::<Option<String>, _>("current_track_id")?
                .map(parse_uuid)
                .transpose()?,
            position_ms: row.try_get("position_ms")?,
            changed_by: row.try_get("changed_by")?,
            updated_at: row.try_get("updated_at")?,
            songs: self
                .songs_by_ids_lenient_on(connection, user_id, &ids)
                .await?,
        }))
    }

    /// Bookmarks the user has set, most recently changed first.
    pub async fn bookmarks(&self, user_id: Uuid) -> Result<Vec<BookmarkItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.bookmarks_on(&mut connection, user_id).await
    }

    async fn bookmarks_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<BookmarkItem>, ServiceError> {
        // Joined against `song_select!` so a bookmark on a track that has become
        // unavailable, or on a library the account has lost, simply stops being
        // listed rather than being returned pointing at nothing.
        let rows = sqlx::query(concat!(
            "SELECT b.position_ms, b.comment AS bookmark_comment, \
                    b.created_at AS bookmark_created_at, b.updated_at AS bookmark_updated_at, \
                    song.* FROM bookmark b JOIN (",
            song_select!(),
            ") AS song ON song.id=b.track_id \
             WHERE b.user_id=? ORDER BY b.updated_at DESC, song.id"
        ))
        .bind(user_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut bookmarks = Vec::with_capacity(rows.len());
        for row in rows {
            bookmarks.push(BookmarkItem {
                position_ms: row.try_get("position_ms")?,
                comment: row.try_get("bookmark_comment")?,
                created_at: row.try_get("bookmark_created_at")?,
                updated_at: row.try_get("bookmark_updated_at")?,
                song: song_from_row(row)?,
            });
        }
        let mut songs = bookmarks
            .iter()
            .map(|bookmark| bookmark.song.clone())
            .collect::<Vec<_>>();
        attach_song_relations(&mut *connection, user_id, &mut songs).await?;
        for (bookmark, song) in bookmarks.iter_mut().zip(songs) {
            bookmark.song = song;
        }
        Ok(bookmarks)
    }

    pub async fn set_bookmark(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        position_ms: i64,
        comment: Option<&str>,
    ) -> Result<(), ServiceError> {
        self.set_bookmark_with_context(
            user_id,
            track_id,
            position_ms,
            comment,
            MutationContext::server_generated(),
        )
        .await
    }

    /// Sets, or moves, the bookmark on one track.
    ///
    /// A bookmark answers "where did I stop in this file", so there is one per
    /// account and track and a second call moves it rather than adding another.
    pub async fn set_bookmark_with_context(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        position_ms: i64,
        comment: Option<&str>,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        if position_ms < 0 {
            return Err(ServiceError::Invalid);
        }
        let intent = MutationIntent::new(
            "set-bookmark",
            &format!("bookmark:{track_id}"),
            &serde_json::json!({ "position_ms": position_ms, "comment": comment }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "bookmark")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, "track", track_id)
            .await?;
        let now = now_ms();
        sqlx::query(
            "INSERT INTO bookmark (user_id, track_id, position_ms, comment, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (user_id, track_id) DO UPDATE SET position_ms=excluded.position_ms, \
               comment=excluded.comment, updated_at=excluded.updated_at",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .bind(position_ms)
        .bind(comment)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "bookmark",
                track_id,
                "upsert",
                &serde_json::json!({
                    "track_id": track_id,
                    "position_ms": position_ms,
                    "comment": comment,
                }),
                Some(track_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn delete_bookmark(&self, user_id: Uuid, track_id: Uuid) -> Result<(), ServiceError> {
        self.delete_bookmark_with_context(user_id, track_id, MutationContext::server_generated())
            .await
    }

    /// Removes the bookmark on one track.
    ///
    /// Removing one that is not there succeeds: the caller asked for the track
    /// to carry no bookmark, and it does not. Reporting not-found would also
    /// answer a question about another account's catalogue, which the rest of
    /// the surface refuses to do.
    pub async fn delete_bookmark_with_context(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            "delete-bookmark",
            &format!("bookmark:{track_id}"),
            &serde_json::json!({}),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "bookmark")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, "track", track_id)
            .await?;
        sqlx::query("DELETE FROM bookmark WHERE user_id=? AND track_id=?")
            .bind(user_id.to_string())
            .bind(track_id.to_string())
            .execute(&mut *tx)
            .await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "bookmark",
                track_id,
                "delete",
                &serde_json::json!({ "track_id": track_id }),
                Some(track_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn shares(&self, user_id: Uuid) -> Result<Vec<ShareItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.shares_on(&mut connection, user_id).await
    }

    async fn shares_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<ShareItem>, ServiceError> {
        let rows = sqlx::query("SELECT id, description, expires_at, created_at, visit_count FROM share WHERE owner_user_id=? ORDER BY created_at DESC")
            .bind(user_id.to_string()).fetch_all(&mut *connection).await?;
        let track_rows = sqlx::query(
            "SELECT st.share_id, st.track_id FROM share_track st \
             JOIN share s ON s.id=st.share_id WHERE s.owner_user_id=? \
             ORDER BY st.share_id, st.position",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut track_owners = Vec::with_capacity(track_rows.len());
        let mut track_ids = Vec::with_capacity(track_rows.len());
        for track_row in track_rows {
            track_owners.push(parse_uuid(track_row.try_get("share_id")?)?);
            track_ids.push(parse_uuid(track_row.try_get("track_id")?)?);
        }
        let songs = self
            .songs_by_ids_lenient_on(connection, user_id, &track_ids)
            .await?
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        let mut songs_by_share = HashMap::<Uuid, Vec<SongItem>>::new();
        for (share_id, track_id) in track_owners.into_iter().zip(track_ids) {
            if let Some(song) = songs.get(&track_id) {
                songs_by_share
                    .entry(share_id)
                    .or_default()
                    .push(song.clone());
            }
        }

        let mut shares = Vec::with_capacity(rows.len());
        for row in rows {
            let id = parse_uuid(row.try_get("id")?)?;
            shares.push(ShareItem {
                id,
                owner_id: user_id,
                url_token: None,
                description: row.try_get("description")?,
                expires_at: row.try_get("expires_at")?,
                created_at: row.try_get("created_at")?,
                visit_count: row.try_get("visit_count")?,
                songs: songs_by_share.remove(&id).unwrap_or_default(),
            });
        }
        Ok(shares)
    }

    pub async fn create_share(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        description: Option<&str>,
        expires_at: Option<i64>,
    ) -> Result<ShareItem, ServiceError> {
        self.create_share_with_context(
            user_id,
            ids,
            description,
            expires_at,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn create_share_with_context(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        description: Option<&str>,
        expires_at: Option<i64>,
        context: MutationContext,
    ) -> Result<ShareItem, ServiceError> {
        let intent = MutationIntent::new(
            "create",
            "share",
            &serde_json::json!({
                "track_ids": ids,
                "description": description,
                "expires_at": expires_at,
            }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "share")?;
            let id = receipt.result_entity_id.ok_or(ServiceError::Conflict)?;
            drop(_writer);
            let mut share = self
                .shares(user_id)
                .await?
                .into_iter()
                .find(|share| share.id == id)
                .ok_or(ServiceError::NotFound)?;
            share.url_token = Some(self.secret_box.derive_share_token(id));
            return Ok(share);
        }
        if ids.is_empty() || ids.len() > MAX_SHARE_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let songs = self.songs_by_ids_on(&mut tx, user_id, ids).await?;
        let id = Uuid::new_v4();
        let token = self.secret_box.derive_share_token(id);
        let token_hash = security::token_hash(&token);
        let now = now_ms();
        sqlx::query("INSERT INTO share (id, owner_user_id, token_hash, description, expires_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(id.to_string()).bind(user_id.to_string()).bind(token_hash.as_slice()).bind(description).bind(expires_at).bind(now).bind(now).execute(&mut *tx).await?;
        for (position, track) in ids.iter().enumerate() {
            sqlx::query("INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, ?)")
                .bind(id.to_string())
                .bind(track.to_string())
                .bind(position as i64)
                .execute(&mut *tx)
                .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "share",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "track_ids": ids,
                    "description": description,
                    "expires_at": expires_at,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        Ok(ShareItem {
            id,
            owner_id: user_id,
            url_token: Some(token),
            description: description.map(str::to_owned),
            expires_at,
            created_at: now,
            visit_count: 0,
            songs,
        })
    }

    pub async fn public_share(&self, token: &str) -> Result<ShareItem, ServiceError> {
        let hash = security::token_hash(token);
        let row = sqlx::query("SELECT id, owner_user_id, description, expires_at, created_at, visit_count FROM share WHERE token_hash=? AND (expires_at IS NULL OR expires_at>?)")
            .bind(hash.as_slice()).bind(now_ms()).fetch_optional(self.db.pool()).await?.ok_or(ServiceError::NotFound)?;
        let id = parse_uuid(row.try_get("id")?)?;
        let owner = parse_uuid(row.try_get("owner_user_id")?)?;
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT track_id FROM share_track WHERE share_id=? ORDER BY position",
        )
        .bind(id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
        // The rows above were read outside the writer gate, and acquiring it can
        // block behind a scan. Re-check both revocation and expiry at write
        // time: no affected row means the share died during that wait, and a
        // visitor must not see what an owner deleted or let expire.
        let _writer = self.db.writer_guard().await;
        let visited_at = now_ms();
        let visited = sqlx::query(
            "UPDATE share SET visit_count=visit_count+1, last_visited_at=? \
             WHERE id=? AND (expires_at IS NULL OR expires_at>?)",
        )
        .bind(visited_at)
        .bind(id.to_string())
        .bind(visited_at)
        .execute(self.db.pool())
        .await?
        .rows_affected();
        drop(_writer);
        if visited == 0 {
            return Err(ServiceError::NotFound);
        }
        Ok(ShareItem {
            id,
            owner_id: owner,
            url_token: None,
            description: row.try_get("description")?,
            expires_at: row.try_get("expires_at")?,
            created_at: row.try_get("created_at")?,
            visit_count: row.try_get::<i64, _>("visit_count")? + 1,
            songs: self.songs_by_ids(owner, &ids).await?,
        })
    }

    pub async fn update_share(
        &self,
        user_id: Uuid,
        id: Uuid,
        description: Option<&str>,
        expires_at: Option<i64>,
        clear: ShareClear,
    ) -> Result<ShareItem, ServiceError> {
        self.update_share_with_context(
            user_id,
            id,
            description,
            expires_at,
            clear,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn update_share_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        description: Option<&str>,
        expires_at: Option<i64>,
        clear: ShareClear,
        context: MutationContext,
    ) -> Result<ShareItem, ServiceError> {
        // Clearing must be part of the intent: "set expiry to X" and "remove the
        // expiry" are different mutations, and an operation id replayed across
        // both has to be rejected rather than silently treated as the same.
        let intent = MutationIntent::new(
            "update",
            &format!("share:{id}"),
            &serde_json::json!({
                "description": description,
                "expires_at": expires_at,
                "clear_description": clear.description,
                "clear_expires_at": clear.expires_at,
            }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "share")?;
            drop(_writer);
            return self
                .shares(user_id)
                .await?
                .into_iter()
                .find(|share| share.id == id)
                .ok_or(ServiceError::NotFound);
        }
        let persisted = sqlx::query(
            "UPDATE share SET \
               description=CASE WHEN ? THEN NULL ELSE COALESCE(?, description) END, \
               expires_at=CASE WHEN ? THEN NULL ELSE COALESCE(?, expires_at) END, \
               updated_at=? \
             WHERE id=? AND owner_user_id=? RETURNING description, expires_at",
        )
        .bind(clear.description)
        .bind(description)
        .bind(clear.expires_at)
        .bind(expires_at)
        .bind(now_ms())
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(persisted) = persisted else {
            tx.rollback().await?;
            return Err(ServiceError::NotFound);
        };
        let persisted_description: Option<String> = persisted.try_get("description")?;
        let persisted_expires_at: Option<i64> = persisted.try_get("expires_at")?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "share",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "description": persisted_description,
                    "expires_at": persisted_expires_at,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.shares(user_id)
            .await?
            .into_iter()
            .find(|share| share.id == id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn delete_share(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        self.delete_share_with_context(user_id, id, MutationContext::server_generated())
            .await
    }

    pub async fn delete_share_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new("delete", &format!("share:{id}"), &serde_json::json!({}));
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "share")?;
            return Ok(());
        }
        let changed = sqlx::query("DELETE FROM share WHERE id=? AND owner_user_id=?")
            .bind(id.to_string())
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if changed == 0 {
            tx.rollback().await?;
            Err(ServiceError::NotFound)
        } else {
            let receipt = self
                .sync
                .complete_operation(
                    &mut tx,
                    user_id,
                    context,
                    "share",
                    id,
                    "delete",
                    &serde_json::json!({}),
                    Some(id),
                )
                .await?;
            tx.commit().await?;
            self.sync.publish(user_id, receipt);
            Ok(())
        }
    }

    pub async fn users(&self, actor_id: Uuid) -> Result<Vec<UserItem>, ServiceError> {
        self.require_admin(actor_id).await?;
        let mut users = sqlx::query("SELECT a.id, a.username, a.role, a.disabled, c.user_id IS NOT NULL AS has_credential FROM account a LEFT JOIN subsonic_credential c ON c.user_id=a.id ORDER BY a.username COLLATE NOCASE")
            .fetch_all(self.db.pool()).await?.into_iter().map(|row| Ok(UserItem { id: parse_uuid(row.try_get("id")?)?, username: row.try_get("username")?, role: AccountRole::from_str(row.try_get::<&str, _>("role")?).map_err(|error| sqlx::Error::Decode(error.into()))?, disabled: row.try_get::<i64, _>("disabled")? != 0, has_subsonic_credential: row.try_get::<i64, _>("has_credential")? != 0, folder_ids: Vec::new() })).collect::<Result<Vec<_>, sqlx::Error>>()?;
        let memberships = sqlx::query(
            "SELECT user_id, library_id FROM library_member ORDER BY user_id, library_id",
        )
        .fetch_all(self.db.pool())
        .await?;
        for row in memberships {
            let user_id = parse_uuid(row.try_get("user_id")?)?;
            let library_id = parse_uuid(row.try_get("library_id")?)?;
            if let Some(user) = users.iter_mut().find(|user| user.id == user_id) {
                user.folder_ids.push(library_id);
            }
        }
        Ok(users)
    }

    pub async fn create_web_user(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
        role: AccountRole,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        validate_username(username)?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || security::hash_password(&password))
            .await
            .map_err(|_| ServiceError::Unavailable)??;
        let id = self
            .db
            .create_account(username.trim(), &password_hash, role, now_ms())
            .await
            .map_err(|error| {
                if matches!(error, sqlx::Error::Database(ref db) if db.is_unique_violation()) {
                    ServiceError::Conflict
                } else {
                    ServiceError::Database(error)
                }
            })?;
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == id)
            .ok_or(ServiceError::NotFound)
    }

    /// Sets a dedicated Subsonic password and rotates the API key. The clear
    /// API key is returned once; only its hash is persisted.
    pub async fn set_subsonic_credential(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
    ) -> Result<String, ServiceError> {
        self.require_admin(actor_id).await?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let api_key = security::generate_token("wfsk_");
        let api_key_hash = security::token_hash(&api_key);
        self.db
            .set_subsonic_credential(actor_id, account.id, &encrypted, &api_key_hash, now_ms())
            .await?;
        Ok(api_key)
    }

    pub async fn revoke_subsonic_credential(
        &self,
        actor_id: Uuid,
        username: &str,
    ) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if self
            .db
            .revoke_subsonic_credential(actor_id, account.id, now_ms())
            .await?
        {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }

    /// The API tokens issued to one account.
    ///
    /// Administrative like the Subsonic credential routes beside it: a token
    /// carries the authority of the account it belongs to, so who may mint one
    /// is a question about the instance, not about the account itself.
    pub async fn api_tokens(
        &self,
        actor_id: Uuid,
        username: &str,
    ) -> Result<Vec<ApiTokenRecord>, ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        Ok(self.db.api_tokens_for_user(account.id).await?)
    }

    /// Issues a token and returns it beside its record.
    ///
    /// The secret is returned once and stored only as a SHA-256 hash, exactly
    /// as `set_subsonic_credential` returns its API key: a caller that loses it
    /// issues another one rather than reading it back.
    pub async fn create_api_token(
        &self,
        actor_id: Uuid,
        username: &str,
        name: &str,
        scopes: &[String],
    ) -> Result<(ApiTokenRecord, String), ServiceError> {
        self.require_admin(actor_id).await?;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(ServiceError::Invalid);
        }
        // Normalised on the way in, so the value a listing shows is the value
        // authorization compares. Trimming at the check instead would let a
        // stored `" admin "` grant what a reader of the listing would not
        // expect it to.
        let scopes = scopes
            .iter()
            .map(|scope| scope.trim().to_owned())
            .collect::<Vec<_>>();
        if scopes.iter().any(String::is_empty) {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let token = security::generate_token("wfapi_");
        let record = self
            .db
            .create_api_token(
                account.id,
                name,
                &security::token_hash(&token),
                &scopes,
                now_ms(),
            )
            .await?;
        Ok((record, token))
    }

    /// Revokes one token of one account.
    ///
    /// A token that is not this account's, or is already revoked, answers as a
    /// missing one: the caller asked for it to stop working, and naming the
    /// wrong owner must not confirm that it exists elsewhere.
    pub async fn revoke_api_token(
        &self,
        actor_id: Uuid,
        username: &str,
        token_id: Uuid,
    ) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if self
            .db
            .revoke_api_token(actor_id, account.id, token_id, now_ms())
            .await?
        {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }

    pub async fn create_subsonic_user(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
        admin: bool,
        folder_ids: Option<&[Uuid]>,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        validate_name(username)?;
        if password.is_empty() {
            return Err(ServiceError::Invalid);
        }
        let placeholder = security::generate_token("web-disabled-");
        let password_hash =
            tokio::task::spawn_blocking(move || security::hash_password(&placeholder))
                .await
                .map_err(|_| ServiceError::Unavailable)??;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let api_key = security::generate_token("wfsk_");
        let api_key_hash = security::token_hash(&api_key);
        let requested_folders = self.resolve_library_ids(folder_ids).await?;
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let user_id = Uuid::new_v4();
        let now = now_ms();
        let insert = sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(username.trim())
        .bind(password_hash)
        .bind(if admin { "admin" } else { "user" })
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await;
        if let Err(error) = insert {
            return Err(
                if matches!(error, sqlx::Error::Database(ref db) if db.is_unique_violation()) {
                    ServiceError::Conflict
                } else {
                    ServiceError::Database(error)
                },
            );
        }
        sqlx::query(
            "INSERT INTO subsonic_credential \
             (user_id, password_nonce, password_ciphertext, api_key_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(encrypted.nonce.as_slice())
        .bind(encrypted.ciphertext)
        .bind(api_key_hash.as_slice())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        for library_id in requested_folders {
            sqlx::query(
                "INSERT INTO library_member (library_id, user_id, role, created_at) \
                 VALUES (?, ?, 'listener', ?)",
            )
            .bind(library_id.to_string())
            .bind(user_id.to_string())
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO audit_event (actor_user_id, kind, subject_id, occurred_at) \
             VALUES (?, 'subsonic.user_created', ?, ?)",
        )
        .bind(actor_id.to_string())
        .bind(user_id.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        drop(_writer);
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == user_id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn update_user(
        &self,
        actor_id: Uuid,
        username: &str,
        update: UserUpdate<'_>,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        if update.subsonic_password.is_some_and(str::is_empty)
            || update
                .web_password
                .is_some_and(|password| password.len() < 12)
        {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if account.id == actor_id && (update.admin == Some(false) || update.disabled == Some(true))
        {
            return Err(ServiceError::Forbidden);
        }
        let requested_folders = match update.folder_ids {
            Some(ids) => Some(self.resolve_library_ids(Some(ids)).await?),
            None => None,
        };
        let encrypted = update
            .subsonic_password
            .map(|password| self.secret_box.encrypt(password.as_bytes()))
            .transpose()?;
        let web_password_hash = if let Some(password) = update.web_password {
            let password = password.to_owned();
            Some(
                tokio::task::spawn_blocking(move || security::hash_password(&password))
                    .await
                    .map_err(|_| ServiceError::Unavailable)??,
            )
        } else {
            None
        };
        let revoke_sessions = web_password_hash.is_some();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        sqlx::query("UPDATE account SET role=COALESCE(?, role), disabled=COALESCE(?, disabled), password_hash=COALESCE(?, password_hash), updated_at=? WHERE id=?")
            .bind(update.admin.map(|value| if value { "admin" } else { "user" })).bind(update.disabled.map(i64::from)).bind(web_password_hash.as_deref()).bind(now_ms()).bind(account.id.to_string()).execute(&mut *tx).await?;
        if revoke_sessions {
            sqlx::query("UPDATE session SET revoked_at=? WHERE user_id=? AND revoked_at IS NULL")
                .bind(now_ms())
                .bind(account.id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(encrypted) = encrypted {
            let changed = sqlx::query(
                "UPDATE subsonic_credential SET password_nonce=?, password_ciphertext=?, updated_at=? WHERE user_id=?",
            )
            .bind(encrypted.nonce.as_slice())
            .bind(encrypted.ciphertext)
            .bind(now_ms())
            .bind(account.id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if changed == 0 {
                return Err(ServiceError::NotFound);
            }
        }
        if let Some(folder_ids) = requested_folders {
            sqlx::query("DELETE FROM library_member WHERE user_id=? AND role='listener'")
                .bind(account.id.to_string())
                .execute(&mut *tx)
                .await?;
            for library_id in folder_ids {
                sqlx::query(
                    "INSERT INTO library_member (library_id, user_id, role, created_at) \
                     VALUES (?, ?, 'listener', ?) \
                     ON CONFLICT (library_id, user_id) DO NOTHING",
                )
                .bind(library_id.to_string())
                .bind(account.id.to_string())
                .bind(now_ms())
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        drop(_writer);
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == account.id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn delete_user(&self, actor_id: Uuid, username: &str) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if account.id == actor_id {
            return Err(ServiceError::Forbidden);
        }
        let _writer = self.db.writer_guard().await;
        sqlx::query("DELETE FROM account WHERE id=?")
            .bind(account.id.to_string())
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    pub async fn change_subsonic_password(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
    ) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        if password.is_empty() {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let _writer = self.db.writer_guard().await;
        let changed = sqlx::query("UPDATE subsonic_credential SET password_nonce=?, password_ciphertext=?, updated_at=? WHERE user_id=?")
            .bind(encrypted.nonce.as_slice()).bind(encrypted.ciphertext).bind(now_ms()).bind(account.id.to_string()).execute(self.db.pool()).await?.rows_affected();
        if changed == 0 {
            Err(ServiceError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn require_admin(&self, actor_id: Uuid) -> Result<(), ServiceError> {
        let account = self
            .db
            .account_by_id(actor_id)
            .await?
            .ok_or(ServiceError::Forbidden)?;
        if account.role == AccountRole::Admin && !account.disabled {
            Ok(())
        } else {
            Err(ServiceError::Forbidden)
        }
    }

    async fn resolve_library_ids(
        &self,
        requested: Option<&[Uuid]>,
    ) -> Result<Vec<Uuid>, ServiceError> {
        let available = sqlx::query_scalar::<_, String>("SELECT id FROM library ORDER BY id")
            .fetch_all(self.db.pool())
            .await?
            .into_iter()
            .map(parse_uuid)
            .collect::<Result<Vec<_>, _>>()?;
        let Some(requested) = requested else {
            return Ok(available);
        };
        let mut unique = Vec::new();
        for id in requested {
            if !available.contains(id) {
                return Err(ServiceError::NotFound);
            }
            if !unique.contains(id) {
                unique.push(*id);
            }
        }
        Ok(unique)
    }

    async fn authorize_entity_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        kind: &str,
        id: Uuid,
    ) -> Result<(), ServiceError> {
        let query = match kind {
            "track" => "SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=?",
            "album" => "SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=?",
            "artist" => "SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=?",
            _ => return Err(ServiceError::Invalid),
        };
        let exists = sqlx::query_scalar::<_, i64>(query)
            .bind(id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(&mut *connection)
            .await?;
        if exists.is_some() {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }
}

/// Fills in the relations a single projected row cannot carry.
///
/// `song_select!` collapses credited artists and genres into the display strings
/// the tags happened to contain; the structured form lives in `track_artist` and
/// `track_genre`, ordered and deduplicated by the scanner. Reading them per song
/// would be two queries per row on every listing, so both are fetched once for
/// the whole batch and distributed by track id.
///
/// Tenancy is re-checked here rather than inherited from the caller: the batch
/// is keyed by track id alone, and a join that trusted those ids would be the
/// one place in the read path where membership is not proven.
/// The JSON library list a scoped projection binds, or `None` for "every
/// library the account can reach". Built once here because every scoped
/// query binds the same value twice and a second spelling of it would be a
/// second chance to get the empty case wrong.
fn folder_filter(library_ids: &[Uuid]) -> Option<String> {
    (!library_ids.is_empty())
        .then(|| serde_json::to_string(library_ids).expect("UUID list serialization cannot fail"))
}

/// Fills in the album relations OpenSubsonic expects on `AlbumID3`.
///
/// Both are derived from the album's own available tracks rather than stored:
/// an album has no genre or credit of its own in the schema, it has the union
/// of what its files carry. Loaded in one batch per relation like the song
/// relations, because an album listing is up to five hundred rows and a query
/// each would be a query per row.
///
/// Tenancy is re-checked in the query. The batch is keyed by album id alone, so
/// the `library_member` join is what stops an id from another account resolving
/// to real names.
async fn attach_album_relations(
    connection: &mut SqliteConnection,
    user_id: Uuid,
    albums: &mut [AlbumItem],
) -> Result<(), sqlx::Error> {
    if albums.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(&albums.iter().map(|album| album.id).collect::<Vec<_>>())
        .expect("UUID list serialization cannot fail");
    let mut artists: HashMap<Uuid, Vec<ArtistRef>> = HashMap::new();
    for row in sqlx::query(
        // The album's own credits, which are its album artists — not the union
        // of its tracks' credits, which is what this used to answer. An album
        // with a guest on one track was reporting the guest as one of its
        // artists; the reference reports the two the album is credited to, and
        // leaves the guest to the track that names them.
        "SELECT ap.album_id, ar.id, ar.name \
         FROM album_participant ap \
         JOIN artist ar ON ar.id=ap.artist_id \
         JOIN library_member m ON m.library_id=ap.library_id \
         WHERE m.user_id=? AND ap.role='albumartist' \
           AND ap.album_id IN (SELECT value FROM json_each(?)) \
         ORDER BY ap.album_id, ap.position, ar.name COLLATE NOCASE, ar.id",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        artists
            .entry(parse_uuid(row.try_get("album_id")?)?)
            .or_default()
            .push(ArtistRef {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            });
    }
    let mut genres: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in sqlx::query(
        // Grouped on the canonical name for the same reason `list_genres` is:
        // otherwise one album spelling "Hip-Hop" on some tracks and "Hip Hop"
        // on others reports two genres.
        "SELECT t.album_id, MIN(g.name) AS name FROM track t \
         JOIN track_genre tg ON tg.track_id=t.id \
         JOIN genre g ON g.id=tg.genre_id \
         JOIN library_member m ON m.library_id=t.library_id \
         WHERE m.user_id=? AND t.is_available=1 \
           AND t.album_id IN (SELECT value FROM json_each(?)) \
         GROUP BY t.album_id, g.canonical_name \
         ORDER BY t.album_id, name COLLATE NOCASE",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        genres
            .entry(parse_uuid(row.try_get("album_id")?)?)
            .or_default()
            .push(row.try_get("name")?);
    }
    for album in albums {
        album.artists = artists.remove(&album.id).unwrap_or_default();
        album.genres = genres.remove(&album.id).unwrap_or_default();
    }
    Ok(())
}

async fn attach_song_relations(
    connection: &mut SqliteConnection,
    user_id: Uuid,
    songs: &mut [SongItem],
) -> Result<(), sqlx::Error> {
    if songs.is_empty() {
        return Ok(());
    }
    let ids = serde_json::to_string(&songs.iter().map(|song| song.id).collect::<Vec<_>>())
        .expect("UUID list serialization cannot fail");
    let mut artists: HashMap<Uuid, Vec<ArtistRef>> = HashMap::new();
    for row in sqlx::query(
        "SELECT tp.track_id, ar.id, ar.name FROM track_participant tp \
         JOIN artist ar ON ar.id=tp.artist_id \
         JOIN library_member m ON m.library_id=tp.library_id \
         WHERE m.user_id=? AND tp.role='artist' \
           AND tp.track_id IN (SELECT value FROM json_each(?)) \
         ORDER BY tp.track_id, tp.position",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        artists
            .entry(parse_uuid(row.try_get("track_id")?)?)
            .or_default()
            .push(ArtistRef {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            });
    }
    let mut genres: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in sqlx::query(
        // `track_genre` has no position column, so the order is the genre name.
        // It has to be deterministic: a client diffing two responses would
        // otherwise see a change that is not one.
        "SELECT tg.track_id, g.name FROM track_genre tg \
         JOIN genre g ON g.id=tg.genre_id \
         JOIN library_member m ON m.library_id=tg.library_id \
         WHERE m.user_id=? AND tg.track_id IN (SELECT value FROM json_each(?)) \
         ORDER BY tg.track_id, g.name COLLATE NOCASE, g.id",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        genres
            .entry(parse_uuid(row.try_get("track_id")?)?)
            .or_default()
            .push(row.try_get("name")?);
    }
    for song in songs {
        song.artists = artists.remove(&song.id).unwrap_or_default();
        song.genres = genres.remove(&song.id).unwrap_or_default();
    }
    Ok(())
}

async fn fetch_songs(
    db: &Database,
    user_id: Uuid,
    folder_filter: Option<&str>,
    id: Option<Uuid>,
) -> Result<Vec<SongItem>, sqlx::Error> {
    let id = id.map(|id| id.to_string());
    let mut songs = sqlx::query(concat!(
        song_select!(),
        " AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
           AND (? IS NULL OR t.id=?) ORDER BY t.title COLLATE NOCASE"
    ))
    .bind(user_id.to_string())
    .bind(folder_filter)
    .bind(folder_filter)
    .bind(id.as_deref())
    .bind(id.as_deref())
    .fetch_all(db.pool())
    .await?
    .into_iter()
    .map(song_from_row)
    .collect::<Result<Vec<_>, _>>()?;
    attach_song_relations(&mut *db.pool().acquire().await?, user_id, &mut songs).await?;
    Ok(songs)
}

fn credential_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<SubsonicCredentialRecord, sqlx::Error> {
    let nonce = row.try_get::<Vec<u8>, _>("password_nonce")?;
    let nonce: [u8; 12] = nonce.try_into().map_err(|value: Vec<u8>| {
        sqlx::Error::Decode(format!("invalid credential nonce length: {}", value.len()).into())
    })?;
    Ok(SubsonicCredentialRecord {
        account: AccountRecord {
            id: parse_uuid(row.try_get("id")?)?,
            username: row.try_get("username")?,
            password_hash: row.try_get("password_hash")?,
            role: AccountRole::from_str(row.try_get::<&str, _>("role")?)
                .map_err(|error| sqlx::Error::Decode(error.into()))?,
            disabled: row.try_get::<i64, _>("disabled")? != 0,
        },
        encrypted_password: EncryptedSecret {
            nonce,
            ciphertext: row.try_get("password_ciphertext")?,
        },
    })
}

fn artist_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ArtistItem, sqlx::Error> {
    Ok(ArtistItem {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        name: row.try_get("name")?,
        artwork_hash: row.try_get("artwork_hash")?,
        musicbrainz_id: row.try_get("musicbrainz_id")?,
        sort_name: row.try_get("sort_name")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
    })
}

fn artist_summary_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ArtistSummary, sqlx::Error> {
    let album_count = row.try_get("album_count")?;
    Ok(ArtistSummary {
        artist: artist_from_row(row)?,
        album_count,
    })
}

fn album_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AlbumItem, sqlx::Error> {
    Ok(AlbumItem {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        title: row.try_get("title")?,
        artist: row.try_get("album_artist_name")?,
        artist_id: row
            .try_get::<Option<String>, _>("album_artist_id")?
            .map(parse_uuid)
            .transpose()?,
        artwork_hash: row.try_get("artwork_hash")?,
        year: row.try_get("year")?,
        is_compilation: row.try_get::<i64, _>("is_compilation")? != 0,
        sort_name: row.try_get("sort_name")?,
        musicbrainz_id: row.try_get("musicbrainz_id")?,
        // Loaded in a batch by `attach_album_relations`, never row by row.
        artists: Vec::new(),
        genres: Vec::new(),
        created_at: row.try_get("created_at")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
        play_count: row.try_get("play_count")?,
        last_played_at: row.try_get("last_played_at")?,
        song_count: row.try_get("song_count")?,
        duration_ms: row.try_get("duration_ms")?,
    })
}

fn lyrics_list_from_rows(
    track_id: Uuid,
    rows: Vec<sqlx::sqlite::SqliteRow>,
) -> Result<LyricsList, ServiceError> {
    let first = rows.first().ok_or(ServiceError::NotFound)?;
    let display_title: String = first.try_get("title")?;
    let display_artist: Option<String> = first.try_get("artist_display")?;
    let mut structured_lyrics = Vec::new();
    for row in rows {
        let Some(content) = row.try_get::<Option<String>, _>("content")? else {
            continue;
        };
        let synced = row.try_get::<Option<i64>, _>("synced")?.unwrap_or(0) != 0;
        structured_lyrics.push(StructuredLyrics {
            display_artist: display_artist.clone(),
            display_title: display_title.clone(),
            lang: row
                .try_get::<Option<String>, _>("lang")?
                .unwrap_or_else(|| "xxx".into()),
            synced,
            lines: lyrics::lines(&content, synced),
        });
    }
    Ok(LyricsList {
        track_id,
        structured_lyrics,
    })
}

/// Splits a multi-valued tag string the way the scanner stored it.
///
/// The scanner writes these joined on `;`, so a reader that did not split
/// them would hand a client one value that is really several.
fn split_tag_values(raw: Option<&str>) -> Vec<String> {
    raw.into_iter()
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn song_from_row(row: sqlx::sqlite::SqliteRow) -> Result<SongItem, sqlx::Error> {
    let relative: String = row.try_get("relative_path")?;
    Ok(SongItem {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        album_id: row
            .try_get::<Option<String>, _>("album_id")?
            .map(parse_uuid)
            .transpose()?,
        title: row.try_get("title")?,
        album: row.try_get("album_title")?,
        artist: row.try_get("artist_display")?,
        artist_id: row
            .try_get::<Option<String>, _>("artist_id")?
            .map(parse_uuid)
            .transpose()?,
        genre: row.try_get("genre_display")?,
        year: row.try_get("year")?,
        track: row.try_get("track_number")?,
        disc: row.try_get("disc_number")?,
        duration_ms: row.try_get("duration_ms")?,
        bitrate: row.try_get("bitrate")?,
        codec: row.try_get("codec")?,
        suffix: PathBuf::from(relative)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase(),
        size: row.try_get("file_size")?,
        artwork_hash: row.try_get("artwork_hash")?,
        full_hash: row.try_get("full_hash")?,
        created_at: row.try_get("created_at")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
        sample_rate: row.try_get("sample_rate")?,
        channels: row.try_get("channels")?,
        bit_depth: row.try_get("bit_depth")?,
        play_count: row.try_get("play_count")?,
        last_played_at: row.try_get("last_played_at")?,
        // Filled in by `attach_song_relations`: one row cannot carry them.
        artists: Vec::new(),
        genres: Vec::new(),
        album_artist: row.try_get("album_artist_name")?,
        album_artist_id: row
            .try_get::<Option<String>, _>("album_artist_id")?
            .map(parse_uuid)
            .transpose()?,
        musicbrainz_id: row.try_get("musicbrainz_recording_id")?,
        replay_gain_track_gain: row.try_get("replay_gain_track_gain")?,
        replay_gain_track_peak: row.try_get("replay_gain_track_peak")?,
        replay_gain_album_gain: row.try_get("replay_gain_album_gain")?,
        replay_gain_album_peak: row.try_get("replay_gain_album_peak")?,
        bpm: row.try_get("bpm")?,
        sort_name: row.try_get("sort_title")?,
        comment: row.try_get("comment")?,
        isrc: split_tag_values(row.try_get::<Option<String>, _>("isrc")?.as_deref()),
        moods: split_tag_values(row.try_get::<Option<String>, _>("moods")?.as_deref()),
        explicit_status: row.try_get("explicit_status")?,
    })
}

async fn replace_playlist_tracks(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    playlist: Uuid,
    ids: &[Uuid],
    now: i64,
) -> Result<(), sqlx::Error> {
    for (position, id) in ids.iter().enumerate() {
        sqlx::query("INSERT INTO playlist_track (playlist_id, track_id, position, added_at) VALUES (?, ?, ?, ?)")
            .bind(playlist.to_string()).bind(id.to_string()).bind(position as i64).bind(now).execute(&mut **tx).await?;
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), ServiceError> {
    if (1..=200).contains(&name.trim().chars().count()) {
        Ok(())
    } else {
        Err(ServiceError::Invalid)
    }
}

fn validate_replay_type(receipt: &MutationReceipt, expected: &str) -> Result<(), ServiceError> {
    if receipt.entity_type == expected {
        Ok(())
    } else {
        Err(ServiceError::Conflict)
    }
}

fn validate_username(username: &str) -> Result<(), ServiceError> {
    let username = username.trim();
    if !(3..=64).contains(&username.len())
        || !username.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        Err(ServiceError::Invalid)
    } else {
        Ok(())
    }
}

fn parse_uuid(value: String) -> Result<Uuid, sqlx::Error> {
    Uuid::from_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}
