//! Multi-device sync hub. Phase 1.f per RFC-001 §6.6.
//!
//! Three responsibilities live here:
//!
//! 1. **Broadcast fan-out** — every successful `POST /api/v1/sync/ops`
//!    pushes its newly-assigned rows onto a single tokio
//!    `broadcast::Sender`. WebSocket handlers subscribe at connect and
//!    filter on `user_id` so cross-tenant ops never reach a foreign
//!    socket. One channel rather than per-user channels keeps the
//!    state structure trivial — the per-receive `user_id` equality
//!    check is cheap compared to the lock + map lookup a per-user
//!    `DashMap<i64, Sender>` would need on every emit.
//!
//! 2. **ACK debouncing** — `POST /api/v1/sync/ack` (and WebSocket
//!    `{"ack": N}` frames) write into an in-memory `AckBuffer` keyed
//!    on `(user_id, device_id)`. A background task wakes every
//!    `flush_interval` (default 5 s) and flushes every dirty entry to
//!    `device_sync_cursor` in a single transaction. The compaction job
//!    flushes the buffer synchronously before it reads — so the MIN
//!    it computes always reflects the latest ACK every device sent,
//!    closing the "compaction MIN reads a stale Postgres row" race
//!    the RFC calls out as the dangerous failure mode.
//!
//! 3. **Compaction** — a daily tokio task collapses superseded ops
//!    `WHERE id <= MIN(last_seen_id)` (excluding stale devices). The
//!    delete and the `sync_compaction_watermark` update happen in the
//!    same Postgres transaction so a kill mid-compaction leaves either
//!    the old state OR the new state, never one without the other.
//!
//! Boot is in [`SyncHub::spawn`], which returns the hub itself plus the
//! `JoinHandle`s the binary needs to keep alive. Tests can build a
//! hub via [`SyncHub::for_tests`] which skips the background tasks so
//! the test stays deterministic.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::{
    sync::broadcast,
    task::JoinHandle,
    time::{interval, MissedTickBehavior},
};
use utoipa::ToSchema;
use uuid::Uuid;

/// Capacity of the global broadcast channel. Each `POST /sync/ops`
/// pushes one row per accepted op; with thousands of concurrently-
/// connected WebSocket subscribers, a slow consumer could lag behind
/// — `broadcast` then returns `RecvError::Lagged` instead of buffering
/// without bound. 4096 is enough headroom for a bursty batch from a
/// freshly-rescanned library without forcing an unbounded queue.
const BROADCAST_CAPACITY: usize = 4096;

/// Default ACK flush cadence. Five seconds matches the RFC and gives
/// a sensible knob: short enough that a crash loses at most one
/// window of cursors (the compaction job rejoins the lost ground from
/// the in-memory buffer anyway), long enough that a chatty WebSocket
/// session doesn't write-amplify the cursor row.
pub const DEFAULT_FLUSH_INTERVAL: Duration = Duration::from_secs(5);

/// Default compaction cadence. Daily is the spec's call.
pub const DEFAULT_COMPACTION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// A device that hasn't ACKed for this long stops pinning the
/// compaction floor. 90 days is the spec's call — long enough that a
/// laptop-on-shelf-for-summer comes back fine, short enough that a
/// permanently-lost device doesn't drag the log forever.
pub const STALE_DEVICE_MS: i64 = 90 * 24 * 60 * 60 * 1000;

/// Wire format for a single op the client pushes. `payload` stays
/// opaque to the server — it's stored as JSONB and replayed verbatim
/// to other devices, so the schema can evolve client-side without a
/// server release.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SyncOpIn {
    /// Client-generated UUIDv4. Idempotency key against
    /// `(user_id, device_id, operation_id)`.
    pub operation_id: Uuid,
    /// Per-(user, device) monotonic counter. Strictly increasing.
    pub lamport_ts: i64,
    /// Entity type, e.g. `"playlist"`, `"library"`.
    pub entity: String,
    /// Free-form entity id (TEXT to span the desktop's mixed id schemes).
    pub entity_id: String,
    /// `None` for whole-entity ops (insert / delete), `Some` for
    /// partial updates ("set name", "set color").
    #[serde(default)]
    pub field: Option<String>,
    /// `"set" | "delete" | "insert" | "noop"`. Kept as a free-form
    /// string so the protocol can grow without a server-side enum
    /// expansion + migration.
    pub op: String,
    /// Op-specific JSON payload.
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

/// Wire format for an accepted op. Mirrors the row shape so the
/// pull (`GET /sync/ops`) and broadcast paths emit the same JSON
/// straight from the `SELECT … RETURNING …` rowset.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SyncOp {
    /// Server-assigned BIGSERIAL — the monotonic watermark every
    /// device tracks via `last_seen_id`.
    pub id: i64,
    /// Echoed back so the client can correlate a POST response with
    /// the original request even when `accepted` reorders.
    pub operation_id: Uuid,
    /// Echoed back so a pulling device can keep its own clock in sync.
    pub device_id: String,
    pub lamport_ts: i64,
    pub entity: String,
    pub entity_id: String,
    pub field: Option<String>,
    pub op: String,
    pub payload: Option<serde_json::Value>,
    pub created_at: i64,
}

/// Broadcast payload. We pre-serialise to JSON once at emit time so
/// every receiver (potentially hundreds of WebSocket subscribers)
/// avoids re-serialising the same row.
#[derive(Debug, Clone)]
pub struct SyncBroadcast {
    /// Owner of the op. WS subscribers compare against the
    /// connection's authenticated `user_id`; mismatches are dropped
    /// before the frame ever leaves the socket task.
    pub user_id: i64,
    /// Pre-serialised `{"type":"op","op":{…}}` envelope, ready to
    /// hand to `WebSocket::send(Message::Text(...))`.
    pub frame: Arc<String>,
}

/// Per-(user, device) ACK accumulator. Atomics let the
/// REST + WS handlers update without taking a lock; the periodic
/// flusher reads + clears `dirty` to decide what to UPSERT.
#[derive(Debug)]
struct AckEntry {
    last_seen_id: AtomicI64,
    last_seen_at_ms: AtomicI64,
    dirty: AtomicBool,
}

/// Shared sync state. Cheap to clone — every field is `Arc`-backed
/// (the broadcast `Sender` clones into a new handle, the `DashMap`
/// is `Arc`-internal, the pool is `Arc`-backed).
#[derive(Clone)]
pub struct SyncHub {
    inner: Arc<SyncHubInner>,
}

struct SyncHubInner {
    broadcast_tx: broadcast::Sender<SyncBroadcast>,
    ack_buffer: DashMap<(i64, String), Arc<AckEntry>>,
    db: PgPool,
}

impl SyncHub {
    /// Build a hub against the live pool. Spawns the flush + compaction
    /// tasks and returns their join handles so the binary can keep them
    /// alive for the lifetime of the process.
    pub fn spawn(
        db: PgPool,
        flush_interval: Duration,
        compaction_interval: Duration,
    ) -> (Self, JoinHandle<()>, JoinHandle<()>) {
        let (broadcast_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let hub = Self {
            inner: Arc::new(SyncHubInner {
                broadcast_tx,
                ack_buffer: DashMap::new(),
                db,
            }),
        };

        let flush_task = {
            let hub = hub.clone();
            tokio::spawn(async move { hub.run_flush_loop(flush_interval).await })
        };
        let compaction_task = {
            let hub = hub.clone();
            tokio::spawn(async move { hub.run_compaction_loop(compaction_interval).await })
        };

        (hub, flush_task, compaction_task)
    }

    /// Test helper — no background tasks. Tests drive [`flush_acks`]
    /// and [`compact_once`] directly so each step is observable.
    pub fn for_tests(db: PgPool) -> Self {
        let (broadcast_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(SyncHubInner {
                broadcast_tx,
                ack_buffer: DashMap::new(),
                db,
            }),
        }
    }

    /// Subscribe a new WebSocket session to the global broadcast.
    /// The receiver itself filters per-user — see the WS handler.
    pub fn subscribe(&self) -> broadcast::Receiver<SyncBroadcast> {
        self.inner.broadcast_tx.subscribe()
    }

    /// Emit a fan-out frame. Returns the receiver count for the metric
    /// the handler logs; the underlying `send` returns `Err` when no
    /// subscribers are attached, which is fine and means "nothing to
    /// do".
    pub fn broadcast(&self, payload: SyncBroadcast) {
        let _ = self.inner.broadcast_tx.send(payload);
    }

    /// Record an ACK. Idempotent + monotonic — a lower `last_seen_id`
    /// is ignored. Marks the entry dirty so the next flush picks it up.
    pub fn record_ack(&self, user_id: i64, device_id: &str, last_seen_id: i64, now_ms: i64) {
        let entry = self
            .inner
            .ack_buffer
            .entry((user_id, device_id.to_string()))
            .or_insert_with(|| {
                Arc::new(AckEntry {
                    last_seen_id: AtomicI64::new(0),
                    last_seen_at_ms: AtomicI64::new(0),
                    dirty: AtomicBool::new(false),
                })
            })
            .clone();
        // CAS-loop bumps `last_seen_id` only if the incoming value is
        // strictly higher. A no-op CAS short-circuits without writing
        // (and crucially without marking the row dirty) so a chatty
        // client re-acknowledging the same id never amplifies writes.
        let mut current = entry.last_seen_id.load(Ordering::Relaxed);
        loop {
            if last_seen_id <= current {
                return;
            }
            match entry.last_seen_id.compare_exchange_weak(
                current,
                last_seen_id,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(seen) => current = seen,
            }
        }
        entry.last_seen_at_ms.store(now_ms, Ordering::Relaxed);
        entry.dirty.store(true, Ordering::Release);
    }

    /// Flush every dirty ACK to `device_sync_cursor`. Public so the
    /// compaction job can call it synchronously before reading the MIN
    /// (otherwise the MIN reflects stale ACKs and compaction would
    /// over-retain history at best, miscalculate the floor at worst).
    pub async fn flush_acks(&self) -> Result<usize, sqlx::Error> {
        // Snapshot the dirty rows under a single pass over the map.
        // We `.clone()` the `Arc<AckEntry>` so the iteration releases
        // each shard lock quickly and the actual SQL runs lock-free.
        let mut staged: Vec<(i64, String, i64, i64)> = Vec::new();
        for entry in self.inner.ack_buffer.iter() {
            let ((user_id, device_id), state) = (entry.key(), entry.value());
            // Clear dirty optimistically — if the SQL fails we re-set
            // it from the error path so the next flush retries. The
            // ordering matters: `swap(false, AcqRel)` returns the old
            // value, so we only stage entries that were actually dirty
            // since the last flush.
            if state.dirty.swap(false, Ordering::AcqRel) {
                staged.push((
                    *user_id,
                    device_id.clone(),
                    state.last_seen_id.load(Ordering::Acquire),
                    state.last_seen_at_ms.load(Ordering::Acquire),
                ));
            }
        }
        if staged.is_empty() {
            return Ok(0);
        }

        let mut tx = self.inner.db.begin().await?;
        let mut applied = 0usize;
        for (user_id, device_id, last_seen_id, last_seen_at) in &staged {
            // Monotonic UPSERT — refuse to lower an existing
            // last_seen_id. A pull-without-process scenario where the
            // client acked N + then crashed mid-process would otherwise
            // let a "behind" cursor reach the cursor and pin compaction
            // too high.
            let res = sqlx::query(
                "INSERT INTO device_sync_cursor \
                    (user_id, device_id, last_seen_id, last_seen_at) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (user_id, device_id) DO UPDATE \
                 SET last_seen_id = EXCLUDED.last_seen_id, \
                     last_seen_at = EXCLUDED.last_seen_at \
                 WHERE device_sync_cursor.last_seen_id < EXCLUDED.last_seen_id",
            )
            .bind(user_id)
            .bind(device_id)
            .bind(last_seen_id)
            .bind(last_seen_at)
            .execute(&mut *tx)
            .await;
            match res {
                Ok(_) => applied += 1,
                Err(err) => {
                    // Re-mark the staged entries dirty so the next
                    // flush retries them — losing an ACK to a
                    // transient DB hiccup would let the cursor drift
                    // and over-retain forever.
                    self.requeue_dirty(&staged);
                    tx.rollback().await.ok();
                    return Err(err);
                }
            }
        }
        tx.commit().await?;
        Ok(applied)
    }

    fn requeue_dirty(&self, staged: &[(i64, String, i64, i64)]) {
        for (user_id, device_id, _, _) in staged {
            if let Some(entry) = self.inner.ack_buffer.get(&(*user_id, device_id.clone())) {
                entry.dirty.store(true, Ordering::Release);
            }
        }
    }

    async fn run_flush_loop(self, period: Duration) {
        let mut ticker = interval(period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        // First tick fires immediately — burn it so we don't flush an
        // empty buffer on boot.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            if let Err(err) = self.flush_acks().await {
                tracing::error!(error = %err, "sync ack flush failed");
            }
        }
    }

    async fn run_compaction_loop(self, period: Duration) {
        let mut ticker = interval(period);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        ticker.tick().await; // burn the immediate first tick
        loop {
            ticker.tick().await;
            if let Err(err) = self.compact_once().await {
                tracing::error!(error = %err, "sync compaction failed");
            }
        }
    }

    /// Run one compaction pass. Public so tests can step through.
    /// Algorithm:
    ///
    /// 1. Flush the in-memory ACK buffer to `device_sync_cursor` so the
    ///    MIN below sees every device's true position.
    /// 2. For each user with at least one fresh cursor (ACKed within
    ///    [`STALE_DEVICE_MS`]):
    ///    - Compute `MIN(last_seen_id)` across the fresh cursors.
    ///    - In the same transaction, delete superseded ops
    ///      (`id <= min`, keeping only the latest per
    ///      `(entity, entity_id, COALESCE(field, ''))`) and UPSERT the
    ///      watermark to `min`.
    ///
    /// Invariants:
    ///
    /// - Phase 1 flush is the "ACK MIN includes unflushed" guarantee.
    /// - The delete + watermark UPSERT happen in the same Postgres
    ///   transaction so a crash mid-compaction is atomic.
    /// - Watermark is monotonic — the UPSERT `WHERE` clause refuses
    ///   to lower an existing value.
    pub async fn compact_once(&self) -> Result<CompactionReport, sqlx::Error> {
        // Phase 1: flush pending ACKs so the MIN read below sees the
        // freshest device positions.
        self.flush_acks().await?;

        let now = Utc::now().timestamp_millis();
        let stale_threshold = now - STALE_DEVICE_MS;
        let mut report = CompactionReport::default();

        let users: Vec<i64> = sqlx::query_scalar(
            "SELECT DISTINCT user_id FROM device_sync_cursor \
             WHERE last_seen_at >= $1",
        )
        .bind(stale_threshold)
        .fetch_all(&self.inner.db)
        .await?;

        for user_id in users {
            let mut tx = self.inner.db.begin().await?;

            let min_seen: Option<i64> = sqlx::query_scalar(
                "SELECT MIN(last_seen_id) FROM device_sync_cursor \
                 WHERE user_id = $1 AND last_seen_at >= $2",
            )
            .bind(user_id)
            .bind(stale_threshold)
            .fetch_one(&mut *tx)
            .await?;

            let Some(min) = min_seen else {
                tx.commit().await?;
                continue;
            };

            // Collapse: keep only the most-recent op per
            // `(entity, entity_id, COALESCE(field, ''))` at or below
            // `min`. Older partial updates to the same field are
            // dead weight by definition — no device behind the
            // watermark needs them.
            let deleted = sqlx::query(
                "DELETE FROM sync_op \
                 WHERE id IN ( \
                    SELECT id FROM ( \
                        SELECT id, ROW_NUMBER() OVER ( \
                            PARTITION BY entity, entity_id, COALESCE(field, '') \
                            ORDER BY lamport_ts DESC, id DESC \
                        ) AS rn \
                        FROM sync_op \
                        WHERE user_id = $1 AND id <= $2 \
                    ) ranked \
                    WHERE rn > 1 \
                 )",
            )
            .bind(user_id)
            .bind(min)
            .execute(&mut *tx)
            .await?
            .rows_affected();

            // Monotonic UPSERT — refuse to lower an existing
            // watermark. A simultaneous compaction (we don't expect
            // one, but the invariant is cheap to preserve) wouldn't
            // be able to roll history back.
            sqlx::query(
                "INSERT INTO sync_compaction_watermark \
                    (user_id, compacted_up_to, updated_at) \
                 VALUES ($1, $2, $3) \
                 ON CONFLICT (user_id) DO UPDATE \
                 SET compacted_up_to = EXCLUDED.compacted_up_to, \
                     updated_at = EXCLUDED.updated_at \
                 WHERE sync_compaction_watermark.compacted_up_to \
                       < EXCLUDED.compacted_up_to",
            )
            .bind(user_id)
            .bind(min)
            .bind(now)
            .execute(&mut *tx)
            .await?;

            tx.commit().await?;

            report.users_compacted += 1;
            report.rows_deleted += deleted;
        }

        Ok(report)
    }

    /// Direct access to the underlying pool — handlers reuse it for
    /// the per-request reads they own (push, pull, watermark check).
    pub fn pool(&self) -> &PgPool {
        &self.inner.db
    }
}

/// Summary of one compaction pass. Returned from [`SyncHub::compact_once`]
/// so tests can assert on observable progress without re-querying the
/// schema.
#[derive(Debug, Default, Clone, Copy)]
pub struct CompactionReport {
    pub users_compacted: usize,
    pub rows_deleted: u64,
}

/// Build a `SyncBroadcast` envelope for an accepted op. Pre-serialises
/// the frame so every receiver hands the cached `Arc<String>` to
/// `WebSocket::send` without re-serialising.
pub fn build_broadcast(user_id: i64, op: &SyncOp) -> SyncBroadcast {
    // The frame schema is `{"type":"op","op":{…}}`. Pre-serialise once
    // here so every subscriber dispatches an `Arc<String>` clone
    // rather than re-serialising the same row N times.
    let frame = serde_json::to_string(&serde_json::json!({
        "type": "op",
        "op": op,
    }))
    .expect("SyncOp serialises");
    SyncBroadcast {
        user_id,
        frame: Arc::new(frame),
    }
}
