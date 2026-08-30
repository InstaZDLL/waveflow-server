//! Shared v2 domain services and tenant-filtered read models.

use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};

use serde::{Deserialize, Serialize};
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
        "SELECT t.id, t.library_id, t.album_id, \
                COALESCE(ovr.title, t.title) AS title, t.album_title, t.artist_display, \
                (SELECT tp.artist_id FROM track_participant tp \
                  WHERE tp.track_id=t.id AND tp.role='artist' \
                  ORDER BY tp.position LIMIT 1) AS artist_id, \
                t.genre_display, COALESCE(ovr.year, t.year) AS year, \
                COALESCE(ovr.track_number, t.track_number) AS track_number, \
                COALESCE(ovr.disc_number, t.disc_number) AS disc_number, t.duration_ms, t.bitrate, \
                t.codec, t.relative_path, t.file_size, t.artwork_hash, t.full_hash, t.created_at, \
                us.starred_at, ur.rating AS user_rating, \
                t.sample_rate, t.channels, t.bit_depth, \
                (SELECT COUNT(*) FROM play_event pe \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pe.track_id=t.id) \
                 AS play_count, \
                (SELECT MAX(pe.played_at) FROM play_event pe \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pe.track_id=t.id) \
                 AS last_played_at, \
                COALESCE(ovr.musicbrainz_recording_id, t.musicbrainz_recording_id) \
                  AS musicbrainz_recording_id, \
                t.replay_gain_track_gain, t.replay_gain_track_peak, \
                t.replay_gain_album_gain, t.replay_gain_album_peak, t.bpm, \
                COALESCE(ovr.sort_title, t.sort_title) AS sort_title, \
                COALESCE(ovr.comment, t.comment) AS comment, t.isrc, t.moods, t.explicit_status, \
                alb.album_artist_name, alb.album_artist_id \
         FROM track t JOIN library_member m ON m.library_id=t.library_id \
         LEFT JOIN album alb ON alb.id=t.album_id \
         LEFT JOIN track_override ovr ON ovr.track_id=t.id \
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
                al.original_release_date, al.release_date, al.release_types, al.record_labels, \
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
                    ar.sort_name, us.starred_at, ur.rating AS user_rating, \
                    (SELECT group_concat(role) FROM \
                       (SELECT ars.role FROM artist_role_stats ars \
                         WHERE ars.artist_id=ar.id ORDER BY ars.role)) AS roles",
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
    /// The capacities this artist is credited in, anywhere in the catalogue.
    ///
    /// Derived from the credits rather than stored on the row, so an artist
    /// who stops being a producer stops saying so at the next scan.
    pub roles: Vec<String>,
}

/// A disc of an album, and the title its tracks give it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DiscTitle {
    pub disc: i64,
    pub title: String,
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
    /// The release description OpenSubsonic asks an album for. The dates are
    /// kept as the file spelled them and taken apart only at the wire — a tag
    /// naming a year alone must not be reported as the first of January.
    pub original_release_date: Option<String>,
    pub release_date: Option<String>,
    pub release_types: Vec<String>,
    pub record_labels: Vec<String>,
    /// One entry per disc the album's available tracks name a title for.
    /// Derived like the genres, because an album has as many as it has discs.
    pub disc_titles: Vec<DiscTitle>,
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
    /// Every artist the album is credited to, which the single `album_artist_id`
    /// above can only ever name the first of. It stays because the frozen
    /// `artistId` field needs one.
    pub album_artists: Vec<ArtistRef>,
    /// Every credit that is neither the track's artist nor its album artist:
    /// composer, producer, performer and the rest, in role then tag order.
    pub contributors: Vec<Contributor>,
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

/// One artist credited on a track in some capacity other than being its
/// artist or its album artist.
///
/// The role is the reference's own name for it, and `sub_role` carries the
/// instrument a performer is credited on — the only role that has one.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct Contributor {
    pub role: String,
    pub sub_role: Option<String>,
    pub artist: ArtistRef,
}

/// A browse view that stops short of the tracks.
#[derive(Debug, Clone)]
pub struct CatalogOverview {
    pub folders: Vec<MusicFolderItem>,
    /// Carried as summaries because the browse that renders them needs the
    /// album count, and computing it in the facade was a loop over every album
    /// for every artist.
    pub artists: Vec<ArtistSummary>,
    pub albums: Vec<AlbumItem>,
}

#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    pub folders: Vec<MusicFolderItem>,
    pub artists: Vec<ArtistSummary>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
}

/// The complete set of corrections a track carries.
///
/// Wholesale rather than incremental: the body is every override the track
/// should have afterwards, and a field left out is not overridden. A tag editor
/// sends its whole form, so clearing a correction is saying nothing about that
/// field rather than sending a null that means something special. An empty or
/// blank string is read the same way — no correction — because a track with no
/// title is not a correction anyone wants to store.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct TrackMetadataPatch {
    pub title: Option<String>,
    pub sort_title: Option<String>,
    pub year: Option<i64>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub musicbrainz_recording_id: Option<String>,
    pub comment: Option<String>,
    /// The track's artists, in order, and its genres. Explicit lists rather
    /// than the `;`-joined string a file carries: that form exists because a
    /// tagger writes names however it likes and the mapper has to guess where
    /// one ends, which is not a guess worth reintroducing on a list someone
    /// typed on purpose.
    pub artists: Option<Vec<String>>,
    pub genres: Option<Vec<String>>,
}

/// One file a client is offering, before any of its bytes have moved.
///
/// `full_hash` is what the client computed over the whole file. It is used to
/// avoid a transfer and never to establish an identity: the server recomputes
/// it from the bytes it actually received, and an identity founded on what a
/// client asserts would let any authorised member pass one file off as another.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UploadOffer {
    pub full_hash: String,
    pub size_bytes: i64,
    /// Without its dot, and matched case-insensitively against the extensions
    /// the scanner can index. It is not proof of anything — the file is read
    /// for real at commit — but it is what makes a refusal cheap here rather
    /// than after the last byte.
    pub extension: String,
}

/// What the server decided about one offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UploadDecision {
    /// This library already holds these bytes. Scoped to this library on
    /// purpose: a server-wide lookup would tell a member of one library that
    /// some library they cannot see holds exactly this file, and would leave
    /// them believing they had a track they do not have.
    Present,
    /// A session is open and the bytes are wanted.
    Accepted,
    /// The extension is not one the scanner can index, so storing the file
    /// would spend disk on something the catalogue could never show.
    UnsupportedFormat,
    /// Above the per-file ceiling.
    TooLarge,
    /// The library has no room, counting what open sessions have reserved.
    QuotaExceeded,
    /// The caller is entitled to be here and the library does not accept
    /// files. Distinct from a 404, which is what someone not entitled gets.
    LibraryClosed,
    /// The account already holds as many sessions as it may.
    TooManySessions,
}

/// An open session, as the client needs to see it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UploadSessionState {
    pub session_id: Uuid,
    /// The fragment the server wants next. A client that restarts asks rather
    /// than assumes, so a lost acknowledgement costs one request instead of a
    /// transfer.
    pub next_chunk: i64,
    pub received_bytes: i64,
    /// The size to send each fragment at, advertised rather than assumed.
    pub chunk_bytes: i64,
    pub expires_at: i64,
}

/// One blob of the canvas store.
///
/// Cross-cutting rather than private to the canvas service: the media surface
/// serves it, and the hash it carries is both the file name and the `ETag`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct CanvasBlob {
    pub hash: String,
    /// `mp4` or `webm`, decided by reading the bytes rather than by trusting
    /// what the request called them.
    pub format: String,
    pub byte_size: i64,
}

impl CanvasBlob {
    /// The name the bytes carry on disk. The hash is the name, so this is a
    /// pure function of the row rather than something stored twice.
    pub fn file_name(&self) -> String {
        format!("{}.{}", self.hash, self.format)
    }
}

/// One verdict, carrying back the hash it answers.
///
/// The hash rather than a position: a client matching verdicts to offers by
/// index is one reordering away from filing a session under the wrong file.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UploadVerdict {
    pub full_hash: String,
    pub decision: UploadDecision,
    /// Set when the decision is `present` — the track that already holds these
    /// bytes, so the client can reconcile without a catalogue sweep.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub track_id: Option<Uuid>,
    /// Set when the decision is `accepted`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<UploadSessionState>,
}

/// What a committed upload became.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CommittedUpload {
    /// The track the file became, available immediately rather than at the next
    /// scan.
    pub track_id: Uuid,
    /// Recomputed from the bytes that arrived, never the one the client
    /// declared. At this moment, and only at this moment, both sides know their
    /// file and this track are the same bytes — which is the link a client
    /// would otherwise re-read the whole file to establish.
    pub full_hash: String,
}

/// One fact about a library's catalogue, as its change feed tells it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryEvent {
    pub cursor: i64,
    /// `track`, `album` or `artist` — the vocabulary is CHECK-constrained in
    /// the schema because it is part of the contract.
    pub entity_type: String,
    pub entity_id: Uuid,
    /// `upsert` or `delete`.
    pub action: String,
    /// For a track upsert, carries `full_hash`. Nothing else tells a client
    /// that a file was retagged outside the API: the track keeps its id while
    /// its bytes move.
    pub payload: serde_json::Value,
    pub changed_at: i64,
    /// The device that asked for this change, when a client did.
    ///
    /// `None` means no client asked — a scan wrote it — or that the client did
    /// not name a device. A client filters its own writes out of the feed with
    /// this; without it, its own upload comes back as a track it just
    /// discovered and it treats it as one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_device_id: Option<Uuid>,
}

/// A page of one library's change feed.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryEventPage {
    pub events: Vec<LibraryEvent>,
    pub next_cursor: i64,
    pub has_more: bool,
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
/// Upper bound on the tracks one playlist may hold.
///
/// Deliberately not [`MAX_QUEUE_TRACKS`]: that one bounds a request, and a
/// queue is written whole by every call. A playlist grows across many calls, so
/// the same number would refuse ordinary libraries. What this bounds is the
/// rewrite — `replace_playlist_tracks` deletes and reinserts the whole list on
/// every edit, under the process-wide writer gate — and ten thousand keeps that
/// bounded while sitting far above any playlist a person curates by hand.
pub const MAX_PLAYLIST_TRACKS: usize = 10_000;

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
    uploads: crate::config::UploadLimits,
    /// One lock per open upload session.
    ///
    /// A fragment is a file write and a row update that have to agree, and the
    /// writer gate cannot be what makes them agree: file I/O has no business
    /// happening while the process-wide gate is held. The same shape the
    /// scanner uses for its per-library lock.
    upload_locks: Arc<dashmap::DashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    canvas: crate::config::CanvasLimits,
    canvas_dir: PathBuf,
    ffprobe_path: PathBuf,
    /// One lock per canvas blob, keyed by its hash.
    ///
    /// Placing and removing the same blob must not interleave: between a
    /// removal's commit and its unlink, a placement of the same content would
    /// find no row, write its bytes and insert its own, and the unlink would
    /// then carry off the file of a live link. Keyed by hash rather than being
    /// the writer gate, because the race is per blob and because file I/O has
    /// no business happening while the process-wide gate is held — the same
    /// reason `upload_locks` exists.
    canvas_locks: Arc<dashmap::DashMap<String, Arc<tokio::sync::Mutex<()>>>>,
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

mod admin;
mod albums;
mod artists;
mod bookmarks;
mod canvas;
mod catalog;
mod credentials;
mod favorites;
mod library_events;
mod playback;
mod playlists;
mod scan;
mod search;
mod shares;
mod songs;
mod sync;
mod track_metadata;
mod uploads;

impl DomainServices {
    pub fn new(
        db: Database,
        secret_box: Arc<SecretBox>,
        sync: SyncService,
        scanner: crate::scanner::ScanManager,
        config: &crate::config::Config,
    ) -> Self {
        // Copied here rather than borrowed, which is why mutating a `Config`
        // after `initialize` changes nothing: these are the values this
        // instance runs under for its lifetime.
        Self {
            db,
            secret_box,
            sync,
            scanner,
            uploads: config.uploads,
            upload_locks: Arc::new(dashmap::DashMap::new()),
            canvas: config.canvas,
            canvas_dir: config.canvas_dir.clone(),
            ffprobe_path: config.ffprobe_path.clone(),
            canvas_locks: Arc::new(dashmap::DashMap::new()),
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
    let mut disc_titles: HashMap<Uuid, Vec<DiscTitle>> = HashMap::new();
    for row in sqlx::query(
        // One title per disc, and the first spelling in disc order when the
        // tracks of one disc disagree — the same `MIN` the genres use, for the
        // same reason: an album must not report a disc twice because two of
        // its files were tagged by different hands.
        "SELECT t.album_id, t.disc_number, MIN(t.disc_subtitle) AS title FROM track t \
         JOIN library_member m ON m.library_id=t.library_id \
         WHERE m.user_id=? AND t.is_available=1 AND t.disc_subtitle IS NOT NULL \
           AND t.disc_number IS NOT NULL \
           AND t.album_id IN (SELECT value FROM json_each(?)) \
         GROUP BY t.album_id, t.disc_number \
         ORDER BY t.album_id, t.disc_number",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        disc_titles
            .entry(parse_uuid(row.try_get("album_id")?)?)
            .or_default()
            .push(DiscTitle {
                disc: row.try_get("disc_number")?,
                title: row.try_get("title")?,
            });
    }
    for album in albums {
        album.artists = artists.remove(&album.id).unwrap_or_default();
        album.genres = genres.remove(&album.id).unwrap_or_default();
        album.disc_titles = disc_titles.remove(&album.id).unwrap_or_default();
    }
    Ok(())
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
    // Everything credited on the track that is neither its artist nor its
    // album artist. Ordered by role then position so two responses for one
    // track are byte-identical — the reference emits these in map-iteration
    // order and answers differently on every request.
    let mut contributors: HashMap<Uuid, Vec<Contributor>> = HashMap::new();
    for row in sqlx::query(
        "SELECT tp.track_id, tp.role, tp.sub_role, ar.id, ar.name \
         FROM track_participant tp \
         JOIN artist ar ON ar.id=tp.artist_id \
         JOIN library_member m ON m.library_id=tp.library_id \
         WHERE m.user_id=? AND tp.role NOT IN ('artist', 'albumartist') \
           AND tp.track_id IN (SELECT value FROM json_each(?)) \
         ORDER BY tp.track_id, tp.role, tp.position, tp.sub_role",
    )
    .bind(user_id.to_string())
    .bind(&ids)
    .fetch_all(&mut *connection)
    .await?
    {
        let sub_role: String = row.try_get("sub_role")?;
        contributors
            .entry(parse_uuid(row.try_get("track_id")?)?)
            .or_default()
            .push(Contributor {
                role: row.try_get("role")?,
                sub_role: (!sub_role.is_empty()).then_some(sub_role),
                artist: ArtistRef {
                    id: parse_uuid(row.try_get("id")?)?,
                    name: row.try_get("name")?,
                },
            });
    }
    // The album's credit, which is not the track's: a guest appearance names
    // the guest while the album still belongs under its album artists. Keyed
    // on the album, so every track of one album answers the same list.
    let album_ids = serde_json::to_string(
        &songs
            .iter()
            .filter_map(|song| song.album_id)
            .collect::<Vec<_>>(),
    )
    .expect("UUID list serialization cannot fail");
    let mut album_artists: HashMap<Uuid, Vec<ArtistRef>> = HashMap::new();
    for row in sqlx::query(
        "SELECT ap.album_id, ar.id, ar.name FROM album_participant ap \
         JOIN artist ar ON ar.id=ap.artist_id \
         JOIN library_member m ON m.library_id=ap.library_id \
         WHERE m.user_id=? AND ap.role='albumartist' \
           AND ap.album_id IN (SELECT value FROM json_each(?)) \
         ORDER BY ap.album_id, ap.position, ar.name COLLATE NOCASE, ar.id",
    )
    .bind(user_id.to_string())
    .bind(&album_ids)
    .fetch_all(&mut *connection)
    .await?
    {
        album_artists
            .entry(parse_uuid(row.try_get("album_id")?)?)
            .or_default()
            .push(ArtistRef {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            });
    }
    for song in songs {
        song.artists = artists.remove(&song.id).unwrap_or_default();
        song.genres = genres.remove(&song.id).unwrap_or_default();
        song.contributors = contributors.remove(&song.id).unwrap_or_default();
        song.album_artists = song
            .album_id
            .and_then(|album| album_artists.get(&album).cloned())
            .unwrap_or_default();
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
        // Sorted here rather than trusted from `group_concat`, whose order
        // SQLite does not guarantee even when the subquery feeding it is
        // ordered. Two responses for one artist have to be byte-identical.
        roles: {
            let mut roles: Vec<String> = row
                .try_get::<Option<String>, _>("roles")?
                .map(|roles| roles.split(',').map(str::to_owned).collect())
                .unwrap_or_default();
            roles.sort_unstable();
            roles
        },
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
        original_release_date: row.try_get("original_release_date")?,
        release_date: row.try_get("release_date")?,
        release_types: split_tag_values(
            row.try_get::<Option<String>, _>("release_types")?
                .as_deref(),
        ),
        record_labels: split_tag_values(
            row.try_get::<Option<String>, _>("record_labels")?
                .as_deref(),
        ),
        // Loaded in a batch by `attach_album_relations`, never row by row.
        artists: Vec::new(),
        genres: Vec::new(),
        disc_titles: Vec::new(),
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
        album_artists: Vec::new(),
        contributors: Vec::new(),
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
