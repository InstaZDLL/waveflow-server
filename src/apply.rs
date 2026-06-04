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

use crate::sync::SyncOpIn;

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
pub async fn apply_op(
    conn: &mut PgConnection,
    user_id: i64,
    op: &SyncOpIn,
    created_at: i64,
) -> Result<ApplyOutcome, ApplyError> {
    let entity = op.entity.as_str();

    // Profile routing prerequisite — every entity below is profile-
    // scoped except `liked_track` / `track_rating`, which key on
    // `(user_id, file_hash)` and skip the profile lookup. The
    // resolution still runs for the profile-scoped entities so a
    // missing canonical id surfaces as `Skipped` rather than a
    // silent partial apply.
    match entity {
        "playlist" | "library" => {
            let Some(profile_canonical) = op.profile_canonical_id.as_deref() else {
                tracing::debug!(
                    entity = entity,
                    "apply: missing profile_canonical_id, skipping"
                );
                return Ok(ApplyOutcome::Skipped);
            };
            let profile_id =
                profile_resolve::find_or_provision(conn, user_id, profile_canonical, created_at)
                    .await?;
            match entity {
                "playlist" => playlist::apply(conn, profile_id, op, created_at).await,
                "library" => library::apply(conn, profile_id, op, created_at).await,
                _ => unreachable!(),
            }
        }
        "liked_track" => liked::apply(conn, user_id, op, created_at).await,
        "track_rating" => rating::apply(conn, user_id, op, created_at).await,
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

    use super::ApplyError;

    /// Resolve a profile canonical id to a server `profile.id`.
    /// Auto-provisions the row if it's the first op for that
    /// canonical id. The placeholder name / color match the
    /// desktop's default-profile palette so the row is usable even
    /// before any "set name" / "set color" ops land.
    ///
    /// Read-first / write-on-miss, same shape as
    /// `users::find_or_provision_by_external_id` — the common path
    /// (every op for an already-synced profile) is a single SELECT.
    pub async fn find_or_provision(
        conn: &mut PgConnection,
        user_id: i64,
        canonical_id: &str,
        now: i64,
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
            "INSERT INTO profile (user_id, canonical_id, name, color_id, data_dir, created_at, last_used_at) \
             VALUES ($1, $2, $3, $4, '', $5, $5) \
             ON CONFLICT (user_id, canonical_id) WHERE canonical_id IS NOT NULL \
                 DO UPDATE SET canonical_id = EXCLUDED.canonical_id \
             RETURNING id",
        )
        .bind(user_id)
        .bind(canonical_id)
        .bind("Synced profile")
        .bind("violet")
        .bind(now)
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

/// Extract an optional string. Returns `Ok(None)` when the key is
/// absent or explicitly null.
fn payload_optional_string(payload: Option<&Value>, key: &str) -> Option<String> {
    payload
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
        .map(str::to_owned)
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
    use sqlx::PgConnection;

    use crate::sync::SyncOpIn;

    use super::{payload_optional_string, payload_string, ApplyError, ApplyOutcome};

    const ENTITY: &str = "playlist";

    pub async fn apply(
        conn: &mut PgConnection,
        profile_id: i64,
        op: &SyncOpIn,
        now: i64,
    ) -> Result<ApplyOutcome, ApplyError> {
        let canonical_id = op.entity_id.as_str();

        match (op.op.as_str(), op.field.as_deref()) {
            // Whole-entity create.
            ("insert", None) => insert(conn, profile_id, canonical_id, op, now).await,
            // Whole-entity delete.
            ("delete", None) => delete(conn, profile_id, canonical_id).await,
            // Partial scalar update (name / description / color_id / icon_id).
            ("set", Some(field @ ("name" | "description" | "color_id" | "icon_id"))) => {
                set_field(conn, profile_id, canonical_id, field, op, now).await
            }
            // `tracks` ops are recognised but unsupported until
            // desktop emits file_hash references instead of local
            // BIGINT track ids. See migration doc.
            (_, Some("tracks")) => Ok(ApplyOutcome::Skipped),
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
    ) -> Result<ApplyOutcome, ApplyError> {
        let name = payload_string(ENTITY, "insert", op.payload.as_ref(), "name")?;
        let description = payload_optional_string(op.payload.as_ref(), "description");
        let color_id = payload_optional_string(op.payload.as_ref(), "color_id")
            .unwrap_or_else(|| "violet".to_owned());
        let icon_id = payload_optional_string(op.payload.as_ref(), "icon_id")
            .unwrap_or_else(|| "music".to_owned());

        // ON CONFLICT (profile_id, canonical_id) DO NOTHING — the
        // partial unique index from the migration covers this. A
        // retry of the same insert is a no-op rather than an error.
        sqlx::query(
            "INSERT INTO playlist \
                (profile_id, canonical_id, name, description, color_id, icon_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7) \
             ON CONFLICT (profile_id, canonical_id) WHERE canonical_id IS NOT NULL DO NOTHING",
        )
        .bind(profile_id)
        .bind(canonical_id)
        .bind(name)
        .bind(description)
        .bind(color_id)
        .bind(icon_id)
        .bind(now)
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

    async fn set_field(
        conn: &mut PgConnection,
        profile_id: i64,
        canonical_id: &str,
        field: &str,
        op: &SyncOpIn,
        now: i64,
    ) -> Result<ApplyOutcome, ApplyError> {
        // `description` is the only nullable column among the
        // four — distinguish so a `{ "value": null }` payload
        // explicitly clears it rather than failing the string
        // extraction.
        let sql_with_value = match field {
            "name" => "UPDATE playlist SET name = $1, updated_at = $2 WHERE profile_id = $3 AND canonical_id = $4",
            "color_id" => "UPDATE playlist SET color_id = $1, updated_at = $2 WHERE profile_id = $3 AND canonical_id = $4",
            "icon_id" => "UPDATE playlist SET icon_id = $1, updated_at = $2 WHERE profile_id = $3 AND canonical_id = $4",
            "description" => "UPDATE playlist SET description = $1, updated_at = $2 WHERE profile_id = $3 AND canonical_id = $4",
            _ => unreachable!("set_field caller already narrowed the field"),
        };

        if field == "description" {
            let value = payload_optional_string(op.payload.as_ref(), "value");
            sqlx::query(sql_with_value)
                .bind(value)
                .bind(now)
                .bind(profile_id)
                .bind(canonical_id)
                .execute(&mut *conn)
                .await?;
        } else {
            let value = payload_string(ENTITY, "set", op.payload.as_ref(), "value")?;
            sqlx::query(sql_with_value)
                .bind(value)
                .bind(now)
                .bind(profile_id)
                .bind(canonical_id)
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

    use super::{payload_optional_string, payload_string, ApplyError, ApplyOutcome};

    const ENTITY: &str = "library";

    pub async fn apply(
        conn: &mut PgConnection,
        profile_id: i64,
        op: &SyncOpIn,
        now: i64,
    ) -> Result<ApplyOutcome, ApplyError> {
        let canonical_id = op.entity_id.as_str();

        match (op.op.as_str(), op.field.as_deref()) {
            ("insert", None) => insert(conn, profile_id, canonical_id, op, now).await,
            ("delete", None) => delete(conn, profile_id, canonical_id).await,
            ("set", Some(field @ ("name" | "description" | "color_id" | "icon_id"))) => {
                set_field(conn, profile_id, canonical_id, field, op, now).await
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
    ) -> Result<ApplyOutcome, ApplyError> {
        let name = payload_string(ENTITY, "insert", op.payload.as_ref(), "name")?;
        let description = payload_optional_string(op.payload.as_ref(), "description");
        let color_id = payload_optional_string(op.payload.as_ref(), "color_id")
            .unwrap_or_else(|| "emerald".to_owned());
        let icon_id = payload_optional_string(op.payload.as_ref(), "icon_id")
            .unwrap_or_else(|| "library".to_owned());

        sqlx::query(
            "INSERT INTO library \
                (profile_id, canonical_id, name, description, color_id, icon_id, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7) \
             ON CONFLICT (profile_id, canonical_id) WHERE canonical_id IS NOT NULL DO NOTHING",
        )
        .bind(profile_id)
        .bind(canonical_id)
        .bind(name)
        .bind(description)
        .bind(color_id)
        .bind(icon_id)
        .bind(now)
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
    ) -> Result<ApplyOutcome, ApplyError> {
        let sql = match field {
            "name" => "UPDATE library SET name = $1, updated_at = $2 WHERE profile_id = $3 AND canonical_id = $4",
            "color_id" => "UPDATE library SET color_id = $1, updated_at = $2 WHERE profile_id = $3 AND canonical_id = $4",
            "icon_id" => "UPDATE library SET icon_id = $1, updated_at = $2 WHERE profile_id = $3 AND canonical_id = $4",
            "description" => "UPDATE library SET description = $1, updated_at = $2 WHERE profile_id = $3 AND canonical_id = $4",
            _ => unreachable!("set_field caller already narrowed the field"),
        };

        if field == "description" {
            let value = payload_optional_string(op.payload.as_ref(), "value");
            sqlx::query(sql)
                .bind(value)
                .bind(now)
                .bind(profile_id)
                .bind(canonical_id)
                .execute(&mut *conn)
                .await?;
        } else {
            let value = payload_string(ENTITY, "set", op.payload.as_ref(), "value")?;
            sqlx::query(sql)
                .bind(value)
                .bind(now)
                .bind(profile_id)
                .bind(canonical_id)
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

    use super::{ApplyError, ApplyOutcome};

    pub async fn apply(
        conn: &mut PgConnection,
        user_id: i64,
        op: &SyncOpIn,
        now: i64,
    ) -> Result<ApplyOutcome, ApplyError> {
        // `entity_id` IS the file_hash for like / rating ops —
        // tracks have no canonical_id because the audio content
        // itself is the cross-device identity.
        let file_hash = op.entity_id.as_str();

        match (op.op.as_str(), op.field.as_deref()) {
            ("insert", None) => {
                sqlx::query(
                    "INSERT INTO user_liked_track (user_id, file_hash, liked_at) \
                     VALUES ($1, $2, $3) \
                     ON CONFLICT (user_id, file_hash) DO NOTHING",
                )
                .bind(user_id)
                .bind(file_hash)
                .bind(now)
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

    use super::{payload_i64, ApplyError, ApplyOutcome};

    const ENTITY: &str = "track_rating";

    pub async fn apply(
        conn: &mut PgConnection,
        user_id: i64,
        op: &SyncOpIn,
        now: i64,
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
                sqlx::query(
                    "INSERT INTO user_track_rating (user_id, file_hash, rating, updated_at) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (user_id, file_hash) DO UPDATE \
                         SET rating = EXCLUDED.rating, updated_at = EXCLUDED.updated_at",
                )
                .bind(user_id)
                .bind(file_hash)
                .bind(value)
                .bind(now)
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
