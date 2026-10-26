//! Tenant-scoped catalogue repository used by scanner and HTTP surfaces.

use std::str::FromStr;

use sqlx::{Sqlite, Transaction};
use uuid::Uuid;

mod apply;
mod availability;
mod credits;
mod derivations;
mod identity;
mod libraries;
mod overrides;
mod scan_jobs;
mod tracks;

pub use apply::{ApplyOutcome, ArtworkInput, CatalogApply, CatalogTrackInput};
pub use availability::SkipFacts;
pub use identity::{track_pid, ExistingTrack};
pub use libraries::{LibraryAccess, LibraryRecord, LibraryScanSettings};
pub use scan_jobs::ScanJobRecord;
pub use tracks::{StreamTrack, TrackRecord, MAX_SEARCH_QUERY_CHARS, MAX_SEARCH_TERMS};

pub(crate) use credits::{credit_decisions, recompute_track_participants, write_track_credits};
// Nothing outside `credits` names these today, but they were reachable at
// `crate::catalog::*`, and a split that moves code does not take paths away.
#[allow(unused_imports)]
pub(crate) use credits::{read_track_credits, Recomputed};
pub(crate) use overrides::{
    apply_track_override_lists, display_of, track_override_lists, TrackOverrideLists,
};
pub(crate) use tracks::fts_prefix_query;

/// Appends one fact to a library's change feed.
///
/// Written inside the caller's transaction so an event and the row it describes
/// commit together: a feed that could outlive a rolled-back write would tell a
/// client about a track that does not exist.
///
/// Reachable from the domain services because a scan is no longer the only
/// thing that changes a catalogue — correcting a track's tags does too, and the
/// two must land on the same feed rather than each inventing one.
/// One fact for a library's change feed.
///
/// A struct rather than a row of arguments, and not only because the list grew
/// past what clippy tolerates: `entity_type` and `action` are adjacent strings
/// with no type between them, and swapping them at a call site would compile,
/// pass, and write a vocabulary the CHECK constraint happens to accept.
pub(crate) struct LibraryChange<'a> {
    pub library_id: Uuid,
    pub entity_type: &'a str,
    pub entity_id: Uuid,
    pub action: &'a str,
    pub payload: serde_json::Value,
    pub changed_at: i64,
    /// The device that asked, when a client did. `None` for a scan.
    pub origin_device_id: Option<Uuid>,
}

pub(crate) async fn record_library_event(
    tx: &mut Transaction<'_, Sqlite>,
    change: LibraryChange<'_>,
) -> Result<(), sqlx::Error> {
    let LibraryChange {
        library_id,
        entity_type,
        entity_id,
        action,
        payload,
        changed_at,
        origin_device_id,
    } = change;
    sqlx::query(
        "INSERT INTO library_event \
           (event_id, library_id, entity_type, entity_id, action, payload_json, changed_at, \
            origin_device_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(library_id.to_string())
    .bind(entity_type)
    .bind(entity_id.to_string())
    .bind(action)
    .bind(payload.to_string())
    .bind(changed_at)
    .bind(origin_device_id.map(|id| id.to_string()))
    .execute(&mut **tx)
    .await?;
    Ok(())
}

fn parse_uuid(value: String) -> Result<Uuid, sqlx::Error> {
    Uuid::from_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

#[cfg(test)]
mod tests;
