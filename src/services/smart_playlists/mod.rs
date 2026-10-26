//! Smart playlists: a playlist defined by rules, evaluated when it is read
//! (RFC-025 decisions 1 to 6).
//!
//! Three layers stand between a rule a client sent and the SQL that answers it,
//! and the RFC explains why each is there:
//!
//! 1. **An allowlist parse.** [`SmartRules::parse`] reads the request as JSON
//!    and accepts only the fields, operators and value types named in this
//!    file. Anything else is refused with the path of the offending value, and
//!    nothing that was refused is stored. The service methods take the parsed
//!    type, never the JSON, so there is no way to reach the database with rules
//!    that skipped the parse.
//! 2. **One static statement.** [`EVALUATE`](crate::services::smart_playlists::statement::EVALUATE) is a single literal, assembled by
//!    `concat!` like every projection here. The rules reach it as *one bound
//!    value* — a JSON document SQLite takes apart with `json_each` — and every
//!    field name, operator and logical connective in it is compared against a
//!    string literal inside a `CASE`. No byte of a rule is ever SQL text.
//! 3. **Tenancy in the statement.** It wraps `song_select!`, so the first
//!    bind is an account id and only tracks of that account's libraries can
//!    match. A playlist is evaluated as its owner, then read through the
//!    viewer's own libraries (decision 5).
//!
//! Ampache 8.4.0 shipped an SQL injection in exactly this feature, because the
//! logical operator joining the rules was concatenated into the statement. Here
//! the connective is an enum parsed from `"all"` or `"any"`, re-serialised by
//! this code, bound, and compared with `p.mode = 'all'`.

use super::*;

mod evaluation;
mod playlists;
mod rules;
mod statement;

pub use playlists::{SmartPlaylistItem, SmartPlaylistPreview};
pub use rules::{
    RuleError, SmartCondition, SmartField, SmartMatch, SmartOperator, SmartOrder, SmartRules,
    SmartSort,
};

/// The most tracks one smart playlist answers, and the default when it names
/// no limit.
///
/// A smart playlist is evaluated when it is opened — never by a listing
/// (decision 4) — and this bounds what one evaluation returns and what the
/// reader's projection then loads, whatever the size of the catalogue.
pub const MAX_SMART_PLAYLIST_TRACKS: u32 = 1_000;

/// How many conditions one playlist may carry.
pub const MAX_SMART_CONDITIONS: usize = 20;

/// How many smart playlists one account may keep.
const MAX_SMART_PLAYLISTS: i64 = 100;

/// How many virtual-machine instructions one evaluation may spend, and how
/// often it is asked whether to go on (decision 4).
///
/// Two hundred million is several seconds of work on a catalogue of a hundred
/// thousand tracks with play-count rules — enough for any rule set over a
/// real library, and a ceiling on what one request can make the server do.
pub const SMART_EVALUATION_BUDGET_OPS: u64 = 200_000_000;
const BUDGET_STEP_OPS: i32 = 1_000;

/// The same bound in time, whichever is reached first: a slow disk spends
/// time without spending instructions.
const SMART_EVALUATION_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// How many (playlist, reader) answers the listing memory holds before it is
/// emptied. A bound on memory, not on correctness: an entry that is gone is a
/// playlist listed without counts until it is opened again.
const MAX_REMEMBERED_EXTENTS: usize = 10_000;

/// What one reader was last shown of one smart playlist, which is all a
/// listing reports (decision 4).
#[derive(Debug, Clone, Copy)]
pub(super) struct RememberedExtent {
    /// The playlist's `updated_at` when it was evaluated: a rule edit moves
    /// it, and an entry from before the edit is not reported.
    updated_at: i64,
    evaluated_at: i64,
    song_count: i64,
    duration_ms: i64,
}

/// The smart playlist half of the services' runtime state.
#[derive(Clone)]
pub(super) struct SmartRuntime {
    extents: Arc<dashmap::DashMap<(Uuid, Uuid), RememberedExtent>>,
    evaluations: Arc<std::sync::atomic::AtomicU64>,
    budget_ops: Arc<std::sync::atomic::AtomicU64>,
    /// How many evaluations may run at once: half the pool, at least one
    /// (decision 4). Each holds a connection for as long as its budget lets
    /// it, so without this bound a burst of them from any account would hold
    /// every connection the server has.
    permits: Arc<tokio::sync::Semaphore>,
    /// Evaluations running now, and the most there have been at once: what
    /// the suite reads to prove the bound holds.
    running: Arc<std::sync::atomic::AtomicU64>,
    peak: Arc<std::sync::atomic::AtomicU64>,
}

impl SmartRuntime {
    pub(super) fn new(db_max_connections: u32) -> Self {
        Self {
            extents: Arc::new(dashmap::DashMap::new()),
            evaluations: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            budget_ops: Arc::new(std::sync::atomic::AtomicU64::new(
                SMART_EVALUATION_BUDGET_OPS,
            )),
            permits: Arc::new(tokio::sync::Semaphore::new(
                usize::try_from(db_max_connections / 2).unwrap_or(1).max(1),
            )),
            running: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            peak: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }
}

#[cfg(test)]
mod tests;
