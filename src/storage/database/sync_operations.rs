//! The idempotency ledger of client sync operations: reserving an operation
//! id, and recognising one already applied.

use sqlx::{Row, SqliteConnection};
use tokio::sync::OwnedMutexGuard;
use uuid::Uuid;

use super::Database;

#[derive(Debug)]
pub(crate) struct SyncOperationReplayRecord {
    pub result_entity_id: Option<String>,
    pub event_cursor: i64,
    pub intent_hash: Option<Vec<u8>>,
    pub entity_type: String,
}

#[derive(Debug)]
pub(crate) enum SyncOperationReservation {
    InvalidOriginDevice,
    New,
    Incomplete,
    Replayed(SyncOperationReplayRecord),
}

impl Database {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reserve_sync_operation(
        &self,
        _writer_guard: &OwnedMutexGuard<()>,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        operation_id: Uuid,
        origin_device_id: Option<Uuid>,
        intent_hash: &[u8],
        created_at: i64,
    ) -> Result<SyncOperationReservation, sqlx::Error> {
        if let Some(device_id) = origin_device_id {
            let owned = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM device \
                 WHERE id=? AND user_id=? AND revoked_at IS NULL)",
            )
            .bind(device_id.to_string())
            .bind(user_id.to_string())
            .fetch_one(&mut *connection)
            .await?;
            if !owned {
                return Ok(SyncOperationReservation::InvalidOriginDevice);
            }
        }

        let inserted = sqlx::query(
            "INSERT INTO sync_operation \
             (user_id, operation_id, origin_device_id, intent_hash, created_at) VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (user_id, operation_id) DO NOTHING",
        )
        .bind(user_id.to_string())
        .bind(operation_id.to_string())
        .bind(origin_device_id.map(|id| id.to_string()))
        .bind(intent_hash)
        .bind(created_at)
        .execute(&mut *connection)
        .await?
        .rows_affected();
        if inserted == 1 {
            return Ok(SyncOperationReservation::New);
        }

        let row = sqlx::query(
            "SELECT so.result_entity_id, so.event_cursor, so.intent_hash, se.entity_type \
             FROM sync_operation so JOIN sync_event se ON se.cursor=so.event_cursor AND se.user_id=so.user_id \
             WHERE so.user_id=? AND so.operation_id=? AND so.applied_at IS NOT NULL",
        )
        .bind(user_id.to_string())
        .bind(operation_id.to_string())
        .fetch_optional(&mut *connection)
        .await?;
        let Some(row) = row else {
            return Ok(SyncOperationReservation::Incomplete);
        };
        Ok(SyncOperationReservation::Replayed(
            SyncOperationReplayRecord {
                result_entity_id: row.try_get("result_entity_id")?,
                event_cursor: row.try_get("event_cursor")?,
                intent_hash: row.try_get("intent_hash")?,
                entity_type: row.try_get("entity_type")?,
            },
        ))
    }
}
