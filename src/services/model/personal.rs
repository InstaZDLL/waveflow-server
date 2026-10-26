//! What one account keeps for itself: bookmarks, radio stations, the play
//! queue, ratings, history, shares, and the snapshot a sync starts from.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{catalog::SongItem, playlists::PlaylistItem};

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

/// An internet radio station one account keeps.
///
/// Not a catalogue entity: no library, no tags, nothing scanned. The server
/// stores the address and hands it to the client, and never fetches it — see
/// `src/services/radio.rs` for why that single decision is the feature's security
/// answer. The station's image is the one address it fetches, once, when it
/// is set.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RadioStationItem {
    pub id: Uuid,
    pub name: String,
    /// What a player is pointed at. Stored as the account typed it.
    pub stream_url: String,
    /// The station's own page, when it gave one. Omitting it on a write clears
    /// it, because a station is replaced rather than patched.
    pub homepage_url: Option<String>,
    /// The image address the account gave, kept whether or not it could be
    /// fetched. Shown to the station's own account only, as the station is.
    pub image_url: Option<String>,
    /// The image, fetched once into the artwork store
    /// (`/api/v2/artwork/{hash}`). `None` when there is no image address or
    /// its fetch failed; a client then shows its default symbol.
    pub artwork_hash: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
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
    /// Lift the visit cap (RFC-020 decision 6).
    pub max_visits: bool,
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
    /// How many times the share may be opened, or `None` for no cap (RFC-020
    /// decision 6).
    pub max_visits: Option<i64>,
    pub songs: Vec<SongItem>,
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
