//! Shared v2 domain services and tenant-filtered read models.

use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqliteConnection};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    auth::authentication::now_ms,
    auth::security::{self, EncryptedSecret, SecretBox},
    library::lyrics::{self, LyricsList, StructuredLyrics},
    library::playlist_file::{self, PlaylistFileEntry},
    storage::database::{
        AccountRecord, AccountRole, ApiTokenRecord, Database, SubsonicCredentialStatus,
    },
    storage::sync::{
        MutationContext, MutationIntent, MutationReceipt, OperationClaim, SyncService,
    },
};

#[macro_use]
mod projections;

#[derive(Clone)]
pub struct DomainServices {
    db: Database,
    secret_box: Arc<SecretBox>,
    sync: SyncService,
    scanner: crate::library::scanner::ScanManager,
    uploads: crate::config::UploadLimits,
    library_event_retention: crate::config::LibraryEventRetention,
    /// How long a missing track is kept before the retention pass purges it,
    /// or `None`, the default, for never (RFC-021 decision 4).
    unavailable_retention: Option<std::time::Duration>,
    /// How long a purge stays restorable before its tracks are deleted for
    /// good; zero deletes at once (RFC-021, undoing a purge).
    purge_grace: std::time::Duration,
    /// The folders outside every library a stored path may lead into through
    /// a link (`WAVEFLOW_LINK_ROOTS`): the quality analysis reads those files
    /// as the stream does.
    link_roots: crate::library::link_roots::LinkRoots,
    /// Bounds a credential an account mints for itself. See the field on
    /// [`crate::config::Config`] for why the administrative path is not
    /// bounded by it.
    api_token_ttl: Option<std::time::Duration>,
    /// One lock per open upload session.
    ///
    /// A fragment is a file write and a row update that have to agree, and the
    /// writer gate cannot be what makes them agree: file I/O has no business
    /// happening while the process-wide gate is held. The same shape the
    /// scanner uses for its per-library lock.
    upload_locks: Arc<dashmap::DashMap<Uuid, Arc<tokio::sync::Mutex<()>>>>,
    canvas: crate::config::CanvasLimits,
    canvas_dir: PathBuf,
    /// Where covers and their thumbnails live. Read by the sweep only: the
    /// scanner writes this directory through `waveflow_core`, which is the
    /// whole reason the sweep cannot hold a lock over the writer.
    artwork_dir: PathBuf,
    ffprobe_path: PathBuf,
    ffmpeg_path: PathBuf,
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
    scrobbling: crate::config::ScrobbleLimits,
    /// The destinations this process knows how to reach.
    ///
    /// Filled after construction rather than at it, which is what lets a test
    /// drive the whole queue against a double and lets a server run with no
    /// outbound adapter at all — the ordinary case, since RFC-010 decision 4
    /// forbids shipping any provider credential in an AGPL binary.
    scrobble_targets: scrobbling::ScrobbleTargets,
    /// Every instance of every destination the operator declared, by
    /// `(provider, name)`, with the fingerprint of its URL.
    ///
    /// Copied out of `Config` like the limits beside it. It is what makes a
    /// link's named destination checkable — at the moment it is posed, at
    /// reconciliation, and before a retry leaves — without three readings of
    /// the environment that could disagree.
    scrobble_destinations: Arc<std::collections::HashMap<(ScrobbleProvider, String), String>>,
    /// What one Last.fm journey needs beyond an account and a destination, or
    /// `None` when this server cannot carry one at all.
    lastfm_journey: Option<lastfm::LastFmJourney>,
    /// Why not, as a code the caller words. Published beside the
    /// destination rather than discovered when somebody tries to link.
    lastfm_unavailable: Option<ScrobbleUnavailable>,
    /// How a request token becomes a session key, by destination name. Filled
    /// after construction like [`Self::register_scrobble_target`], and for the
    /// same reason.
    lastfm_exchanges: lastfm::LastFmExchanges,
    /// Metadata enrichment — RFC-011. Disabled at construction and placed by
    /// [`Self::use_enrichment`], like the scrobbling adapters and for the same
    /// reason: a test drives it against a provider of its own.
    enrichment: Arc<std::sync::RwLock<crate::remote::enrichment::Enrichment>>,
    /// How long a portrait search's task may run, in milliseconds (RFC-013):
    /// [`ARTIST_IMAGE_SEARCH_DEADLINE`], except where a test shortens it
    /// rather than wait five minutes.
    image_search_deadline_ms: Arc<std::sync::atomic::AtomicU64>,
    /// The quality analysis (RFC-016): its bound, the FFmpeg it measures with,
    /// and the spectrograms in flight.
    quality: quality::QualityRuntime,
    /// Lyrics from LRCLIB — RFC-024. Disabled at construction and placed by
    /// [`Self::use_lyrics_lookup`], like enrichment and for the same reason.
    lyrics_lookup: Arc<std::sync::RwLock<crate::remote::lrclib::LyricsLookup>>,
    /// Podcasts (RFC-025): the settings, and the one download permit and one
    /// refresh lock the background work shares.
    podcasts: podcasts::PodcastRuntime,
    /// Podcast covers and station images (RFC-025, amended): whether they are
    /// fetched, how long one may take, and which addresses it may reach.
    remote_artwork: crate::config::RemoteArtworkSettings,
    /// How many station images each account has had fetched this hour.
    station_images: remote_artwork::FetchBudget,
    /// Smart playlists (RFC-025): what each reader was last shown, the
    /// evaluation counter and the evaluation budget.
    smart: smart_playlists::SmartRuntime,
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
    /// The caller already has one of these in flight and must wait for it:
    /// a portrait search on Deezer (RFC-013), one per account at a time.
    #[error("too many requests")]
    Busy,
    #[error("service unavailable")]
    Unavailable,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Security(#[from] security::SecurityError),
}

impl From<crate::storage::sync::SyncError> for ServiceError {
    fn from(error: crate::storage::sync::SyncError) -> Self {
        match error {
            crate::storage::sync::SyncError::Invalid => Self::Invalid,
            // Mutations claim operations; they never read the journal, so
            // CursorExpired cannot reach this conversion. Folded into Conflict
            // rather than given a domain variant nothing would ever construct.
            crate::storage::sync::SyncError::Conflict
            | crate::storage::sync::SyncError::CursorExpired => Self::Conflict,
            crate::storage::sync::SyncError::Database(error) => Self::Database(error),
        }
    }
}

mod admin;
mod albums;
mod artist_enrichment;
mod artist_image_search;
mod artist_images;
mod artists;
mod artwork;
mod attention;
mod bookmarks;
mod canvas;
mod catalog;
mod credentials;
mod credit_decisions;
mod favorites;
mod lastfm;
mod library_events;
mod library_folders;
mod library_lifecycle;
mod library_tree;
mod lyrics_uploads;
mod playback;
mod playlist_admin;
mod playlist_order;
mod playlists;
mod podcasts;
mod quality;
mod radio;
mod remote_artwork;
mod remote_lyrics;
// RFC-024: what the listening history and the catalogue suggest.
mod local_similar;
mod mixes;
mod scan;
mod scrobbling;
mod search;
mod shares;
mod smart_playlists;
mod songs;
mod statistics;
mod sync;
mod track_metadata;
pub(crate) mod uploads;

mod access;
mod model;
mod relations;
mod rows;
mod validate;

use projections::folder_filter;
use relations::{attach_album_relations, attach_song_relations, fetch_songs};
use rows::{
    album_from_row, artist_from_row, artist_summary_from_row, credential_from_row,
    lyrics_list_from_rows, parse_uuid, song_from_row,
};
use validate::{validate_name, validate_replay_type, validate_username};

/// The records the services answer with, filed by domain under `model/` and
/// named from here, which is the path every caller has always used.
pub(crate) use model::metadata::present;
pub use model::{
    accounts::{AuthorizationRequest, SubsonicCredentialRecord, UserItem, UserUpdate},
    browse::{
        playlist_import_body_limit, queue_body_limit, AlbumListQuery, AlbumOrder, BrowsePage,
        SongListQuery, SongOrder, MAX_BROWSE_LIMIT, MAX_HISTORY_LIMIT, MAX_PLAYLIST_TRACKS,
        MAX_QUEUE_TRACKS, MAX_REQUEST_IDENTIFIERS, MAX_SHARE_TRACKS,
    },
    catalog::{
        AlbumDetail, AlbumItem, ArtistDetail, ArtistItem, ArtistRef, ArtistSummary,
        CatalogOverview, CatalogSearch, CatalogSnapshot, Contributor, DiscTitle, GenreItem,
        MusicFolderItem, ReleaseGroup, SearchResult, SongItem, StarredCatalog, TrackDetail,
    },
    enrichment::{
        ArtistEnrichment, EnrichedBio, EnrichedImage, EnrichedSimilar, EnrichmentSource,
        EnrichmentState, ImageChoice, ImageChoices,
    },
    library::{LibraryEvent, LibraryEventPage, LibraryMember},
    metadata::{TrackMetadataPatch, TrackOverrideValues, TrackOverrides, TrackSourceTags},
    personal::{
        BookmarkItem, HistoryItem, QueueItem, RadioStationItem, RatingItem, ShareClear, ShareItem,
        SyncSnapshotData,
    },
    playlists::{PlaylistClear, PlaylistImport, PlaylistItem, UnresolvedEntry, UnresolvedReason},
    uploads::{
        CanvasBlob, CommittedUpload, UploadDecision, UploadOffer, UploadSessionState, UploadVerdict,
    },
};

pub use artist_image_search::{ArtistImageCandidate, ARTIST_IMAGE_SEARCH_DEADLINE};
pub use artist_images::ArtistImageKind;
pub use attention::{AttentionCategory, AttentionCount, AttentionTrack};
pub use credit_decisions::{
    CreditCandidate, CreditDecisionKind, CreditDecisionRecord, CreditFragment,
};
/// Crate-visible only: the adapter builds the browser-less journey's address
/// from it, and nothing outside this binary has a use for it.
pub(crate) use lastfm::LASTFM_AUTHORIZE_URL;
/// The scrobbling vocabulary, which is the one thing in this module tree that
/// has to be spoken outside it: an adapter implements [`ScrobbleTarget`]
/// without being part of the domain, and the API reads the link states and the
/// entries still waiting for a person to decide about them.
///
/// A named list rather than a glob, so that crossing this boundary stays a
/// decision. `UncertainScrobble` joined it when the routes arrived, because
/// decision 13 asks somebody to choose and nothing outside could see what
/// about.
pub use lastfm::{
    LastFmApproval, LastFmJourneyStart, LastFmSessionExchange, LASTFM_CALLBACK_PREFIX,
    LASTFM_JOURNEY_COOKIE,
};
pub use library_folders::{
    FiledFolder, FiledTree, FolderPatch, FolderRemoved, Placed, MAX_FOLDERS_PER_LIBRARY,
    MAX_FOLDER_DEPTH, MAX_PLACEMENTS,
};
pub use library_lifecycle::{
    LibraryActor, LibraryDeleted, LibrarySettings, LibraryUpdate, LibraryUpdated, PurgeOutcome,
    PurgeRequest, RestoreOutcome, UnavailableTrack, UnavailableTracks,
};
pub use library_tree::{DiskFolder, DiskListing, DiskTrack};
pub use lyrics_uploads::{LyricsFile, MAX_UPLOAD_BYTES as MAX_LYRICS_UPLOAD_BYTES};
pub use mixes::{Mix, MixQuery, MixSummary, DEFAULT_MIX_SIZE, MAX_MIX_SIZE};
pub use podcasts::{
    EpisodeBookmark, PodcastChannelItem, PodcastEpisodeItem, PodcastFailure, PodcastOverview,
    MAX_EPISODES_LISTED, NEWEST_EPISODES_DEFAULT,
};
pub use quality::{
    AnalysisState, QualityAnalysis, QualitySummary, QualityTrack, QualityVerdict,
    QualityVerdictCount, Spectrogram, TrackQuality, TrackQualityDetail,
};
pub use radio::StationImage;
pub use scrobbling::{
    ScrobbleDestinationName, ScrobbleDrain, ScrobbleEnvelope, ScrobbleLinkState, ScrobbleProvider,
    ScrobbleTarget, ScrobbleUnavailable, ScrobbleVerdict, UncertainScrobble,
};
pub use smart_playlists::{
    RuleError, SmartCondition, SmartField, SmartMatch, SmartOperator, SmartOrder,
    SmartPlaylistItem, SmartPlaylistPreview, SmartRules, SmartSort, MAX_SMART_PLAYLIST_TRACKS,
    SMART_EVALUATION_BUDGET_OPS,
};
pub use statistics::{
    ListeningStats, PlayMoment, StatsBucket, StatsBucketSize, StatsQuery, StatsTotals, TopAlbum,
    TopArtist, TopGenre, TopTrack,
};

impl DomainServices {
    pub fn new(
        db: Database,
        secret_box: Arc<SecretBox>,
        sync: SyncService,
        scanner: crate::library::scanner::ScanManager,
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
            library_event_retention: config.library_event_retention,
            unavailable_retention: config.unavailable_retention,
            purge_grace: config.purge_grace,
            link_roots: crate::library::link_roots::LinkRoots::new(config.link_roots.clone()),
            api_token_ttl: config.api_token_ttl,
            upload_locks: Arc::new(dashmap::DashMap::new()),
            canvas: config.canvas,
            canvas_dir: config.canvas_dir.clone(),
            artwork_dir: config.artwork_dir.clone(),
            ffprobe_path: config.ffprobe_path.clone(),
            ffmpeg_path: config.ffmpeg_path.clone(),
            canvas_locks: Arc::new(dashmap::DashMap::new()),
            scrobbling: config.scrobbling,
            scrobble_targets: Arc::new(dashmap::DashMap::new()),
            scrobble_destinations: Arc::new(
                config
                    .destinations
                    .iter()
                    .map(|destination| {
                        (
                            (destination.provider, destination.name.clone()),
                            destination.fingerprint.clone(),
                        )
                    })
                    .collect(),
            ),
            lastfm_journey: lastfm::LastFmJourney::from_config(config),
            lastfm_unavailable: lastfm::lastfm_unavailability(config),
            lastfm_exchanges: Arc::new(dashmap::DashMap::new()),
            enrichment: Arc::new(std::sync::RwLock::new(
                crate::remote::enrichment::Enrichment::disabled(),
            )),
            image_search_deadline_ms: Arc::new(std::sync::atomic::AtomicU64::new(
                u64::try_from(ARTIST_IMAGE_SEARCH_DEADLINE.as_millis()).unwrap_or(u64::MAX),
            )),
            quality: quality::QualityRuntime::new(
                config.quality_parallelism,
                config.request_timeout,
            ),
            lyrics_lookup: Arc::new(std::sync::RwLock::new(
                crate::remote::lrclib::LyricsLookup::disabled(),
            )),
            podcasts: podcasts::PodcastRuntime::new(config.podcasts.clone()),
            remote_artwork: config.remote_artwork,
            station_images: remote_artwork::FetchBudget::default(),
            smart: smart_playlists::SmartRuntime::new(config.db_max_connections),
        }
    }

    /// Places the enrichment engine every reader goes through.
    pub fn use_enrichment(&self, enrichment: crate::remote::enrichment::Enrichment) {
        *self
            .enrichment
            .write()
            .unwrap_or_else(|poison| poison.into_inner()) = enrichment;
    }
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
