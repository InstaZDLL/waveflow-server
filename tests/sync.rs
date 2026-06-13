//! End-to-end tests for `/api/v1/sync/*` (Phase 1.f).
//!
//! Coverage map:
//!
//! - REST contract: push → pull round-trip, idempotent replay, lamport
//!   regression 409, oversized batch 400, empty `device_id` 400, pull
//!   never advances the cursor.
//! - ACK pipeline: `POST /sync/ack` lands the row only after the
//!   in-memory buffer is flushed; ack semantics are monotonic.
//! - Resurrected device: a pull with `since` below the compaction
//!   watermark returns 410 + the watermark.
//! - Compaction: collapse keeps only the latest op per (entity,
//!   entity_id, field), stale devices are skipped, watermark advances.
//! - Tenant isolation: user A's ops never reach user B's pull / WS.
//! - WebSocket fan-out: a push by one device reaches the other's
//!   socket in real time, scoped to the same user.

mod support;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_app_with_sync, spawn_authenticated, spawn_two_authenticated, JwksHarness};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;
use waveflow_server::sync::SyncHub;

fn op_payload(operation_id: Uuid, lamport_ts: i64, entity_id: &str, value: &str) -> Value {
    json!({
        "operation_id": operation_id,
        "lamport_ts": lamport_ts,
        "entity": "playlist",
        "entity_id": entity_id,
        "field": "name",
        "op": "set",
        "payload": { "value": value },
    })
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn push_then_pull_round_trip(pool: PgPool) {
    let auth = spawn_authenticated(pool, "round-trip").await;
    let op_id = Uuid::new_v4();

    let push: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [op_payload(op_id, 1, "pl-1", "Soirée")],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(push["accepted"].as_array().unwrap().len(), 1);
    let row = &push["accepted"][0];
    assert_eq!(row["operation_id"], json!(op_id));
    assert_eq!(row["device_id"], "device-a");
    let assigned_id = row["id"].as_i64().unwrap();
    assert!(assigned_id > 0);

    let pull: Value = reqwest::Client::new()
        .get(format!("{}/api/v1/sync/ops?since=0", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(pull["ops"].as_array().unwrap().len(), 1);
    assert_eq!(pull["ops"][0]["id"], assigned_id);
    assert_eq!(pull["last_id"], assigned_id);

    // Empty pull when `since` is already at the head.
    let empty: Value = reqwest::Client::new()
        .get(format!("{}/api/v1/sync/ops?since={assigned_id}", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(empty["ops"].as_array().unwrap().is_empty());
    assert_eq!(empty["last_id"], assigned_id);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn idempotent_replay_returns_same_id(pool: PgPool) {
    let auth = spawn_authenticated(pool, "idem-user").await;
    let op_id = Uuid::new_v4();
    let body = json!({
        "device_id": "device-a",
        "ops": [op_payload(op_id, 1, "pl-1", "First")],
    });

    let first: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let first_id = first["accepted"][0]["id"].as_i64().unwrap();

    let second: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&body)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let second_id = second["accepted"][0]["id"].as_i64().unwrap();
    assert_eq!(
        first_id, second_id,
        "idempotent replay must return the original row id"
    );

    // And only one row physically exists.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_op")
        .fetch_one(&auth.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn lamport_regression_returns_409_with_stored_max(pool: PgPool) {
    let auth = spawn_authenticated(pool, "lamport").await;

    let _: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [
                op_payload(Uuid::new_v4(), 10, "pl-1", "a"),
                op_payload(Uuid::new_v4(), 11, "pl-1", "b"),
            ],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    // Now replay an op at lamport_ts=10, which is already taken by a
    // DIFFERENT operation_id — the unique constraint on (user_id,
    // device_id, lamport_ts) fires and we expect 409 + stored_max=11.
    let resp = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [op_payload(Uuid::new_v4(), 10, "pl-2", "regression")],
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "lamport_regression");
    assert_eq!(body["device_id"], "device-a");
    assert_eq!(body["stored_max"], 11);
    assert_eq!(body["offending_lamport_ts"], 10);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn pull_does_not_advance_cursor(pool: PgPool) {
    // Deterministic harness — `SyncHub::for_tests` skips the 5 s
    // flusher, so the assertion below isn't racing a background task.
    let harness = std::sync::Arc::new(JwksHarness::spawn().await);
    let sync = SyncHub::for_tests(pool.clone());
    let base = spawn_app_with_sync(pool.clone(), harness.verifier_arc(), sync.clone()).await;
    let token = harness.mint(
        &support::good_claims("pull-readonly"),
        &support::header_with_kid(support::TEST_KID),
    );
    // Warm-up request lazy-provisions the user row.
    reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let _: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ops"))
        .bearer_auth(&token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [op_payload(Uuid::new_v4(), 1, "pl-1", "x")],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    // A pull. Read-only by contract — must not touch the cursor.
    let _: Value = reqwest::Client::new()
        .get(format!("{base}/api/v1/sync/ops?since=0"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    // Force-drain any pending ACK buffer so a stray write would show
    // up here. The pull above never recorded one, so the flush is a
    // no-op; the assertion is the real signal.
    sync.flush_acks().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM device_sync_cursor")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "GET /sync/ops must never advance the cursor");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn ack_writes_cursor_after_flush(pool: PgPool) {
    // Deterministic harness — drive `flush_acks` by hand so the
    // assertion isn't racing the 5 s loop.
    let harness = std::sync::Arc::new(JwksHarness::spawn().await);
    let sync = SyncHub::for_tests(pool.clone());
    let base = spawn_app_with_sync(pool.clone(), harness.verifier_arc(), sync.clone()).await;
    let token = harness.mint(
        &support::good_claims("ack-flush"),
        &support::header_with_kid(support::TEST_KID),
    );
    reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE external_id = 'ack-flush'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Land an op so there's an `id` to ACK at.
    let push: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ops"))
        .bearer_auth(&token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [op_payload(Uuid::new_v4(), 1, "pl-1", "x")],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let head_id = push["accepted"][0]["id"].as_i64().unwrap();

    let ack = reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ack"))
        .bearer_auth(&token)
        .json(&json!({ "device_id": "device-a", "last_seen_id": head_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(ack.status(), StatusCode::NO_CONTENT);

    // Force the flush — no sleeps, no polling.
    let flushed = sync.flush_acks().await.unwrap();
    assert_eq!(flushed, 1, "first flush must UPSERT the new cursor row");
    let last_seen: i64 = sqlx::query_scalar(
        "SELECT last_seen_id FROM device_sync_cursor \
         WHERE user_id = $1 AND device_id = 'device-a'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(last_seen, head_id);

    // Re-ACK at a lower id must NOT regress — the in-memory CAS
    // short-circuits without even marking the entry dirty, so the
    // second flush has nothing to write AND the cursor row is
    // unchanged.
    reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ack"))
        .bearer_auth(&token)
        .json(&json!({ "device_id": "device-a", "last_seen_id": 0 }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let flushed_after_regress = sync.flush_acks().await.unwrap();
    assert_eq!(
        flushed_after_regress, 0,
        "regressing ACK must not produce a flush write",
    );
    let still: i64 = sqlx::query_scalar(
        "SELECT last_seen_id FROM device_sync_cursor \
         WHERE user_id = $1 AND device_id = 'device-a'",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(still, head_id, "ACK must be monotonic");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn resurrected_device_returns_410(pool: PgPool) {
    let auth = spawn_authenticated(pool, "resurrected").await;

    // Manually plant a high watermark to simulate a compacted log
    // without having to actually compact. The whole point of this
    // test is the *guard*, not the compaction job.
    sqlx::query(
        "INSERT INTO sync_compaction_watermark (user_id, compacted_up_to, updated_at) \
         VALUES ($1, 100, 0)",
    )
    .bind(auth.user_id)
    .execute(&auth.pool)
    .await
    .unwrap();

    let resp = reqwest::Client::new()
        .get(format!("{}/api/v1/sync/ops?since=50", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "resync_required");
    assert_eq!(body["compacted_up_to"], 100);

    // `since=0` is the special "send me everything" case and must
    // *not* trip the guard — otherwise a fresh device would never
    // bootstrap.
    let ok = reqwest::Client::new()
        .get(format!("{}/api/v1/sync/ops?since=0", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn tenant_isolation_pull(pool: PgPool) {
    let two = spawn_two_authenticated(pool, "tenant-a", "tenant-b").await;

    let _: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", two.base))
        .bearer_auth(&two.a.token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [op_payload(Uuid::new_v4(), 1, "pl-a", "for A")],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    // User B's pull must NOT see user A's ops.
    let pull_b: Value = reqwest::Client::new()
        .get(format!("{}/api/v1/sync/ops?since=0", two.base))
        .bearer_auth(&two.b.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        pull_b["ops"].as_array().unwrap().is_empty(),
        "user B must not see user A's ops, got: {pull_b}"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn oversized_batch_is_rejected(pool: PgPool) {
    let auth = spawn_authenticated(pool, "oversized").await;
    let ops: Vec<Value> = (0..1025)
        .map(|i| op_payload(Uuid::new_v4(), i as i64 + 1, "pl-1", "x"))
        .collect();
    let resp = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({ "device_id": "device-a", "ops": ops }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn compaction_collapses_old_ops_and_advances_watermark(pool: PgPool) {
    // Manual control over both loops — push three SET ops over the
    // same (entity, entity_id, field), ACK to the head, run
    // `compact_once` exactly once, and observe only the latest op
    // survives at or below the watermark.
    let harness = std::sync::Arc::new(JwksHarness::spawn().await);
    let sync = SyncHub::for_tests(pool.clone());
    let base = spawn_app_with_sync(pool.clone(), harness.verifier_arc(), sync.clone()).await;
    let token = harness.mint(
        &support::good_claims("compaction"),
        &support::header_with_kid(support::TEST_KID),
    );

    // Provision the user via the warm-up request.
    let _ = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE external_id = 'compaction'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Push three superseding SETs on the same field, plus an
    // unrelated entity op that compaction must leave alone.
    for lamport in 1..=3 {
        let _: Value = reqwest::Client::new()
            .post(format!("{base}/api/v1/sync/ops"))
            .bearer_auth(&token)
            .json(&json!({
                "device_id": "device-a",
                "ops": [op_payload(Uuid::new_v4(), lamport, "pl-keep", &format!("v{lamport}"))],
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
    }
    let _: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ops"))
        .bearer_auth(&token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [op_payload(Uuid::new_v4(), 4, "pl-other", "untouched")],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    let head_id: i64 = sqlx::query_scalar("SELECT MAX(id) FROM sync_op")
        .fetch_one(&pool)
        .await
        .unwrap();

    // ACK to the head. Buffered — flush by running `compact_once`,
    // which calls `flush_acks` internally before reading the MIN.
    let ack = reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ack"))
        .bearer_auth(&token)
        .json(&json!({ "device_id": "device-a", "last_seen_id": head_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(ack.status(), StatusCode::NO_CONTENT);

    let report = sync.compact_once().await.unwrap();
    assert_eq!(report.users_compacted, 1);
    assert_eq!(report.rows_deleted, 2, "two superseded SETs collapse away");

    // The latest `pl-keep` SET + the `pl-other` op remain.
    let remaining: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, entity_id, payload->>'value' FROM sync_op ORDER BY id ASC")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(remaining.len(), 2, "got {remaining:?}");
    assert_eq!(remaining[0].1, "pl-keep");
    assert_eq!(remaining[0].2, "v3", "only the latest SET survives");
    assert_eq!(remaining[1].1, "pl-other");

    // Watermark equals the MIN we computed (= head_id, the only
    // device is fully caught up).
    let watermark: i64 = sqlx::query_scalar(
        "SELECT compacted_up_to FROM sync_compaction_watermark WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(watermark, head_id);

    // Second compaction with no new ACKs is a no-op (no rows
    // deleted, watermark unchanged).
    let report2 = sync.compact_once().await.unwrap();
    assert_eq!(report2.rows_deleted, 0);
    let watermark2: i64 = sqlx::query_scalar(
        "SELECT compacted_up_to FROM sync_compaction_watermark WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(watermark2, head_id, "watermark must be monotonic");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn compaction_skips_stale_devices(pool: PgPool) {
    let harness = std::sync::Arc::new(JwksHarness::spawn().await);
    let sync = SyncHub::for_tests(pool.clone());
    let base = spawn_app_with_sync(pool.clone(), harness.verifier_arc(), sync.clone()).await;
    let token = harness.mint(
        &support::good_claims("stale"),
        &support::header_with_kid(support::TEST_KID),
    );
    let _ = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let user_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE external_id = 'stale'")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Push three superseding ops as one device.
    for lamport in 1..=3 {
        let _: Value = reqwest::Client::new()
            .post(format!("{base}/api/v1/sync/ops"))
            .bearer_auth(&token)
            .json(&json!({
                "device_id": "device-a",
                "ops": [op_payload(Uuid::new_v4(), lamport, "pl-1", &format!("v{lamport}"))],
            }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
    }
    let head_id: i64 = sqlx::query_scalar("SELECT MAX(id) FROM sync_op")
        .fetch_one(&pool)
        .await
        .unwrap();

    // Device A is up to date.
    let _ = reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ack"))
        .bearer_auth(&token)
        .json(&json!({ "device_id": "device-a", "last_seen_id": head_id }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    // Plant a STALE cursor for device-stale (200 days ago) at id 1 —
    // if the cutoff weren't applied, the MIN would be 1 and nothing
    // would compact. We expect the compactor to skip this row.
    let stale_ts = chrono::Utc::now().timestamp_millis() - 200 * 24 * 60 * 60 * 1000;
    sqlx::query(
        "INSERT INTO device_sync_cursor (user_id, device_id, last_seen_id, last_seen_at) \
         VALUES ($1, 'device-stale', 1, $2)",
    )
    .bind(user_id)
    .bind(stale_ts)
    .execute(&pool)
    .await
    .unwrap();

    let report = sync.compact_once().await.unwrap();
    assert_eq!(
        report.rows_deleted, 2,
        "compaction must ignore the stale device and collapse using device-a's MIN",
    );
    let watermark: i64 = sqlx::query_scalar(
        "SELECT compacted_up_to FROM sync_compaction_watermark WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(watermark, head_id);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn websocket_fans_out_to_same_tenant(pool: PgPool) {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let auth = spawn_authenticated(pool, "ws-user").await;

    // Reach the WS endpoint over plain `ws://`. The bearer + query
    // string identify the device.
    let ws_url = format!(
        "ws://{}/api/v1/sync/ws?device_id=ws-device",
        auth.base.trim_start_matches("http://"),
    );
    let mut req = ws_url.into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {}", auth.token).parse().unwrap(),
    );
    let (mut socket, _resp) = tokio_tungstenite::connect_async(req)
        .await
        .expect("ws upgrade failed");

    // Push from a different device under the same user — the WS
    // session should receive the op envelope.
    let op_id = Uuid::new_v4();
    let _: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({
            "device_id": "pusher",
            "ops": [op_payload(op_id, 1, "pl-1", "from-ws-test")],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    // Receive — bail on a 1 s timeout so a regression doesn't hang CI.
    let msg = tokio::time::timeout(Duration::from_secs(1), socket.next())
        .await
        .expect("ws frame timed out")
        .expect("ws stream ended")
        .expect("ws frame error");
    let text = match msg {
        Message::Text(t) => t.to_string(),
        other => panic!("expected text frame, got {other:?}"),
    };
    let payload: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(payload["type"], "op");
    assert_eq!(payload["op"]["operation_id"], json!(op_id));
    assert_eq!(payload["op"]["device_id"], "pusher");

    // Close cleanly so the on-disconnect flush runs.
    socket.send(Message::Close(None)).await.ok();
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn websocket_isolates_other_tenants(pool: PgPool) {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;

    let two = spawn_two_authenticated(pool, "ws-a", "ws-b").await;

    // User B subscribes; user A pushes — B must not receive the frame.
    let ws_url = format!(
        "ws://{}/api/v1/sync/ws?device_id=b-device",
        two.base.trim_start_matches("http://"),
    );
    let mut req = ws_url.into_client_request().unwrap();
    req.headers_mut().insert(
        "authorization",
        format!("Bearer {}", two.b.token).parse().unwrap(),
    );
    let (mut socket, _resp) = tokio_tungstenite::connect_async(req)
        .await
        .expect("ws upgrade failed");

    // Push as user A.
    let _: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", two.base))
        .bearer_auth(&two.a.token)
        .json(&json!({
            "device_id": "a-pusher",
            "ops": [op_payload(Uuid::new_v4(), 1, "pl-leak", "should-not-reach-b")],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    // Wait 500 ms for the broadcast to (not) arrive. A successful
    // recv would fail the test.
    match tokio::time::timeout(Duration::from_millis(500), socket.next()).await {
        Err(_timeout) => { /* expected — nothing crossed */ }
        Ok(Some(Ok(Message::Close(_)))) | Ok(None) => { /* socket died, also fine */ }
        Ok(other) => panic!("tenant B's WS received a frame from tenant A's push: {other:?}"),
    }
    socket.send(Message::Close(None)).await.ok();
}

// RFC-003 Phase A.2 — v2 wire shape round-trip. A v1 client sending
// only `lamport_ts` should pull back `hlc: { wall: 0, logical:
// lamport_ts }` (the A.1.1 backfill shape). A v2 client sending an
// explicit `hlc` should pull it back verbatim. Mixing the two in one
// batch lands each on its own path without cross-contamination.

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn push_v1_only_pulls_derived_hlc(pool: PgPool) {
    let auth = spawn_authenticated(pool, "hlc-v1").await;

    let _push: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [op_payload(Uuid::new_v4(), 42, "pl-1", "Soirée")],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    let pull: Value = reqwest::Client::new()
        .get(format!("{}/api/v1/sync/ops?since=0", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    let row = &pull["ops"][0];
    // Backfill derivation: (0, lamport_ts).
    assert_eq!(row["hlc"]["wall"], 0);
    assert_eq!(row["hlc"]["logical"], 42);
    assert_eq!(row["lamport_ts"], 42);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn push_v2_hlc_round_trips_verbatim(pool: PgPool) {
    let auth = spawn_authenticated(pool, "hlc-v2").await;

    let _push: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [{
                "operation_id": Uuid::new_v4(),
                "lamport_ts": 1,
                "entity": "playlist",
                "entity_id": "pl-1",
                "field": "name",
                "op": "set",
                "payload": { "value": "Live" },
                "hlc": { "wall": 1_700_000_000_000_i64, "logical": 7 },
            }],
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    let pull: Value = reqwest::Client::new()
        .get(format!("{}/api/v1/sync/ops?since=0", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    let row = &pull["ops"][0];
    assert_eq!(row["hlc"]["wall"], 1_700_000_000_000_i64);
    assert_eq!(row["hlc"]["logical"], 7);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn push_v2_negative_wall_rejected(pool: PgPool) {
    let auth = spawn_authenticated(pool, "hlc-neg-wall").await;

    let res = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [{
                "operation_id": Uuid::new_v4(),
                "lamport_ts": 1,
                "entity": "playlist",
                "entity_id": "pl-1",
                "field": "name",
                "op": "set",
                "payload": { "value": "x" },
                "hlc": { "wall": -1_i64, "logical": 1 },
            }],
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn push_v2_duplicate_hlc_returns_hlc_regression(pool: PgPool) {
    let auth = spawn_authenticated(pool, "hlc-dup").await;

    let push = |op_id: Uuid, lamport: i64, hlc_logical: i32| {
        let base = auth.base.clone();
        let token = auth.token.clone();
        async move {
            reqwest::Client::new()
                .post(format!("{base}/api/v1/sync/ops"))
                .bearer_auth(&token)
                .json(&json!({
                    "device_id": "device-a",
                    "ops": [{
                        "operation_id": op_id,
                        "lamport_ts": lamport,
                        "entity": "playlist",
                        "entity_id": "pl-1",
                        "field": "name",
                        "op": "set",
                        "payload": { "value": "x" },
                        "hlc": { "wall": 1_700_000_000_000_i64, "logical": hlc_logical },
                    }],
                }))
                .send()
                .await
                .unwrap()
        }
    };

    let first = push(Uuid::new_v4(), 1, 7).await;
    assert_eq!(first.status(), StatusCode::OK);

    // Different operation_id (so the ON CONFLICT short-circuit doesn't
    // absorb it) + lamport advanced (so the legacy lamport UNIQUE
    // can't fire) + SAME hlc pair → must hit the HLC UNIQUE.
    let collide = push(Uuid::new_v4(), 2, 7).await;
    assert_eq!(collide.status(), StatusCode::CONFLICT);
    let body: Value = collide.json().await.unwrap();
    assert_eq!(body["error"], "hlc_regression");
    assert_eq!(body["device_id"], "device-a");
    assert_eq!(body["offending_hlc"]["wall"], 1_700_000_000_000_i64);
    assert_eq!(body["offending_hlc"]["logical"], 7);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn push_v2_negative_logical_rejected(pool: PgPool) {
    let auth = spawn_authenticated(pool, "hlc-neg-logical").await;

    let res = reqwest::Client::new()
        .post(format!("{}/api/v1/sync/ops", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({
            "device_id": "device-a",
            "ops": [{
                "operation_id": Uuid::new_v4(),
                "lamport_ts": 1,
                "entity": "playlist",
                "entity_id": "pl-1",
                "field": "name",
                "op": "set",
                "payload": { "value": "x" },
                "hlc": { "wall": 1_700_000_000_000_i64, "logical": -1_i32 },
            }],
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}
