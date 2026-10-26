//! A library's change feed and its members.

use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

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
    /// The highest cursor retention has already cut away. Everything at or
    /// below it is gone; a cursor strictly below it is refused, and the only
    /// way back is the snapshot.
    ///
    /// Reported so a client can see the edge before it reaches it. Without it
    /// the first sign of trouble was the refusal itself, which is the worst
    /// moment to pay for a resync.
    ///
    /// The margin is the client's **own** cursor minus this, not
    /// `next_cursor` minus this: `next_cursor` is the end of the page just
    /// served, so it describes the margin the client will have once it has
    /// processed the page rather than the one it holds now. Either way it can
    /// spend that slack deliberately — on a network it likes, while nobody is
    /// watching — instead of at whatever moment the server says no.
    ///
    /// It is what was *purged*, not what survives. A feed whose oldest row sits
    /// at a high cursor has lost nothing, it merely started late, and a floor
    /// derived from surviving rows cannot tell those two apart.
    pub purged_through: i64,
}

/// One account's standing in a library.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryMember {
    pub user_id: Uuid,
    pub username: String,
    pub role: String,
    pub created_at: i64,
}
