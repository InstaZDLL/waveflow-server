//! `/api/v1/sync/*` — multi-device sync surface per RFC-001 §6.6.
//!
//! Three REST routes + one WebSocket:
//!
//! - `POST /api/v1/sync/ops` — push a batch. Per-op idempotency on
//!   `operation_id`; per-device monotonicity on `lamport_ts`. New rows
//!   broadcast to every WebSocket subscriber owned by the same user.
//! - `GET  /api/v1/sync/ops?since=N` — pull rows with `id > N`.
//!   Resurrected-device guard: a `since` below the compaction
//!   watermark returns `410 Gone` so the client knows to resync from
//!   scratch instead of converging on a half-collapsed view.
//! - `POST /api/v1/sync/ack` — the **only** path that advances the
//!   per-device cursor. Body is buffered in memory and UPSERTed every
//!   five seconds (or synchronously by the compaction job before the
//!   MIN read).
//! - `GET  /api/v1/sync/ws?device_id=…` — WebSocket. Server pushes
//!   `{"type":"op","op":{…}}` envelopes for the authenticated user;
//!   client sends `{"ack": N}` frames to advance its cursor. Disconnect
//!   triggers a synchronous flush of that device's buffered ACK so a
//!   browser tab close doesn't lose the position.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Extension, Query, State,
    },
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgRow, Row};
use tokio::sync::broadcast::error::RecvError;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use crate::{
    db,
    middleware::UserId,
    sync::{build_broadcast, Hlc, SyncOp, SyncOpIn},
    AppState,
};

/// Max ops per batch. A hard cap on the request body keeps a buggy or
/// hostile client from holding a transaction open across a million-row
/// insert. The desktop's scanner pipeline commits every 200 rows; 1024
/// is comfortable headroom for a coalesced flush.
const MAX_BATCH_SIZE: usize = 1024;

/// Max rows per pull response. 1024 matches the push cap; clients
/// loop on `since = last_id` until they receive a short page.
const PULL_PAGE_SIZE: i64 = 1024;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PushBatchRequest {
    /// Originating device. Same value the WS handshake uses on
    /// `?device_id=…`.
    pub device_id: String,
    pub ops: Vec<SyncOpIn>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PushBatchResponse {
    /// Every op the server now considers durably stored — both fresh
    /// inserts AND duplicate replays of previously-accepted rows. The
    /// client uses this to confirm what landed; a missing op means
    /// the batch was aborted (409 / 500).
    pub accepted: Vec<SyncOp>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LamportRegression {
    pub error: &'static str,
    pub device_id: String,
    pub stored_max: i64,
    pub offending_lamport_ts: i64,
}

/// 409 body when a v2 client pushes an `hlc` pair that collides on the
/// `(user_id, device_id, hlc_wall, hlc_logical)` UNIQUE — RFC-003 §2
/// requires per-device HLC monotonicity, so this is the v2 equivalent
/// of the lamport-regression case. The offending pair is echoed so
/// the client can resync its HLC instead of guessing how far the
/// server has advanced.
#[derive(Debug, Serialize, ToSchema)]
pub struct HlcRegression {
    pub error: &'static str,
    pub device_id: String,
    pub offending_hlc: Hlc,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct PullQuery {
    /// Last `sync_op.id` the client has confirmed seeing. `0` (or
    /// omitted) means "send me everything from the start".
    #[serde(default)]
    pub since: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PullResponse {
    pub ops: Vec<SyncOp>,
    /// Highest `id` in this batch — convenience so the client doesn't
    /// have to scan `ops` to compute the next `since`. Equals `since`
    /// when the page is empty.
    pub last_id: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ResurrectedDeviceGone {
    pub error: &'static str,
    pub compacted_up_to: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AckRequest {
    pub device_id: String,
    pub last_seen_id: i64,
}

#[derive(Debug, Deserialize)]
struct WsQuery {
    device_id: String,
}

pub fn router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(push_ops, pull_ops))
        .routes(routes!(ack_ops))
        .route("/api/v1/sync/ws", axum::routing::get(ws_upgrade))
        .with_state(state)
}

/// Push a batch of ops. Per-op semantics:
///
/// - `ON CONFLICT (user_id, device_id, operation_id) DO NOTHING`
///   absorbs idempotent replays.
/// - The `(user_id, device_id, lamport_ts)` UNIQUE bubbles up as
///   sqlx `23505` on a regression — caught + reported as 409.
///
/// All ops land in a single transaction; broadcast fires after the
/// commit so a subscriber never sees an op that gets rolled back.
#[utoipa::path(
    post,
    path = "/api/v1/sync/ops",
    tag = "sync",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
    ),
    request_body = PushBatchRequest,
    responses(
        (status = 200, description = "Batch accepted (one entry per op, fresh or dup)", body = PushBatchResponse),
        (status = 400, description = "Empty `device_id`, oversized batch, malformed op, or hlc.wall/logical < 0"),
        (status = 401, description = "Missing or invalid bearer token"),
        // Two regression shapes can land on 409 — discriminated on the
        // `error` field of the body. The legacy v1 path returns
        // `LamportRegression { error: \"lamport_regression\", stored_max, offending_lamport_ts }`;
        // the v2 path returns `HlcRegression { error: \"hlc_regression\", offending_hlc }`.
        // utoipa 5 has no concise `oneOf` for response bodies, so we
        // list both shapes as sibling 409 entries — clients pattern-
        // match on `error` before reading the discriminating fields.
        (status = 409, description = "Lamport regression (v1) — body discriminated by `error: \"lamport_regression\"`", body = LamportRegression),
        (status = 409, description = "HLC collision (v2) — body discriminated by `error: \"hlc_regression\"`", body = HlcRegression),
        (status = 500, description = "Database or internal failure"),
    ),
)]
async fn push_ops(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Json(req): Json<PushBatchRequest>,
) -> impl IntoResponse {
    let device_id = req.device_id.trim();
    if device_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "device_id is required").into_response();
    }
    if req.ops.is_empty() {
        return (StatusCode::OK, Json(PushBatchResponse { accepted: vec![] })).into_response();
    }
    if req.ops.len() > MAX_BATCH_SIZE {
        return (
            StatusCode::BAD_REQUEST,
            format!("batch exceeds {MAX_BATCH_SIZE} ops"),
        )
            .into_response();
    }

    let now = Utc::now().timestamp_millis();
    let pool = state.sync.pool();

    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(err) => {
            tracing::error!(error = %err, user_id, "sync push begin tx failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "tx begin failed").into_response();
        }
    };

    let mut accepted: Vec<SyncOp> = Vec::with_capacity(req.ops.len());
    let mut freshly_inserted: Vec<SyncOp> = Vec::new();

    for op_in in &req.ops {
        let entity = op_in.entity.trim();
        let entity_id = op_in.entity_id.trim();
        let op_kind = op_in.op.trim();
        if entity.is_empty() || entity_id.is_empty() || op_kind.is_empty() {
            tx.rollback().await.ok();
            return (
                StatusCode::BAD_REQUEST,
                "entity / entity_id / op are required",
            )
                .into_response();
        }

        // RFC-003 Phase A.2 — v2 wire shape carries an explicit
        // `hlc` pair. Validate `wall >= 0` (logical is already i32
        // by the type, so the only out-of-range a v2 client can hit
        // is a negative wall — usually a clock-set bug). The §2
        // tiebreaker `origin_device_id` rides through the existing
        // `device_id` string per A.1.1's design. The server never
        // tries to "fix up" a missing hlc by synthesising one — the
        // v1 path's `(0, lamport_ts)` derivation owns that case.
        if let Some(hlc) = op_in.hlc {
            if hlc.wall < 0 {
                tx.rollback().await.ok();
                return (StatusCode::BAD_REQUEST, "hlc.wall must be >= 0").into_response();
            }
            // Symmetric with `wall`. `logical` is `i32` by the type so
            // a negative is structurally legal but semantically wrong
            // per RFC-003 §2 (unsigned-shaped counter). Catching it
            // here returns 400 instead of letting the db helper's
            // defence-in-depth guard surface as a 500.
            if hlc.logical < 0 {
                tx.rollback().await.ok();
                return (StatusCode::BAD_REQUEST, "hlc.logical must be >= 0").into_response();
            }
        }
        let hlc_pair = op_in.hlc.map(|h| (h.wall, h.logical));

        let insert_res = db::sync::insert_op_returning(
            &mut tx,
            user_id,
            device_id,
            op_in.operation_id,
            op_in.lamport_ts,
            entity,
            entity_id,
            op_in.field.as_deref(),
            op_kind,
            op_in.payload.as_ref(),
            now,
            op_in.profile_canonical_id.as_deref(),
            hlc_pair,
        )
        .await;

        match insert_res {
            Ok(Some(row)) => {
                let op = row_to_op(&row);

                // Apply the op into the entity tables in the same
                // transaction as the durable insert (Phase 1.g.0).
                // A failure here rolls back the log row too — better
                // to refuse the push than to leave an op the server
                // can't honour. Skipped / Unknown are NOT errors:
                // the durable log keeps them so a future server
                // release can replay during compaction.
                match crate::apply::apply_op(&mut tx, user_id, op_in, now).await {
                    Ok(_outcome) => {}
                    Err(err) => {
                        tracing::error!(error = %err, user_id, entity = %op.entity, op = %op.op, "apply failed");
                        tx.rollback().await.ok();
                        return (StatusCode::INTERNAL_SERVER_ERROR, "apply failed").into_response();
                    }
                }

                accepted.push(op.clone());
                freshly_inserted.push(op);
            }
            Ok(None) => {
                // Duplicate `operation_id` — fold in the previously-
                // stored row so the client's `accepted` list is whole.
                match db::sync::fetch_op_by_operation_id(
                    &mut tx,
                    user_id,
                    device_id,
                    op_in.operation_id,
                )
                .await
                {
                    Ok(row) => accepted.push(row_to_op(&row)),
                    Err(err) => {
                        tracing::error!(error = %err, user_id, device_id, "sync dup lookup failed");
                        tx.rollback().await.ok();
                        return (StatusCode::INTERNAL_SERVER_ERROR, "dup lookup failed")
                            .into_response();
                    }
                }
            }
            Err(sqlx::Error::Database(db_err)) if db_err.code().as_deref() == Some("23505") => {
                // After A.1.1 two UNIQUE constraints can fire 23505 on
                // a regression. The third (`operation_id`) is absorbed
                // upstream by `ON CONFLICT DO NOTHING`, so we never see
                // it here. Discriminate on the constraint name so a v2
                // HLC collision doesn't get reported as a misleading
                // lamport regression (the stored lamport_max would be
                // meaningless to a v2 client whose clock is the HLC
                // pair, not lamport_ts).
                tx.rollback().await.ok();
                match db_err.constraint() {
                    Some("sync_op_user_device_hlc_uniq") => {
                        // V2 client pushed an HLC pair already taken
                        // by this device. Echo the offending pair so
                        // the client can resync; `op_in.hlc` is
                        // guaranteed `Some` here because the v1 path
                        // derives `(0, lamport_ts)` from a strictly-
                        // increasing lamport_ts (per the legacy
                        // `(user_id, device_id, lamport_ts)` UNIQUE
                        // also fires on regression), so a v1 client
                        // hitting THIS constraint exclusively would
                        // mean lamport_ts moved forward but the
                        // derived `(0, lamport_ts)` collided — only
                        // possible after a manual DB reset, which is
                        // out of scope. Default the pair anyway in
                        // case a future code path bypasses the v2
                        // gate.
                        return (
                            StatusCode::CONFLICT,
                            Json(HlcRegression {
                                error: "hlc_regression",
                                device_id: device_id.to_string(),
                                offending_hlc: op_in.hlc.unwrap_or(Hlc {
                                    wall: 0,
                                    logical: 0,
                                }),
                            }),
                        )
                            .into_response();
                    }
                    _ => {
                        // Legacy `(user_id, device_id, lamport_ts)`
                        // constraint (auto-named by Postgres). Read
                        // the current max and return it so the v1
                        // client can resync its clock.
                        let stored_max =
                            match db::sync::lamport_max(pool, user_id, device_id).await {
                                Ok(n) => n,
                                Err(err) => {
                                    // Surface the read failure rather
                                    // than masking it behind a `0`
                                    // that would tell the client
                                    // "your clock is fine, retry"
                                    // when it isn't.
                                    tracing::error!(error = %err, user_id, device_id, "lamport_max read failed");
                                    return (
                                        StatusCode::INTERNAL_SERVER_ERROR,
                                        "lamport_max read failed",
                                    )
                                        .into_response();
                                }
                            };
                        return (
                            StatusCode::CONFLICT,
                            Json(LamportRegression {
                                error: "lamport_regression",
                                device_id: device_id.to_string(),
                                stored_max,
                                offending_lamport_ts: op_in.lamport_ts,
                            }),
                        )
                            .into_response();
                    }
                }
            }
            Err(err) => {
                tracing::error!(error = %err, user_id, device_id, "sync insert failed");
                tx.rollback().await.ok();
                return (StatusCode::INTERNAL_SERVER_ERROR, "insert failed").into_response();
            }
        }
    }

    if let Err(err) = tx.commit().await {
        tracing::error!(error = %err, user_id, "sync push commit failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "commit failed").into_response();
    }

    // Broadcast outside the transaction — subscribers must never
    // observe an op that rolls back. The pre-serialised `Arc<String>`
    // means each subscriber gets a cheap clone.
    for op in &freshly_inserted {
        state.sync.broadcast(build_broadcast(user_id, op));
    }

    (StatusCode::OK, Json(PushBatchResponse { accepted })).into_response()
}

/// Pull ops with `id > since`. Read-only — never advances the device
/// cursor (that's `POST /sync/ack`'s sole job). Resurrected-device
/// guard: any `since` below the compaction watermark returns 410.
#[utoipa::path(
    get,
    path = "/api/v1/sync/ops",
    tag = "sync",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        PullQuery,
    ),
    responses(
        (status = 200, description = "Page of ops, capped at 1024", body = PullResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 410, description = "since < compacted_up_to — client must full-resync", body = ResurrectedDeviceGone),
        (status = 500, description = "Database or internal failure"),
    ),
)]
async fn pull_ops(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Query(query): Query<PullQuery>,
) -> impl IntoResponse {
    if query.since < 0 {
        return (StatusCode::BAD_REQUEST, "since must be >= 0").into_response();
    }
    let pool = state.sync.pool();

    // Resurrected-device guard. `since == 0` ("send me everything")
    // is always allowed — the compaction watermark only matters when
    // the client is claiming a position partway up the log. The
    // `compacted_up_to == 0` row (never compacted) is also a no-op
    // guard, so the condition is symmetric: `since > 0 AND since <
    // compacted_up_to`.
    if query.since > 0 {
        // Propagate a watermark read failure as 500 instead of
        // silently dropping it into `None` — masking a transient
        // pool / Postgres hiccup as "no compaction floor" could let a
        // resurrected device skip the 410 path and replay a partial
        // log into a state inconsistent with its peers.
        let compacted = match db::sync::fetch_compacted_up_to(pool, user_id).await {
            Ok(c) => c,
            Err(err) => {
                tracing::error!(error = %err, user_id, "watermark read failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "watermark read failed")
                    .into_response();
            }
        };
        if let Some(watermark) = compacted {
            if query.since < watermark {
                return (
                    StatusCode::GONE,
                    Json(ResurrectedDeviceGone {
                        error: "resync_required",
                        compacted_up_to: watermark,
                    }),
                )
                    .into_response();
            }
        }
    }

    let rows = db::sync::pull_ops_since(pool, user_id, query.since, PULL_PAGE_SIZE).await;

    match rows {
        Ok(rows) => {
            let ops: Vec<SyncOp> = rows.iter().map(row_to_op).collect();
            let last_id = ops.last().map(|o| o.id).unwrap_or(query.since);
            (StatusCode::OK, Json(PullResponse { ops, last_id })).into_response()
        }
        Err(err) => {
            tracing::error!(error = %err, user_id, "sync pull failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "pull failed").into_response()
        }
    }
}

/// Record an ACK. Goes into the in-memory buffer; the flush task
/// UPSERTs the row within `flush_interval` (default 5 s). The
/// compaction job synchronously flushes the buffer before reading the
/// MIN, so a fresh ACK never gets stranded in memory at compaction
/// time.
#[utoipa::path(
    post,
    path = "/api/v1/sync/ack",
    tag = "sync",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
    ),
    request_body = AckRequest,
    responses(
        (status = 204, description = "ACK buffered"),
        (status = 400, description = "Empty `device_id` or negative `last_seen_id`"),
        (status = 401, description = "Missing or invalid bearer token"),
    ),
)]
async fn ack_ops(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Json(req): Json<AckRequest>,
) -> impl IntoResponse {
    let device_id = req.device_id.trim();
    if device_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "device_id is required").into_response();
    }
    if req.last_seen_id < 0 {
        return (StatusCode::BAD_REQUEST, "last_seen_id must be >= 0").into_response();
    }
    state.sync.record_ack(
        user_id,
        device_id,
        req.last_seen_id,
        Utc::now().timestamp_millis(),
    );
    StatusCode::NO_CONTENT.into_response()
}

/// WebSocket upgrade. The `device_id` rides in the query string so the
/// socket task knows who's talking from the first byte — an in-band
/// "hello" frame would race with the first broadcast.
async fn ws_upgrade(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Query(params): Query<WsQuery>,
) -> impl IntoResponse {
    let device_id = params.device_id.trim().to_string();
    if device_id.is_empty() {
        return (StatusCode::BAD_REQUEST, "device_id is required").into_response();
    }
    ws.on_upgrade(move |socket| handle_socket(socket, state, user_id, device_id))
        .into_response()
}

#[derive(Debug, Deserialize)]
struct InboundAck {
    ack: i64,
}

async fn handle_socket(socket: WebSocket, state: AppState, user_id: i64, device_id: String) {
    let (mut sender, mut receiver) = socket.split();
    let mut broadcast_rx = state.sync.subscribe();

    loop {
        tokio::select! {
            // Inbound frame from the client. We only understand
            // `{"ack": N}` today; anything else is logged and dropped
            // so a future protocol extension doesn't break old
            // clients.
            inbound = receiver.next() => {
                match inbound {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ack) = serde_json::from_str::<InboundAck>(&text) {
                            if ack.ack >= 0 {
                                state.sync.record_ack(
                                    user_id,
                                    &device_id,
                                    ack.ack,
                                    Utc::now().timestamp_millis(),
                                );
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => { /* ignore non-text frames */ }
                    Some(Err(err)) => {
                        tracing::debug!(error = %err, user_id, device_id, "ws inbound error");
                        break;
                    }
                }
            }
            // Outbound broadcast. Filter to this user before sending —
            // a cross-tenant frame must never leave the socket.
            recv = broadcast_rx.recv() => {
                match recv {
                    Ok(payload) => {
                        if payload.user_id == user_id {
                            // Send the pre-serialised frame. A failure
                            // here means the socket died; bail and let
                            // the on-disconnect flush run.
                            if sender
                                .send(Message::Text((*payload.frame).clone().into()))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Err(RecvError::Closed) => break,
                    Err(RecvError::Lagged(skipped)) => {
                        // The subscriber fell behind the broadcast
                        // buffer. The cleanest recovery is to drop the
                        // socket and let the client pull via REST +
                        // reconnect — they have `since = last_seen_id`
                        // to resume from.
                        tracing::warn!(
                            user_id,
                            device_id,
                            skipped,
                            "ws broadcast lagged, closing socket",
                        );
                        break;
                    }
                }
            }
        }
    }

    // On disconnect, flush this device's ACK synchronously so a
    // crash on the next compaction tick doesn't over-retain.
    if let Err(err) = state.sync.flush_acks().await {
        tracing::warn!(error = %err, user_id, device_id, "ws teardown flush failed");
    }
}

fn row_to_op(row: &PgRow) -> SyncOp {
    SyncOp {
        id: row.get("id"),
        operation_id: row.get::<Uuid, _>("operation_id"),
        device_id: row.get("device_id"),
        lamport_ts: row.get("lamport_ts"),
        entity: row.get("entity"),
        entity_id: row.get("entity_id"),
        field: row.get("field"),
        op: row.get("op"),
        payload: row.get::<Option<serde_json::Value>, _>("payload"),
        created_at: row.get("created_at"),
        profile_canonical_id: row.get("profile_canonical_id"),
        hlc: Hlc {
            wall: row.get("hlc_wall"),
            logical: row.get("hlc_logical"),
        },
    }
}
