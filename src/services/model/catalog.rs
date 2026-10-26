//! The catalogue's read models: songs, albums, artists, and the browse,
//! search and detail views built from them.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::enrichment::{ArtistEnrichment, ImageChoices};

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
    /// Which section of a discography the album belongs in, decided once here
    /// from `release_types` and `is_compilation` (RFC-022 decision 2) so no
    /// client reinterprets MusicBrainz's vocabulary its own way. Native only:
    /// the Subsonic nodes are built field by field and never read it.
    pub release_group: ReleaseGroup,
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

/// The section of an artist's discography an album is listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseGroup {
    Album,
    Ep,
    Single,
    Live,
    Compilation,
}

impl ReleaseGroup {
    /// Classifies an album by the release types its files carry (RFC-022
    /// decision 2).
    ///
    /// The values are MusicBrainz's, as taggers write them: a primary type
    /// (`album`, `single`, `ep`, `broadcast`, `other`) and any secondary ones
    /// (`compilation`, `live`, `soundtrack`, …). The scanner joins several
    /// with `;`, and Picard's ID3v2.3 output joins them with `/` instead, so
    /// each value is split on both — and on `,` — then compared without case.
    ///
    /// Secondary types win over primary ones, as on MusicBrainz's own artist
    /// pages: a live EP is filed under live, a compilation album under
    /// compilations. Anything else, or nothing at all, is an album, so a
    /// library that carries no release type keeps the one section it had.
    pub fn classify(release_types: &[String], is_compilation: bool) -> Self {
        let mut compilation = is_compilation;
        let (mut live, mut ep, mut single) = (false, false, false);
        for kind in release_types
            .iter()
            .flat_map(|value| value.split([';', '/', ',']))
            .map(str::trim)
        {
            if kind.eq_ignore_ascii_case("compilation") {
                compilation = true;
            } else if kind.eq_ignore_ascii_case("live") {
                live = true;
            } else if kind.eq_ignore_ascii_case("ep") {
                ep = true;
            } else if kind.eq_ignore_ascii_case("single") {
                single = true;
            }
        }
        if compilation {
            Self::Compilation
        } else if live {
            Self::Live
        } else if ep {
            Self::Ep
        } else if single {
            Self::Single
        } else {
            Self::Album
        }
    }
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
    /// Where the file sits in its library, relative to the library root.
    ///
    /// Read with every song and serialised with none: listings would carry a
    /// path per row that nobody browsing albums needs. `GET /tracks/{id}`
    /// sends it, through [`TrackDetail`], for the properties panel and the
    /// library browser (RFC-012 decision 1). The root itself is never sent.
    #[serde(skip)]
    pub relative_path: String,
}

/// One track as `GET /api/v2/tracks/{id}` answers it: the song, and where its
/// file sits in the library.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TrackDetail {
    #[serde(flatten)]
    pub song: SongItem,
    /// Relative to the library root, with `/` separators whatever the
    /// server's platform. Never absolute.
    pub relative_path: String,
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
    /// The albums a track of which credits this artist in its `artist` role,
    /// while the artist is not one of the album's own artists — "Appears on"
    /// (RFC-022 decision 3).
    ///
    /// Filled by [`DomainServices::artist_page`](crate::services::DomainServices::artist_page) alone, like the enrichment:
    /// the Subsonic `getArtist` path has nowhere to put it, and a client that
    /// syncs every artist would pay for a query it never reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appears_on: Option<Vec<AlbumItem>>,
    /// What metadata enrichment says about this artist — RFC-011 decision 11.
    ///
    /// Always present on `GET /api/v2/artists/{id}`, whose answer comes from
    /// [`DomainServices::artist_page`](crate::services::DomainServices::artist_page). Left out of the Subsonic `getArtist`
    /// path, which renders its own nodes: clients that sync call it for every
    /// artist, and asking providers about the whole catalogue at sync time is
    /// the prefill decision 6 refuses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrichment: Option<ArtistEnrichment>,
    /// The banner this artist shows: the one chosen by hand (RFC-013
    /// decision 3), or else TheAudioDB's fanart (RFC-014 decision 4). Absent
    /// when there is neither, and a client then draws what it drew before.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner_hash: Option<String>,
    /// What was chosen by hand, kind by kind, so a client can say "chosen"
    /// rather than "from Deezer". Absent when nothing was.
    #[serde(skip_serializing_if = "ImageChoices::is_empty")]
    pub image_choice: ImageChoices,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SearchResult {
    pub artists: Vec<ArtistItem>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
}
