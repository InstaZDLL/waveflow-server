//! Apply pipeline — materialise inbound sync ops into entity rows.
//!
//! The `sync_op` log is the truth-source of every desktop mutation;
//! until Phase 1.g.0 the server only stored ops and replayed them
//! across devices, leaving the entity tables (`playlist`, `library`,
//! …) empty for sync-originated data. The apply pipeline closes
//! that gap: each accepted op runs through [`apply_op`] in the same
//! transaction as the durable insert, so the entity row appears
//! atomically alongside the log entry.
//!
//! ## Routing
//!
//! Every op carries the source profile's canonical id (per Phase
//! 1.g.0 — see the `sync_op.profile_canonical_id` column added by
//! `20260604000000_apply_pipeline.sql`). The apply path resolves
//! that id to a server `profile.id` via
//! [`profile_resolve::find_or_provision`] before dispatching to a
//! per-entity handler. Legacy ops without a `profile_canonical_id`
//! land in the durable log but skip apply — keeps the upgrade path
//! one-way (clients gain the field on their next release, no need
//! for a backward-compat shim that would block the cleanup).
//!
//! ## Return value
//!
//! [`ApplyOutcome`] discriminates between "applied", "recognised
//! but unsupported in this server version" (e.g. `playlist_track`
//! ops that need desktop-side `file_hash` emission first), and
//! "unknown entity / op shape". The caller logs the result for
//! telemetry; durability is independent — every variant keeps the
//! row in the log so a future server release can replay through
//! compaction.
//!
//! ## Conflict resolution
//!
//! Lamport ordering is enforced at the log layer (the
//! `(user_id, device_id, lamport_ts)` UNIQUE rejects regressions
//! before apply runs). Cross-device "last writer wins" emerges
//! naturally from the apply order — pull-and-replay is monotonic
//! by `sync_op.id`, so a remote device's pull always observes the
//! most recent INSERT/UPDATE/DELETE in causal order.

use serde_json::Value;
use sqlx::PgConnection;
use thiserror::Error;
use uuid::Uuid;

use crate::sync::{Hlc, SyncOpIn};

/// Errors the apply pipeline can return. Surfaced to the push
/// handler so a malformed payload rolls back the durable insert
/// instead of leaving an unapplied op in the log forever.
#[derive(Debug, Error)]
pub enum ApplyError {
    #[error("apply: database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("apply: invalid payload for entity={entity} op={op}: {reason}")]
    InvalidPayload {
        entity: &'static str,
        op: &'static str,
        reason: String,
    },
}

/// RFC-003 Phase A.2.2 — every entity write stamp.
///
/// Bundles the §2 total-order components the apply handlers stamp
/// onto entity rows: the effective HLC pair (v2 verbatim or v1-
/// derived `(0, lamport_ts)`) plus the originator UUID parsed from
/// the push handler's `device_id`. Both are computed once per op
/// in `apply_op` so every downstream handler binds the same values.
///
/// `origin_device_id` stays `None` when the push handler's
/// `device_id` is not UUID-shaped — legacy v1 desktops use free-form
/// strings there. A legacy NULL on the entity row is documented in
/// the A.1.2 migration header; the HlcTriple comparator
/// (`payload_hash::HlcTriple`) treats `None < Some(any)` so legacy
/// rows lose to v2 ops on the tiebreaker.
///
/// `Copy` so each handler can pass it by value without lifetime
/// gymnastics.
#[derive(Debug, Clone, Copy)]
pub struct OpStamp {
    pub hlc: Hlc,
    pub origin_device_id: Option<Uuid>,
}

/// Compute the effective HLC pair for an inbound op.
///
/// V2 ops carry an explicit `hlc` (validated `wall >= 0`,
/// `logical >= 0` at the API boundary). V1 ops are derived from
/// `lamport_ts` exactly the way A.1.1's `sync_op` backfill
/// does — `(0, lamport_ts)` — so the §2 total order treats any v2
/// op (`wall > 0`) as strictly newer than every legacy-derived row.
///
/// V1 path narrowing safety: the push handler's `db::sync::insert_op_returning`
/// already validated `lamport_ts in 0..=i32::MAX` upstream before
/// calling apply, so the `as i32` here can't truncate. Saturating to
/// i32::MAX on the apply side as a defence-in-depth keeps a future
/// refactor that bypasses the gate from silently flapping bytes.
pub fn effective_hlc(op: &SyncOpIn) -> Hlc {
    match op.hlc {
        Some(h) => h,
        None => Hlc {
            wall: 0,
            logical: op.lamport_ts.clamp(0, i64::from(i32::MAX)) as i32,
        },
    }
}

/// Parse the push handler's `device_id` string as a UUID. Returns
/// `None` when the string isn't UUID-shaped — that's the legacy v1
/// case (desktops emit free-form device names). The apply handlers
/// then stamp `origin_device_id = NULL` onto the row; A.1.2's
/// migration header documents this as the v1-legacy backfill shape.
pub fn parse_origin_device_id(device_id: &str) -> Option<Uuid> {
    Uuid::parse_str(device_id).ok()
}

/// Discriminates between "applied", "recognised but skipped", and
/// "unknown". Telemetry-only — the push handler always commits the
/// durable log row regardless of the outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// Entity row was inserted / updated / deleted.
    Applied,
    /// Op recognised but unsupported in this server version. Stays
    /// in the durable log; a later server release can apply it
    /// during compaction or live replay.
    Skipped,
    /// Unknown entity. Logged for telemetry; same durability
    /// guarantees as `Skipped`.
    Unknown,
}

/// Dispatch one inbound op to its per-entity handler.
///
/// Pre-conditions: the op has already been inserted into `sync_op`
/// in the caller's transaction, the `(user_id, device_id, lamport_ts)`
/// UNIQUE has passed, and the caller holds an open `PgConnection`
/// that the entity write will share.
///
/// `device_id` is the push handler's `PushBatchRequest::device_id` —
/// the wire-shape carrier for the §2 originator. The apply pipeline
/// parses it as a UUID for entity-row `origin_device_id` stamping;
/// non-UUID strings (legacy v1 desktops) stamp NULL.
pub async fn apply_op(
    conn: &mut PgConnection,
    user_id: i64,
    device_id: &str,
    op: &SyncOpIn,
    created_at: i64,
) -> Result<ApplyOutcome, ApplyError> {
    let entity = op.entity.as_str();
    let stamp = OpStamp {
        hlc: effective_hlc(op),
        origin_device_id: parse_origin_device_id(device_id),
    };

    // Profile routing prerequisite — every entity below is profile-
    // scoped except `liked_track` / `track_rating`, which key on
    // `(user_id, file_hash)` and skip the profile lookup. The
    // resolution still runs for the profile-scoped entities so a
    // missing canonical id surfaces as `Skipped` rather than a
    // silent partial apply.
    match entity {
        "playlist" | "library" | "track" => {
            let Some(profile_canonical) = op.profile_canonical_id.as_deref() else {
                tracing::debug!(
                    entity = entity,
                    "apply: missing profile_canonical_id, skipping"
                );
                return Ok(ApplyOutcome::Skipped);
            };
            let profile_id = profile_resolve::find_or_provision(
                conn,
                user_id,
                profile_canonical,
                created_at,
                stamp,
            )
            .await?;
            match entity {
                "playlist" => playlist::apply(conn, profile_id, op, created_at, stamp).await,
                "library" => library::apply(conn, profile_id, op, created_at, stamp).await,
                "track" => track::apply(conn, profile_id, op, created_at, stamp).await,
                _ => unreachable!(),
            }
        }
        "liked_track" => liked::apply(conn, user_id, op, created_at, stamp).await,
        "track_rating" => rating::apply(conn, user_id, op, created_at, stamp).await,
        // Forward-compatibility: unknown entities are logged but
        // never error. The durable log keeps the row so a future
        // server release can replay through compaction.
        _ => {
            tracing::debug!(entity = entity, op = %op.op, "apply: unknown entity, skipping");
            Ok(ApplyOutcome::Unknown)
        }
    }
}

// ---------------------------------------------------------------
// Profile auto-provisioning.
// ---------------------------------------------------------------

mod profile_resolve {
    use sqlx::PgConnection;

    use super::{ApplyError, OpStamp};

    /// Resolve a profile canonical id to a server `profile.id`.
    /// Auto-provisions the row if it's the first op for that
    /// canonical id. The placeholder name / color match the
    /// desktop's default-profile palette so the row is usable even
    /// before any "set name" / "set color" ops land.
    ///
    /// Read-first / write-on-miss, same shape as
    /// `users::find_or_provision_by_external_id` — the common path
    /// (every op for an already-synced profile) is a single SELECT.
    ///
    /// RFC-003 Phase A.2.2 — the auto-provisioned row carries the
    /// originating op's `(hlc_wall, hlc_logical, origin_device_id)`
    /// so any future `profile + set name` op can be totally ordered
    /// against the row's current state under §2.
    pub async fn find_or_provision(
        conn: &mut PgConnection,
        user_id: i64,
        canonical_id: &str,
        now: i64,
        stamp: OpStamp,
    ) -> Result<i64, ApplyError> {
        if let Some(id) = sqlx::query_scalar::<_, i64>(
            "SELECT id FROM profile WHERE user_id = $1 AND canonical_id = $2",
        )
        .bind(user_id)
        .bind(canonical_id)
        .fetch_optional(&mut *conn)
        .await?
        {
            return Ok(id);
        }

        // Miss path — INSERT with an UPSERT fallback for the race
        // where a concurrent first-op for the same canonical id
        // landed between our SELECT and INSERT. The no-op DO UPDATE
        // keeps RETURNING firing so the loser of the race still
        // gets the winner's id.
        sqlx::query_scalar::<_, i64>(
            "INSERT INTO profile (user_id, canonical_id, name, color_id, data_dir, created_at, last_used_at, hlc_wall, hlc_logical, origin_device_id) \
             VALUES ($1, $2, $3, $4, '', $5, $5, $6, $7, $8) \
             ON CONFLICT (user_id, canonical_id) WHERE canonical_id IS NOT NULL \
                 DO UPDATE SET canonical_id = EXCLUDED.canonical_id \
             RETURNING id",
        )
        .bind(user_id)
        .bind(canonical_id)
        .bind("Synced profile")
        .bind("violet")
        .bind(now)
        .bind(stamp.hlc.wall)
        .bind(stamp.hlc.logical)
        .bind(stamp.origin_device_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(Into::into)
    }
}

// ---------------------------------------------------------------
// Helpers shared across handlers.
// ---------------------------------------------------------------

/// Extract a required string from a JSON object payload. Used by
/// every "set field" handler where the payload is `{ "value": "..." }`.
fn payload_string(
    entity: &'static str,
    op: &'static str,
    payload: Option<&Value>,
    key: &str,
) -> Result<String, ApplyError> {
    payload
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| ApplyError::InvalidPayload {
            entity,
            op,
            reason: format!("payload.{key} missing or not a string"),
        })
}

/// Extract an optional string with explicit type checking.
///
/// Returns:
/// - `Ok(None)` when the key is absent OR the value is `null` —
///   both are valid "clear this field" signals from the client.
/// - `Ok(Some(s))` when the value is a string.
/// - `Err(InvalidPayload)` when the value is present but not a
///   string or null (e.g. a number, an object). Silently coercing
///   those to `None` would mask a desktop bug as a clear-the-field
///   on the server, so we reject them at the boundary.
fn payload_optional_string(
    entity: &'static str,
    op: &'static str,
    payload: Option<&Value>,
    key: &str,
) -> Result<Option<String>, ApplyError> {
    let Some(value) = payload.and_then(|v| v.get(key)) else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::String(s) => Ok(Some(s.clone())),
        other => Err(ApplyError::InvalidPayload {
            entity,
            op,
            reason: format!(
                "payload.{key} expected string or null, got {}",
                json_kind(other)
            ),
        }),
    }
}

/// Human-readable name of a JSON value's kind for error messages.
fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Extract an i64 from `payload.value`. Used by rating ops.
fn payload_i64(
    entity: &'static str,
    op: &'static str,
    payload: Option<&Value>,
) -> Result<i64, ApplyError> {
    payload
        .and_then(|v| v.get("value"))
        .and_then(Value::as_i64)
        .ok_or_else(|| ApplyError::InvalidPayload {
            entity,
            op,
            reason: "payload.value missing or not an integer".to_owned(),
        })
}

// ---------------------------------------------------------------
// Playlist handlers.
// ---------------------------------------------------------------

mod playlist {
    use serde_json::Value;
    use sqlx::PgConnection;

    use crate::db::playlist_track::TrackSnapshot;
    use crate::sync::SyncOpIn;

    use super::{payload_optional_string, payload_string, ApplyError, ApplyOutcome, OpStamp};

    const ENTITY: &str = "playlist";

    pub async fn apply(
        conn: &mut PgConnection,
        profile_id: i64,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        let canonical_id = op.entity_id.as_str();

        match (op.op.as_str(), op.field.as_deref()) {
            // Whole-entity create.
            ("insert", None) => insert(conn, profile_id, canonical_id, op, now, stamp).await,
            // Whole-entity delete.
            ("delete", None) => delete(conn, profile_id, canonical_id).await,
            // Partial scalar update (name / description / color_id / icon_id).
            ("set", Some(field @ ("name" | "description" | "color_id" | "icon_id"))) => {
                set_field(conn, profile_id, canonical_id, field, op, now, stamp).await
            }
            // Track-list mutations (Phase 1.j.a). Desktop emits:
            // - `insert tracks` with `payload.track_ids: [N, …]`
            //   (optionally `payload.snapshots: { "<id_str>": { title, artist?, duration_ms? } }`
            //   from 1.j.b onward).
            // - `delete tracks` with `payload.track_ids: [N, …]`.
            // - `set tracks` with `payload: { track_id: N, position: M }`
            //   for a single reorder.
            //
            // `track_id` is the source desktop's local-i64 id, NOT a
            // server canonical reference. Snapshots are what makes a
            // row visible in the public share preview; ops without
            // them are stored but filtered out of the public read.
            ("insert", Some("tracks")) => {
                insert_tracks(conn, profile_id, canonical_id, op, now).await
            }
            ("delete", Some("tracks")) => delete_tracks(conn, profile_id, canonical_id, op).await,
            ("set", Some("tracks")) => reorder_track(conn, profile_id, canonical_id, op).await,
            // Anything else with a `field` we don't know about
            // surfaces as Unknown so telemetry can spot a missed
            // protocol extension.
            _ => Ok(ApplyOutcome::Unknown),
        }
    }

    async fn insert(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        let name = payload_string(ENTITY, "insert", op.payload.as_ref(), "name")?;
        let description =
            payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "description")?;
        let color_id = payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "color_id")?
            .unwrap_or_else(|| "violet".to_owned());
        let icon_id = payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "icon_id")?
            .unwrap_or_else(|| "music".to_owned());

        // ON CONFLICT (profile_id, canonical_id) DO NOTHING — the
        // partial unique index from the migration covers this. A
        // retry of the same insert is a no-op rather than an error.
        sqlx::query(
            "INSERT INTO playlist \
                (profile_id, canonical_id, name, description, color_id, icon_id, created_at, updated_at, hlc_wall, hlc_logical, origin_device_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $8, $9, $10) \
             ON CONFLICT (profile_id, canonical_id) WHERE canonical_id IS NOT NULL DO NOTHING",
        )
        .bind(profile_id)
        .bind(canonical_id)
        .bind(name)
        .bind(description)
        .bind(color_id)
        .bind(icon_id)
        .bind(now)
        .bind(stamp.hlc.wall)
        .bind(stamp.hlc.logical)
        .bind(stamp.origin_device_id)
        .execute(&mut *conn)
        .await?;

        Ok(ApplyOutcome::Applied)
    }

    async fn delete(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
    ) -> Result<ApplyOutcome, ApplyError> {
        sqlx::query("DELETE FROM playlist WHERE profile_id = $1 AND canonical_id = $2")
            .bind(profile_id)
            .bind(canonical_id)
            .execute(&mut *conn)
            .await?;
        Ok(ApplyOutcome::Applied)
    }

    /// Resolve a playlist canonical id to its server `playlist.id`.
    /// Returns `None` when the parent playlist hasn't been
    /// materialised yet — caller treats that as `Skipped` so the
    /// tracks op stays in the durable log for replay once the
    /// playlist's own insert lands. Tenant scoping is the
    /// `profile_id` already resolved by `apply_op`.
    async fn lookup_playlist_id(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
    ) -> Result<Option<i64>, ApplyError> {
        sqlx::query_scalar::<_, i64>(
            "SELECT id FROM playlist
              WHERE profile_id = $1 AND canonical_id = $2",
        )
        .bind(profile_id)
        .bind(canonical_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(Into::into)
    }

    async fn insert_tracks(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
        op: &SyncOpIn,
        now: i64,
    ) -> Result<ApplyOutcome, ApplyError> {
        let Some(playlist_id) = lookup_playlist_id(conn, profile_id, canonical_id).await? else {
            // Parent playlist not materialised yet — keep the op in
            // the log. The desktop only emits tracks ops after the
            // playlist's own insert, but a server-side ordering
            // hiccup or out-of-order pull could surface this.
            return Ok(ApplyOutcome::Skipped);
        };

        let track_ids = track_ids_from_payload(op)?;
        if track_ids.is_empty() {
            return Ok(ApplyOutcome::Applied);
        }

        let snapshots = snapshots_from_payload(op)?;
        let rows: Vec<(i64, Option<TrackSnapshot>)> = track_ids
            .iter()
            .map(|id| {
                let snapshot = snapshots.as_ref().and_then(|map| map.get(id).cloned());
                (*id, snapshot)
            })
            .collect();

        crate::db::playlist_track::append_tracks(conn, playlist_id, &rows, now).await?;
        Ok(ApplyOutcome::Applied)
    }

    async fn delete_tracks(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
        op: &SyncOpIn,
    ) -> Result<ApplyOutcome, ApplyError> {
        let Some(playlist_id) = lookup_playlist_id(conn, profile_id, canonical_id).await? else {
            return Ok(ApplyOutcome::Skipped);
        };
        let track_ids = track_ids_from_payload(op)?;
        if track_ids.is_empty() {
            return Ok(ApplyOutcome::Applied);
        }
        crate::db::playlist_track::remove_tracks(conn, playlist_id, &track_ids).await?;
        Ok(ApplyOutcome::Applied)
    }

    async fn reorder_track(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
        op: &SyncOpIn,
    ) -> Result<ApplyOutcome, ApplyError> {
        let Some(playlist_id) = lookup_playlist_id(conn, profile_id, canonical_id).await? else {
            return Ok(ApplyOutcome::Skipped);
        };
        let payload = op
            .payload
            .as_ref()
            .ok_or_else(|| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "set",
                reason: "tracks reorder payload missing (expected {track_id, position})".into(),
            })?;
        let track_id = payload
            .get("track_id")
            .and_then(Value::as_i64)
            .ok_or_else(|| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "set",
                reason: "payload.track_id missing or not an integer".into(),
            })?;
        let position = payload
            .get("position")
            .and_then(Value::as_i64)
            .ok_or_else(|| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "set",
                reason: "payload.position missing or not an integer".into(),
            })?;
        let position_i32: i32 =
            i32::try_from(position).map_err(|_| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "set",
                reason: format!("payload.position {position} out of i32 range"),
            })?;
        if position_i32 < 0 {
            return Err(ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "set",
                reason: format!("payload.position must be >= 0, got {position_i32}"),
            });
        }
        crate::db::playlist_track::set_position(conn, playlist_id, track_id, position_i32).await?;
        Ok(ApplyOutcome::Applied)
    }

    /// Extract `payload.track_ids: [N, …]` — required for both
    /// insert and delete. Same accept-only-integers shape the
    /// desktop's inbound parser uses (see desktop crate's apply.rs).
    fn track_ids_from_payload(op: &SyncOpIn) -> Result<Vec<i64>, ApplyError> {
        let payload = op
            .payload
            .as_ref()
            .ok_or_else(|| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "tracks",
                reason: "payload missing (expected {track_ids: [...]})".into(),
            })?;
        let arr = payload
            .get("track_ids")
            .and_then(Value::as_array)
            .ok_or_else(|| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "tracks",
                reason: "payload.track_ids missing or not an array".into(),
            })?;
        arr.iter()
            .map(|v| {
                v.as_i64().ok_or_else(|| ApplyError::InvalidPayload {
                    entity: ENTITY,
                    op: "tracks",
                    reason: "payload.track_ids must contain only integers".into(),
                })
            })
            .collect()
    }

    /// Optionally extract `payload.snapshots: { "<id_str>": { title, artist?, duration_ms? } }`.
    /// Returns `Ok(None)` when the field is absent (pre-1.j.b
    /// desktop) — the apply path then stores the rows with NULL
    /// snapshot fields, which keeps them out of the public share
    /// preview until a future desktop re-emits with the snapshot.
    ///
    /// Returns an error when the field is present but malformed —
    /// we prefer to reject a corrupt batch up front rather than
    /// store a mix of populated + NULL rows that would be hard to
    /// audit later.
    fn snapshots_from_payload(
        op: &SyncOpIn,
    ) -> Result<Option<std::collections::HashMap<i64, TrackSnapshot>>, ApplyError> {
        let Some(payload) = op.payload.as_ref() else {
            return Ok(None);
        };
        let Some(map) = payload.get("snapshots") else {
            return Ok(None);
        };
        if matches!(map, Value::Null) {
            return Ok(None);
        }
        let obj = map.as_object().ok_or_else(|| ApplyError::InvalidPayload {
            entity: ENTITY,
            op: "tracks",
            reason: "payload.snapshots must be an object keyed by track_id".into(),
        })?;
        let mut out = std::collections::HashMap::with_capacity(obj.len());
        for (key, value) in obj {
            let track_id: i64 = key.parse().map_err(|_| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "tracks",
                reason: format!("payload.snapshots key {key:?} is not an integer"),
            })?;
            let inner = value
                .as_object()
                .ok_or_else(|| ApplyError::InvalidPayload {
                    entity: ENTITY,
                    op: "tracks",
                    reason: format!("payload.snapshots[{key}] must be an object"),
                })?;
            let title = inner
                .get("title")
                .and_then(Value::as_str)
                .ok_or_else(|| ApplyError::InvalidPayload {
                    entity: ENTITY,
                    op: "tracks",
                    reason: format!("payload.snapshots[{key}].title missing or not a string"),
                })?
                .to_owned();
            let artist = inner
                .get("artist")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let duration_ms = inner
                .get("duration_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            out.insert(
                track_id,
                TrackSnapshot {
                    title,
                    artist,
                    duration_ms,
                },
            );
        }
        Ok(Some(out))
    }

    async fn set_field(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
        field: &str,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        // `description` is the only nullable column among the
        // four — distinguish so a `{ "value": null }` payload
        // explicitly clears it rather than failing the string
        // extraction.
        //
        // SET each scalar field alongside hlc + origin_device_id so
        // the row's §2 total-order tuple stays in sync with whatever
        // wrote the field.
        let sql_with_value = match field {
            "name" => "UPDATE playlist SET name = $1, updated_at = $2, hlc_wall = $5, hlc_logical = $6, origin_device_id = $7 WHERE profile_id = $3 AND canonical_id = $4",
            "color_id" => "UPDATE playlist SET color_id = $1, updated_at = $2, hlc_wall = $5, hlc_logical = $6, origin_device_id = $7 WHERE profile_id = $3 AND canonical_id = $4",
            "icon_id" => "UPDATE playlist SET icon_id = $1, updated_at = $2, hlc_wall = $5, hlc_logical = $6, origin_device_id = $7 WHERE profile_id = $3 AND canonical_id = $4",
            "description" => "UPDATE playlist SET description = $1, updated_at = $2, hlc_wall = $5, hlc_logical = $6, origin_device_id = $7 WHERE profile_id = $3 AND canonical_id = $4",
            _ => unreachable!("set_field caller already narrowed the field"),
        };

        if field == "description" {
            let value = payload_optional_string(ENTITY, "set", op.payload.as_ref(), "value")?;
            sqlx::query(sql_with_value)
                .bind(value)
                .bind(now)
                .bind(profile_id)
                .bind(canonical_id)
                .bind(stamp.hlc.wall)
                .bind(stamp.hlc.logical)
                .bind(stamp.origin_device_id)
                .execute(&mut *conn)
                .await?;
        } else {
            let value = payload_string(ENTITY, "set", op.payload.as_ref(), "value")?;
            sqlx::query(sql_with_value)
                .bind(value)
                .bind(now)
                .bind(profile_id)
                .bind(canonical_id)
                .bind(stamp.hlc.wall)
                .bind(stamp.hlc.logical)
                .bind(stamp.origin_device_id)
                .execute(&mut *conn)
                .await?;
        }

        Ok(ApplyOutcome::Applied)
    }
}

// ---------------------------------------------------------------
// Library handlers — mirror of playlist (same op shapes).
// ---------------------------------------------------------------

mod library {
    use sqlx::PgConnection;

    use crate::sync::SyncOpIn;

    use super::{payload_optional_string, payload_string, ApplyError, ApplyOutcome, OpStamp};

    const ENTITY: &str = "library";

    pub async fn apply(
        conn: &mut PgConnection,
        profile_id: i64,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        let canonical_id = op.entity_id.as_str();

        match (op.op.as_str(), op.field.as_deref()) {
            ("insert", None) => insert(conn, profile_id, canonical_id, op, now, stamp).await,
            ("delete", None) => delete(conn, profile_id, canonical_id).await,
            ("set", Some(field @ ("name" | "description" | "color_id" | "icon_id"))) => {
                set_field(conn, profile_id, canonical_id, field, op, now, stamp).await
            }
            _ => Ok(ApplyOutcome::Unknown),
        }
    }

    async fn insert(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        let name = payload_string(ENTITY, "insert", op.payload.as_ref(), "name")?;
        let description =
            payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "description")?;
        let color_id = payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "color_id")?
            .unwrap_or_else(|| "emerald".to_owned());
        let icon_id = payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "icon_id")?
            .unwrap_or_else(|| "library".to_owned());

        sqlx::query(
            "INSERT INTO library \
                (profile_id, canonical_id, name, description, color_id, icon_id, created_at, updated_at, hlc_wall, hlc_logical, origin_device_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $8, $9, $10) \
             ON CONFLICT (profile_id, canonical_id) WHERE canonical_id IS NOT NULL DO NOTHING",
        )
        .bind(profile_id)
        .bind(canonical_id)
        .bind(name)
        .bind(description)
        .bind(color_id)
        .bind(icon_id)
        .bind(now)
        .bind(stamp.hlc.wall)
        .bind(stamp.hlc.logical)
        .bind(stamp.origin_device_id)
        .execute(&mut *conn)
        .await?;

        Ok(ApplyOutcome::Applied)
    }

    async fn delete(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
    ) -> Result<ApplyOutcome, ApplyError> {
        sqlx::query("DELETE FROM library WHERE profile_id = $1 AND canonical_id = $2")
            .bind(profile_id)
            .bind(canonical_id)
            .execute(&mut *conn)
            .await?;
        Ok(ApplyOutcome::Applied)
    }

    async fn set_field(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
        field: &str,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        // SET each scalar field alongside hlc + origin_device_id so
        // the row's §2 total-order tuple stays in sync with whatever
        // wrote the field. payload_hash + metadata_digest_version
        // bump are deferred to A.2.3 (the digest endpoint that
        // actually consumes them).
        let sql = match field {
            "name" => "UPDATE library SET name = $1, updated_at = $2, hlc_wall = $5, hlc_logical = $6, origin_device_id = $7 WHERE profile_id = $3 AND canonical_id = $4",
            "color_id" => "UPDATE library SET color_id = $1, updated_at = $2, hlc_wall = $5, hlc_logical = $6, origin_device_id = $7 WHERE profile_id = $3 AND canonical_id = $4",
            "icon_id" => "UPDATE library SET icon_id = $1, updated_at = $2, hlc_wall = $5, hlc_logical = $6, origin_device_id = $7 WHERE profile_id = $3 AND canonical_id = $4",
            "description" => "UPDATE library SET description = $1, updated_at = $2, hlc_wall = $5, hlc_logical = $6, origin_device_id = $7 WHERE profile_id = $3 AND canonical_id = $4",
            _ => unreachable!("set_field caller already narrowed the field"),
        };

        if field == "description" {
            let value = payload_optional_string(ENTITY, "set", op.payload.as_ref(), "value")?;
            sqlx::query(sql)
                .bind(value)
                .bind(now)
                .bind(profile_id)
                .bind(canonical_id)
                .bind(stamp.hlc.wall)
                .bind(stamp.hlc.logical)
                .bind(stamp.origin_device_id)
                .execute(&mut *conn)
                .await?;
        } else {
            let value = payload_string(ENTITY, "set", op.payload.as_ref(), "value")?;
            sqlx::query(sql)
                .bind(value)
                .bind(now)
                .bind(profile_id)
                .bind(canonical_id)
                .bind(stamp.hlc.wall)
                .bind(stamp.hlc.logical)
                .bind(stamp.origin_device_id)
                .execute(&mut *conn)
                .await?;
        }

        Ok(ApplyOutcome::Applied)
    }
}

// ---------------------------------------------------------------
// liked_track — keyed on (user_id, file_hash).
// ---------------------------------------------------------------

mod liked {
    use sqlx::PgConnection;

    use crate::sync::SyncOpIn;

    use super::{ApplyError, ApplyOutcome, OpStamp};

    pub async fn apply(
        conn: &mut PgConnection,
        user_id: i64,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        // `entity_id` IS the file_hash for like / rating ops —
        // tracks have no canonical_id because the audio content
        // itself is the cross-device identity.
        let file_hash = op.entity_id.as_str();

        match (op.op.as_str(), op.field.as_deref()) {
            ("insert", None) => {
                // UPSERT path mirrors `rating::set` — on conflict
                // refresh the row's §2 total-order tuple AND the
                // `liked_at` timestamp so the materialised row
                // reflects the latest winning op, not the first one
                // that landed. Without this the digest endpoint
                // (A.2.3) would see two replicas converge on
                // different HLCs whenever both devices like the
                // same file.
                sqlx::query(
                    "INSERT INTO user_liked_track \
                        (user_id, file_hash, liked_at, hlc_wall, hlc_logical, origin_device_id) \
                     VALUES ($1, $2, $3, $4, $5, $6) \
                     ON CONFLICT (user_id, file_hash) DO UPDATE \
                         SET liked_at = EXCLUDED.liked_at, \
                             hlc_wall = EXCLUDED.hlc_wall, \
                             hlc_logical = EXCLUDED.hlc_logical, \
                             origin_device_id = EXCLUDED.origin_device_id",
                )
                .bind(user_id)
                .bind(file_hash)
                .bind(now)
                .bind(stamp.hlc.wall)
                .bind(stamp.hlc.logical)
                .bind(stamp.origin_device_id)
                .execute(&mut *conn)
                .await?;
                Ok(ApplyOutcome::Applied)
            }
            ("delete", None) => {
                sqlx::query("DELETE FROM user_liked_track WHERE user_id = $1 AND file_hash = $2")
                    .bind(user_id)
                    .bind(file_hash)
                    .execute(&mut *conn)
                    .await?;
                Ok(ApplyOutcome::Applied)
            }
            _ => Ok(ApplyOutcome::Unknown),
        }
    }
}

// ---------------------------------------------------------------
// track_rating — keyed on (user_id, file_hash).
// ---------------------------------------------------------------

mod rating {
    use sqlx::PgConnection;

    use crate::sync::SyncOpIn;

    use super::{payload_i64, ApplyError, ApplyOutcome, OpStamp};

    const ENTITY: &str = "track_rating";

    pub async fn apply(
        conn: &mut PgConnection,
        user_id: i64,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        let file_hash = op.entity_id.as_str();

        match (op.op.as_str(), op.field.as_deref()) {
            ("set", None) => {
                let value = payload_i64(ENTITY, "set", op.payload.as_ref())?;
                if !(0..=255).contains(&value) {
                    return Err(ApplyError::InvalidPayload {
                        entity: ENTITY,
                        op: "set",
                        reason: format!("rating {value} out of 0..=255 POPM range"),
                    });
                }
                // UPSERT so a later op for the same file replaces
                // the rating instead of inserting a duplicate row.
                // SET hlc + origin_device_id on the UPDATE path too
                // so the row's §2 total-order tuple reflects the
                // latest op, not the first one that landed.
                sqlx::query(
                    "INSERT INTO user_track_rating \
                        (user_id, file_hash, rating, updated_at, hlc_wall, hlc_logical, origin_device_id) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7) \
                     ON CONFLICT (user_id, file_hash) DO UPDATE \
                         SET rating = EXCLUDED.rating, \
                             updated_at = EXCLUDED.updated_at, \
                             hlc_wall = EXCLUDED.hlc_wall, \
                             hlc_logical = EXCLUDED.hlc_logical, \
                             origin_device_id = EXCLUDED.origin_device_id",
                )
                .bind(user_id)
                .bind(file_hash)
                .bind(value)
                .bind(now)
                .bind(stamp.hlc.wall)
                .bind(stamp.hlc.logical)
                .bind(stamp.origin_device_id)
                .execute(&mut *conn)
                .await?;
                Ok(ApplyOutcome::Applied)
            }
            ("delete", None) => {
                sqlx::query("DELETE FROM user_track_rating WHERE user_id = $1 AND file_hash = $2")
                    .bind(user_id)
                    .bind(file_hash)
                    .execute(&mut *conn)
                    .await?;
                Ok(ApplyOutcome::Applied)
            }
            _ => Ok(ApplyOutcome::Unknown),
        }
    }
}

// ---------------------------------------------------------------
// track — keyed on (library_id, file_path). Phase 4.d.0.2.
// ---------------------------------------------------------------
//
// Wire shape:
// - `entity: "track"`, `entity_id: <file_path>` — the per-library
//   natural identity (`UNIQUE (library_id, file_path)` from
//   `20260530000003_track.sql:64`). The BLAKE3 hash rides as a
//   payload field; using it as `entity_id` would break the
//   tag-edit upsert (lofty rewrites embedded metadata frames so
//   the hash changes while the path doesn't — re-emit would land
//   as INSERT, trip the `(library_id, file_path)` UNIQUE, and 500).
//   The cross-device-content identity from
//   `20260604000000_apply_pipeline.sql:26-33` still lives in the
//   `track.file_hash` column for the liked_track / rating joins;
//   it just isn't the row identity for the `track` entity itself.
// - `profile_canonical_id`: required (the dispatcher gates on it).
// - `payload.library_canonical_id`: required. The library is the
//   tenant scope — resolved per profile, Skipped if not yet
//   materialised.
// - INSERT payload carries the full track metadata — `file_hash`,
//   `title`, `file_size`, `duration_ms`, the optional audio specs,
//   the optional `album_title` / `album_artist_name` /
//   `is_compilation` / `artists: [String, ...]` fields. Multi-
//   artist position is the array index — the desktop emits its
//   `; `-split list.
// - DELETE payload carries only `library_canonical_id`. The
//   file_path sits in `entity_id`.
//
// `set` is intentionally Unknown for tracks today: the desktop's
// tag-editor save rewrites the audio file and re-emits a full
// INSERT (the path is unchanged, the hash + tag values aren't).
// The upsert handles re-emit as a merge — every scalar column
// overwrites on conflict.
mod track {
    use serde_json::Value;
    use sqlx::PgConnection;

    use crate::db::track_sync::{
        delete_track, lookup_library_id, replace_track_artists, upsert_album, upsert_artist,
        upsert_track, ArtistLinkInput, TrackInput,
    };
    use crate::sync::SyncOpIn;

    use super::{payload_optional_string, payload_string, ApplyError, ApplyOutcome, OpStamp};

    const ENTITY: &str = "track";

    pub async fn apply(
        conn: &mut PgConnection,
        profile_id: i64,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        // entity_id IS the file_path for tracks — see module
        // banner for the design rationale.
        let file_path = op.entity_id.as_str();
        match (op.op.as_str(), op.field.as_deref()) {
            ("insert", None) => insert(conn, profile_id, file_path, op, now, stamp).await,
            ("delete", None) => delete(conn, profile_id, file_path, op).await,
            // `set` is reserved for a future incremental-edit flow.
            // The desktop's current tag-editor saves rewrite the
            // file (and re-emit INSERT), so `set` would be
            // unreachable today — surfacing it as Unknown keeps the
            // door open without committing to a wire shape.
            _ => Ok(ApplyOutcome::Unknown),
        }
    }

    async fn insert(
        conn: &mut PgConnection,
        profile_id: i64,
        file_path: &str,
        op: &SyncOpIn,
        now: i64,
        stamp: OpStamp,
    ) -> Result<ApplyOutcome, ApplyError> {
        // Library is the tenant scope for the track. A missing
        // library canonical id means the desktop bug-emitted an
        // op without it — surface as InvalidPayload rather than
        // Skipped so the push handler rolls back the durable
        // insert (the op is structurally broken, not just
        // out-of-order).
        let library_canonical_id = payload_string(
            ENTITY,
            "insert",
            op.payload.as_ref(),
            "library_canonical_id",
        )?;

        // Library not yet materialised → Skipped. The op stays in
        // the durable log; the next replay after the library's
        // own insert lands will apply it.
        let Some(library_id) = lookup_library_id(conn, profile_id, &library_canonical_id).await?
        else {
            return Ok(ApplyOutcome::Skipped);
        };

        let title = payload_string(ENTITY, "insert", op.payload.as_ref(), "title")?;
        let file_hash = payload_string(ENTITY, "insert", op.payload.as_ref(), "file_hash")?;
        let file_size = payload_i64_required(op, "file_size")?;
        let duration_ms = payload_i64_required(op, "duration_ms")?;
        let track_number = payload_i64_optional(op, "track_number")?;
        let disc_number = payload_i64_optional(op, "disc_number")?;
        let year = payload_i64_optional(op, "year")?;
        let bitrate = payload_i64_optional(op, "bitrate")?;
        let sample_rate = payload_i64_optional(op, "sample_rate")?;
        let channels = payload_i64_optional(op, "channels")?;
        let bit_depth = payload_i64_optional(op, "bit_depth")?;
        let codec = payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "codec")?;
        let musical_key =
            payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "musical_key")?;
        let added_at = payload_i64_optional(op, "added_at")?.unwrap_or(now);

        // Album metadata. `album_title` is required to mint an
        // album row. `album_artist_name` is optional — absent
        // means compilation (NULL album_artist_id in the natural
        // key). `is_compilation` is sticky — once true on the row,
        // a re-emit with false leaves it true (handled in
        // `upsert_album`).
        //
        // Reject empty strings here rather than letting them flow
        // to the `length(...) > 0` CHECK constraints on `album`
        // and `artist` — the constraint trip would surface as a
        // 500 (DB error), but an empty string is a structural
        // payload bug that deserves a 400 with a clear reason.
        let album_title = reject_empty(
            payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "album_title")?,
            "album_title",
        )?;
        let album_artist_name = reject_empty(
            payload_optional_string(ENTITY, "insert", op.payload.as_ref(), "album_artist_name")?,
            "album_artist_name",
        )?;
        let is_compilation = payload_optional_bool(op, "is_compilation")?.unwrap_or(false);

        // Multi-artist array. `position` is the array index — the
        // desktop emits its `; `-split list in source order.
        let artists = artists_from_payload(op)?;

        // 1. Upsert every contributor artist (collect their ids).
        let mut artist_links: Vec<ArtistLinkInput> = Vec::with_capacity(artists.len());
        for (position, name) in artists.iter().enumerate() {
            artist_links.push(ArtistLinkInput {
                name: name.clone(),
                position: position as i32,
            });
        }
        let mut link_ids: Vec<(i64, i32)> = Vec::with_capacity(artist_links.len());
        for link in &artist_links {
            let id = upsert_artist(conn, library_id, &link.name, now).await?;
            link_ids.push((id, link.position));
        }

        // 2. Resolve the album artist id (optional). If the album
        // artist isn't already in `artists`, mint it via the same
        // upsert so the album FK has something to reference.
        let album_artist_id = match album_artist_name.as_deref() {
            Some(name) => {
                // Re-use an already-upserted artist if the album
                // artist matches one of the contributors.
                let existing = artist_links
                    .iter()
                    .zip(link_ids.iter())
                    .find_map(|(link, (id, _))| if link.name == name { Some(*id) } else { None });
                match existing {
                    Some(id) => Some(id),
                    None => Some(upsert_artist(conn, library_id, name, now).await?),
                }
            }
            None => None,
        };

        // 3. Upsert the album row if a title is present.
        let album_id = match album_title.as_deref() {
            Some(title) => Some(
                upsert_album(
                    conn,
                    library_id,
                    title,
                    album_artist_id,
                    year,
                    is_compilation,
                    now,
                )
                .await?,
            ),
            None => None,
        };

        // 4. Upsert the track row.
        let input = TrackInput {
            library_id,
            file_hash: &file_hash,
            title: &title,
            file_path,
            file_size,
            duration_ms,
            track_number,
            disc_number,
            year,
            bitrate,
            sample_rate,
            channels,
            bit_depth,
            codec: codec.as_deref(),
            musical_key: musical_key.as_deref(),
            added_at,
            album_id,
            hlc_wall: stamp.hlc.wall,
            hlc_logical: stamp.hlc.logical,
            origin_device_id: stamp.origin_device_id,
        };
        let track_id = upsert_track(conn, &input).await?;

        // 5. Replace the multi-artist link rows for this track.
        replace_track_artists(conn, track_id, library_id, &link_ids).await?;

        Ok(ApplyOutcome::Applied)
    }

    async fn delete(
        conn: &mut PgConnection,
        profile_id: i64,
        file_path: &str,
        op: &SyncOpIn,
    ) -> Result<ApplyOutcome, ApplyError> {
        let library_canonical_id = payload_string(
            ENTITY,
            "delete",
            op.payload.as_ref(),
            "library_canonical_id",
        )?;
        let Some(library_id) = lookup_library_id(conn, profile_id, &library_canonical_id).await?
        else {
            return Ok(ApplyOutcome::Skipped);
        };
        delete_track(conn, library_id, file_path).await?;
        Ok(ApplyOutcome::Applied)
    }

    /// Reject empty-string optional payload fields. The schema
    /// CHECK constraints (`length(...) > 0` on `album.canonical_title`
    /// and `artist.name`) would surface an empty string as a 500
    /// rather than a 400, so the apply layer catches it first.
    fn reject_empty(value: Option<String>, key: &str) -> Result<Option<String>, ApplyError> {
        if let Some(ref s) = value {
            if s.is_empty() {
                return Err(ApplyError::InvalidPayload {
                    entity: ENTITY,
                    op: "insert",
                    reason: format!("payload.{key} must not be empty"),
                });
            }
        }
        Ok(value)
    }

    /// Extract `payload.artists: [String, ...]`. Absent / null /
    /// empty array all collapse to an empty Vec — the desktop
    /// emits an empty list for tracks without an artist tag, and
    /// the apply path must NOT reject those as InvalidPayload.
    ///
    /// Deduplicates first-seen-order: a desktop that ships
    /// `["A", "A"]` (e.g., a tag with a duplicated artist after
    /// the `";"` split) collapses to `["A"]` here. Without this,
    /// `upsert_artist` returns the same id for both entries and
    /// `replace_track_artists` trips the `(track_id, artist_id)`
    /// PK on the second INSERT.
    fn artists_from_payload(op: &SyncOpIn) -> Result<Vec<String>, ApplyError> {
        let Some(payload) = op.payload.as_ref() else {
            return Ok(Vec::new());
        };
        let Some(value) = payload.get("artists") else {
            return Ok(Vec::new());
        };
        if matches!(value, Value::Null) {
            return Ok(Vec::new());
        }
        let arr = value.as_array().ok_or_else(|| ApplyError::InvalidPayload {
            entity: ENTITY,
            op: "insert",
            reason: "payload.artists must be an array of strings".into(),
        })?;
        let mut seen: std::collections::HashSet<String> =
            std::collections::HashSet::with_capacity(arr.len());
        let mut out = Vec::with_capacity(arr.len());
        for (idx, entry) in arr.iter().enumerate() {
            let name = entry.as_str().ok_or_else(|| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "insert",
                reason: format!("payload.artists[{idx}] must be a string"),
            })?;
            if name.is_empty() {
                return Err(ApplyError::InvalidPayload {
                    entity: ENTITY,
                    op: "insert",
                    reason: format!("payload.artists[{idx}] must not be empty"),
                });
            }
            if seen.insert(name.to_owned()) {
                out.push(name.to_owned());
            }
            // else: duplicate — silently dropped (first-seen wins).
        }
        Ok(out)
    }

    fn payload_i64_required(op: &SyncOpIn, key: &str) -> Result<i64, ApplyError> {
        op.payload
            .as_ref()
            .and_then(|p| p.get(key))
            .and_then(Value::as_i64)
            .ok_or_else(|| ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "insert",
                reason: format!("payload.{key} missing or not an integer"),
            })
    }

    fn payload_i64_optional(op: &SyncOpIn, key: &str) -> Result<Option<i64>, ApplyError> {
        let Some(payload) = op.payload.as_ref() else {
            return Ok(None);
        };
        let Some(value) = payload.get(key) else {
            return Ok(None);
        };
        match value {
            Value::Null => Ok(None),
            Value::Number(_) => {
                value
                    .as_i64()
                    .map(Some)
                    .ok_or_else(|| ApplyError::InvalidPayload {
                        entity: ENTITY,
                        op: "insert",
                        reason: format!("payload.{key} must fit in i64"),
                    })
            }
            _ => Err(ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "insert",
                reason: format!("payload.{key} must be an integer or null"),
            }),
        }
    }

    fn payload_optional_bool(op: &SyncOpIn, key: &str) -> Result<Option<bool>, ApplyError> {
        let Some(payload) = op.payload.as_ref() else {
            return Ok(None);
        };
        let Some(value) = payload.get(key) else {
            return Ok(None);
        };
        match value {
            Value::Null => Ok(None),
            Value::Bool(b) => Ok(Some(*b)),
            _ => Err(ApplyError::InvalidPayload {
                entity: ENTITY,
                op: "insert",
                reason: format!("payload.{key} must be a boolean or null"),
            }),
        }
    }
}
