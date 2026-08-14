//! Durable user-data change journal for WaveFlow Desktop synchronization.
//!
//! REST is the source of truth. The WebSocket only wakes a client when a newer
//! cursor exists, so reconnects and dropped frames cannot lose state.

use serde::Serialize;
use serde_json::Value;
use sqlx::{Row, SqliteConnection};
use tokio::sync::{broadcast, OwnedMutexGuard};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    authentication::now_ms,
    database::{Database, SyncOperationReservation},
};

pub const DEFAULT_SYNC_LIMIT: i64 = 100;
pub const MAX_SYNC_LIMIT: i64 = 500;

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("invalid synchronization input")]
    Invalid,
    #[error("operation id was already used for another intent")]
    Conflict,
    /// The caller resumes from a point the journal no longer covers, so the
    /// gap cannot be replayed incrementally. The client must take a fresh
    /// snapshot and continue from its cursor.
    ///
    /// Unreachable while the journal stays append-only, which is the case in
    /// v2.0. It exists so clients can implement the recovery branch now rather
    /// than discover it the day retention lands — RFC-003 already requires any
    /// future compaction to keep a snapshot floor.
    #[error("cursor precedes the oldest retained event")]
    CursorExpired,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

#[derive(Debug, Clone, Copy)]
pub struct MutationIntent([u8; 32]);

impl MutationIntent {
    pub fn new(action: &str, target: &str, payload: &Value) -> Self {
        let payload = canonical_json(payload);
        let payload = serde_json::to_vec(&payload).expect("JSON values always serialize");
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"waveflow-sync-intent-v1");
        for component in [action.as_bytes(), target.as_bytes(), payload.as_slice()] {
            hasher.update(&(component.len() as u64).to_le_bytes());
            hasher.update(component);
        }
        Self(*hasher.finalize().as_bytes())
    }

    fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MutationContext {
    pub operation_id: Uuid,
    pub origin_device_id: Option<Uuid>,
}

impl MutationContext {
    pub fn server_generated() -> Self {
        Self {
            operation_id: Uuid::new_v4(),
            origin_device_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SyncChange {
    pub cursor: i64,
    pub event_id: Uuid,
    pub operation_id: Uuid,
    pub origin_device_id: Option<Uuid>,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub action: String,
    pub payload: Value,
    pub changed_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SyncPage {
    pub changes: Vec<SyncChange>,
    pub next_cursor: i64,
    pub has_more: bool,
}

#[derive(Debug, Clone)]
pub struct MutationReceipt {
    pub operation_id: Uuid,
    pub result_entity_id: Option<Uuid>,
    pub entity_type: String,
    pub cursor: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum OperationClaim {
    New,
    Replayed(MutationReceipt),
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct SyncNotice {
    pub cursor: i64,
}

#[derive(Clone)]
pub struct SyncService {
    db: Database,
    notices: broadcast::Sender<(Uuid, SyncNotice)>,
}

impl SyncService {
    pub fn new(db: Database) -> Self {
        let (notices, _) = broadcast::channel(256);
        Self { db, notices }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<(Uuid, SyncNotice)> {
        self.notices.subscribe()
    }

    pub(crate) async fn claim_operation(
        &self,
        writer_guard: &OwnedMutexGuard<()>,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        context: MutationContext,
        intent: MutationIntent,
    ) -> Result<OperationClaim, SyncError> {
        let reservation = self
            .db
            .reserve_sync_operation(
                writer_guard,
                connection,
                user_id,
                context.operation_id,
                context.origin_device_id,
                intent.as_bytes(),
                now_ms(),
            )
            .await?;
        match reservation {
            SyncOperationReservation::InvalidOriginDevice => {
                let device_id = context
                    .origin_device_id
                    .expect("invalid reservation includes an origin device");
                tracing::warn!(
                    %user_id,
                    operation_id = %context.operation_id,
                    %device_id,
                    "sync mutation rejected for an invalid origin device"
                );
                Err(SyncError::Invalid)
            }
            SyncOperationReservation::New => Ok(OperationClaim::New),
            SyncOperationReservation::Incomplete => {
                tracing::error!(
                    %user_id,
                    operation_id = %context.operation_id,
                    "sync operation exists but has no completed event"
                );
                Err(sqlx::Error::Protocol("sync operation is incomplete".into()).into())
            }
            SyncOperationReservation::Replayed(record) => {
                if record.intent_hash.as_deref() != Some(intent.as_bytes()) {
                    return Err(SyncError::Conflict);
                }
                Ok(OperationClaim::Replayed(MutationReceipt {
                    operation_id: context.operation_id,
                    result_entity_id: record.result_entity_id.map(parse_uuid).transpose()?,
                    entity_type: record.entity_type,
                    cursor: record.event_cursor,
                    replayed: true,
                }))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn complete_operation(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        context: MutationContext,
        entity_type: &str,
        entity_id: Uuid,
        action: &str,
        payload: &Value,
        result_entity_id: Option<Uuid>,
    ) -> Result<MutationReceipt, sqlx::Error> {
        let changed_at = now_ms();
        let event_id = Uuid::new_v4();
        let cursor: i64 = sqlx::query_scalar(
            "INSERT INTO sync_event \
             (event_id, user_id, operation_id, origin_device_id, entity_type, entity_id, \
              action, payload_json, changed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             RETURNING cursor",
        )
        .bind(event_id.to_string())
        .bind(user_id.to_string())
        .bind(context.operation_id.to_string())
        .bind(context.origin_device_id.map(|id| id.to_string()))
        .bind(entity_type)
        .bind(entity_id.to_string())
        .bind(action)
        .bind(payload.to_string())
        .bind(changed_at)
        .fetch_one(&mut *connection)
        .await?;
        sqlx::query(
            "UPDATE sync_operation SET result_entity_id=?, event_cursor=?, applied_at=? \
             WHERE user_id=? AND operation_id=?",
        )
        .bind(result_entity_id.map(|id| id.to_string()))
        .bind(cursor)
        .bind(changed_at)
        .bind(user_id.to_string())
        .bind(context.operation_id.to_string())
        .execute(&mut *connection)
        .await?;
        Ok(MutationReceipt {
            operation_id: context.operation_id,
            result_entity_id,
            entity_type: entity_type.to_owned(),
            cursor,
            replayed: false,
        })
    }

    pub(crate) fn publish(&self, user_id: Uuid, receipt: MutationReceipt) {
        if !receipt.replayed {
            let _ = self.notices.send((
                user_id,
                SyncNotice {
                    cursor: receipt.cursor,
                },
            ));
        }
    }

    pub async fn changes(
        &self,
        user_id: Uuid,
        after: i64,
        limit: i64,
    ) -> Result<SyncPage, SyncError> {
        if after < 0 || !(1..=MAX_SYNC_LIMIT).contains(&limit) {
            return Err(SyncError::Invalid);
        }
        // A journal that starts above `after + 1` has dropped events the caller
        // never saw. Returning the surviving tail would look like a successful
        // catch-up while silently skipping the gap, so refuse instead.
        //
        // The floor is the journal's own, not this user's. `cursor` is a single
        // global sequence, so a user's own MIN only marks where they first
        // wrote: on a shared instance a new account snapshots at cursor 0, is
        // handed a high cursor for its first event, and would then be told its
        // perfectly valid cursor had expired.
        //
        // This assumes compaction, when it arrives, trims the head of the
        // journal for everyone. A per-user retention policy would need a stored
        // per-user floor instead — deriving one from the rows that survive
        // cannot distinguish "purged" from "never written".
        let floor: Option<i64> = sqlx::query_scalar("SELECT MIN(cursor) FROM sync_event")
            .fetch_one(self.db.pool())
            .await?;
        if floor.is_some_and(|floor| after < floor - 1) {
            return Err(SyncError::CursorExpired);
        }
        let rows = sqlx::query(
            "SELECT cursor, event_id, operation_id, origin_device_id, entity_type, entity_id, \
                    action, payload_json, changed_at \
             FROM sync_event WHERE user_id=? AND cursor>? ORDER BY cursor LIMIT ?",
        )
        .bind(user_id.to_string())
        .bind(after)
        .bind(limit + 1)
        .fetch_all(self.db.pool())
        .await?;
        let has_more = rows.len() as i64 > limit;
        let changes = rows
            .into_iter()
            .take(limit as usize)
            .map(change_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = changes.last().map_or(after, |change| change.cursor);
        Ok(SyncPage {
            changes,
            next_cursor,
            has_more,
        })
    }

    pub async fn latest_cursor(&self, user_id: Uuid) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT COALESCE(MAX(cursor), 0) FROM sync_event WHERE user_id=?")
            .bind(user_id.to_string())
            .fetch_one(self.db.pool())
            .await
    }

    pub async fn device_belongs_to_user(
        &self,
        user_id: Uuid,
        device_id: Uuid,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM device \
             WHERE id=? AND user_id=? AND revoked_at IS NULL)",
        )
        .bind(device_id.to_string())
        .bind(user_id.to_string())
        .fetch_one(self.db.pool())
        .await
    }

    pub async fn acknowledge(
        &self,
        user_id: Uuid,
        device_id: Uuid,
        cursor: i64,
    ) -> Result<bool, sqlx::Error> {
        if cursor < 0 || cursor > self.latest_cursor(user_id).await? {
            return Ok(false);
        }
        let _writer = self.db.writer_guard().await;
        let result = sqlx::query(
            "INSERT INTO sync_ack (user_id, device_id, cursor, acknowledged_at) \
             SELECT ?, ?, ?, ? WHERE EXISTS ( \
               SELECT 1 FROM device WHERE id=? AND user_id=? AND revoked_at IS NULL \
             ) ON CONFLICT (user_id, device_id) DO UPDATE SET \
               cursor=MAX(sync_ack.cursor, excluded.cursor), acknowledged_at=excluded.acknowledged_at",
        )
        .bind(user_id.to_string())
        .bind(device_id.to_string())
        .bind(cursor)
        .bind(now_ms())
        .bind(device_id.to_string())
        .bind(user_id.to_string())
        .execute(self.db.pool())
        .await?;
        Ok(result.rows_affected() == 1)
    }
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            Value::Object(
                keys.into_iter()
                    .map(|key| (key.clone(), canonical_json(&values[key])))
                    .collect(),
            )
        }
        value => value.clone(),
    }
}

fn change_from_row(row: sqlx::sqlite::SqliteRow) -> Result<SyncChange, sqlx::Error> {
    let payload: String = row.try_get("payload_json")?;
    Ok(SyncChange {
        cursor: row.try_get("cursor")?,
        event_id: parse_uuid(row.try_get("event_id")?)?,
        operation_id: parse_uuid(row.try_get("operation_id")?)?,
        origin_device_id: row
            .try_get::<Option<String>, _>("origin_device_id")?
            .map(parse_uuid)
            .transpose()?,
        entity_type: row.try_get("entity_type")?,
        entity_id: parse_uuid(row.try_get("entity_id")?)?,
        action: row.try_get("action")?,
        payload: serde_json::from_str(&payload)
            .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
        changed_at: row.try_get("changed_at")?,
    })
}

fn parse_uuid(value: String) -> Result<Uuid, sqlx::Error> {
    Uuid::parse_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}

#[cfg(test)]
mod tests {
    use super::MutationIntent;

    #[test]
    fn mutation_intents_are_canonical_and_scope_action_target_and_payload() {
        let first = MutationIntent::new(
            "update",
            "playlist:one",
            &serde_json::json!({ "name": "Road", "tracks": ["a", "b"] }),
        );
        let reordered = MutationIntent::new(
            "update",
            "playlist:one",
            &serde_json::json!({ "tracks": ["a", "b"], "name": "Road" }),
        );
        assert_eq!(first.as_bytes(), reordered.as_bytes());
        assert_ne!(
            first.as_bytes(),
            MutationIntent::new(
                "delete",
                "playlist:one",
                &serde_json::json!({ "name": "Road", "tracks": ["a", "b"] }),
            )
            .as_bytes()
        );
        assert_ne!(
            first.as_bytes(),
            MutationIntent::new(
                "update",
                "playlist:two",
                &serde_json::json!({ "name": "Road", "tracks": ["a", "b"] }),
            )
            .as_bytes()
        );
        assert_ne!(
            first.as_bytes(),
            MutationIntent::new(
                "update",
                "playlist:one",
                &serde_json::json!({ "name": "Night", "tracks": ["a", "b"] }),
            )
            .as_bytes()
        );
    }
}
