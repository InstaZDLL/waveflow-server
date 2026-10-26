//! A library's lifecycle and its missing tracks (RFC-021 decisions 4 and 5):
//! renaming, moving and deleting a library, listing the tracks its scans could
//! no longer find, and purging them.
//!
//! The one implementation behind the native API, the CLI and the web client.

use super::*;

mod edit;
mod missing;
mod purge;
mod retention;

/// What a library may be changed into. A field left out is left as it is.
#[derive(Debug, Default, Clone, Deserialize, ToSchema)]
pub struct LibraryUpdate {
    /// A new name, 1 to 160 characters once trimmed. The library's owner or an
    /// administrator.
    pub name: Option<String>,
    /// A new root: an existing directory, not itself a link, and not another
    /// library's. An administrator only; queues a scan.
    pub path: Option<String>,
    /// Follow symbolic links that stay inside the root. An administrator only;
    /// queues a scan.
    pub follow_links: Option<bool>,
    /// Watch the library for changes (RFC-021 decision 6). An administrator
    /// only: a watch costs the server descriptors, and finds nothing on a
    /// network share changed by another machine. Queues nothing; the periodic
    /// scan stays either way.
    pub watch: Option<bool>,
}

impl LibraryUpdate {
    /// Whether this needs an administrator, and an administrative credential:
    /// the path and the links setting decide which part of the server's file
    /// system the server reads, and the watch what the server spends on it.
    pub fn is_administrative(&self) -> bool {
        self.path.is_some() || self.follow_links.is_some() || self.watch.is_some()
    }
}

/// What an update set in motion.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryUpdated {
    /// The scan a new path or links setting queued, if it queued one.
    pub scan_id: Option<Uuid>,
}

/// A library as an administrator manages it, its root included. Never shown
/// to a member who is not one: where the server keeps its files is the
/// operator's business.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibrarySettings {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    pub follow_links: bool,
    /// Whether the watcher observes this library.
    pub watch: bool,
}

/// What a deletion removed from the catalogue. Nothing was removed from disk.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LibraryDeleted {
    pub tracks: i64,
    pub albums: i64,
    pub artists: i64,
}

/// A track whose file the scans can no longer find.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UnavailableTrack {
    pub id: Uuid,
    /// Where the file was, relative to the library.
    pub relative_path: String,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: i64,
    /// Since when, in epoch milliseconds.
    pub unavailable_since: i64,
    /// What a purge would lose, across every account: playlist entries,
    /// favourites and recorded plays holding this track.
    pub playlist_entries: i64,
    pub favorites: i64,
    pub plays: i64,
    /// When a purge took it out of sight, for a track purged and still
    /// restorable; `null` on the missing-tracks listing.
    pub purged_at: Option<i64>,
    /// When it is deleted for good, unless restored first; `null` on the
    /// missing-tracks listing.
    pub deleted_after: Option<i64>,
}

/// One page of a library's missing tracks, oldest first.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UnavailableTracks {
    /// How many there are in all, not on this page.
    pub total: i64,
    pub tracks: Vec<UnavailableTrack>,
}

/// Which missing tracks to purge: named ones, or all of them, and never
/// neither — a purge is explicit or it does not happen.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct PurgeRequest {
    pub track_ids: Option<Vec<Uuid>>,
    #[serde(default)]
    pub all: bool,
}

/// What a purge did.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PurgeOutcome {
    /// How many tracks it purged.
    pub purged: i64,
    /// Until when they can be restored, in epoch milliseconds; `null` when
    /// the server keeps no grace window (`WAVEFLOW_PURGE_GRACE_HOURS=0`) and
    /// they are already gone.
    pub restorable_until: Option<i64>,
}

/// What a restoration put back.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RestoreOutcome {
    pub restored: i64,
}

/// Which tracks one purge is about.
enum Selection {
    All,
    Named(String),
    OlderThan(i64),
    /// Purged at or before this instant, and past their grace window.
    PurgedBy(i64),
}

impl Selection {
    /// Whether the selection names purged tracks rather than missing ones.
    fn purged(&self) -> bool {
        matches!(self, Self::PurgedBy(_))
    }
}

/// What the caller is to one library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Standing {
    Admin,
    Owner,
    Manager,
    Listener,
}

impl Standing {
    /// Owners, managers and administrators look after a library's missing
    /// tracks, as they look after what needs attention in it.
    fn curates(self) -> bool {
        matches!(self, Self::Admin | Self::Owner | Self::Manager)
    }
}

/// Who is asking, and with what authority.
///
/// An administrator's account is not enough to act as one: the credential the
/// request came in on must carry administrative authority too. A token whose
/// scopes stop short of `admin` holds only what its account's memberships
/// give it, as `AuthUser::scopes` says a narrowed credential must.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryActor {
    pub id: Uuid,
    /// Whether the credential grants administrative authority. The account's
    /// role is checked again on every operation; this can only narrow it.
    pub admin_credential: bool,
}

impl LibraryActor {
    /// The operator at the CLI, whose authority is the account's own.
    pub fn operator(id: Uuid) -> Self {
        Self {
            id,
            admin_credential: true,
        }
    }
}

/// The caller's standing on a library, read on the connection the operation
/// runs on. A library that does not exist, and one the caller is neither a
/// member of nor an administrator over, are the same answer: not found.
///
/// Administrative standing needs both halves: an active administrator's
/// account, and a credential that carries that authority.
async fn standing_on(
    connection: &mut SqliteConnection,
    actor: LibraryActor,
    library_id: Uuid,
) -> Result<Standing, ServiceError> {
    let actor_id = actor.id;
    let row = sqlx::query(
        "SELECT (SELECT role FROM library_member WHERE library_id = l.id AND user_id = ?) AS role, \
                EXISTS (SELECT 1 FROM account a WHERE a.id = ? AND a.role = 'admin' \
                        AND a.disabled = 0) AS admin \
         FROM library l WHERE l.id = ?",
    )
    .bind(actor_id.to_string())
    .bind(actor_id.to_string())
    .bind(library_id.to_string())
    .fetch_optional(&mut *connection)
    .await?
    .ok_or(ServiceError::NotFound)?;
    if actor.admin_credential && row.try_get::<i64, _>("admin")? != 0 {
        return Ok(Standing::Admin);
    }
    match row.try_get::<Option<String>, _>("role")?.as_deref() {
        Some("owner") => Ok(Standing::Owner),
        Some("manager") => Ok(Standing::Manager),
        Some("listener") => Ok(Standing::Listener),
        _ => Err(ServiceError::NotFound),
    }
}

/// An audit row with what it is about, in the transaction of the change.
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor_id: Option<Uuid>,
    kind: &str,
    library_id: Uuid,
    details: serde_json::Value,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO audit_event (actor_user_id, kind, subject_id, occurred_at, details_json) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(actor_id.map(|id| id.to_string()))
    .bind(kind)
    .bind(library_id.to_string())
    .bind(now_ms())
    .bind(details.to_string())
    .execute(&mut **tx)
    .await?;
    Ok(())
}
