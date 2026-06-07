//! End-to-end tests for the Phase 1.g.0 apply pipeline.
//!
//! Each test pushes one or more ops to `/api/v1/sync/ops` and asserts
//! that the entity rows in `playlist` / `library` / `user_liked_track`
//! / `user_track_rating` reflect the op semantics. The apply path
//! runs inside the same transaction as the durable insert, so a
//! successful push response is the contract that materialisation
//! happened.

mod support;

use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_authenticated, spawn_two_authenticated};
use uuid::Uuid;

/// Profile UUID reused across the suite. Most tests need exactly one
/// profile and care about its server id, not its identity.
const PROFILE_CID: &str = "prof-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
const PROFILE_CID_B: &str = "prof-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

// Test helper — 8 args is one over the clippy bound. Folding two
// into a struct would obscure the call sites, all of which read
// like a wire-format dump. Allow locally rather than build a
// builder for a fixture function.
#[allow(clippy::too_many_arguments)]
fn op(
    operation_id: Uuid,
    lamport_ts: i64,
    entity: &str,
    entity_id: &str,
    field: Option<&str>,
    op_kind: &str,
    payload: Value,
    profile_canonical_id: Option<&str>,
) -> Value {
    let mut row = json!({
        "operation_id": operation_id,
        "lamport_ts": lamport_ts,
        "entity": entity,
        "entity_id": entity_id,
        "op": op_kind,
        "payload": payload,
    });
    if let Some(f) = field {
        row["field"] = json!(f);
    }
    if let Some(p) = profile_canonical_id {
        row["profile_canonical_id"] = json!(p);
    }
    row
}

async fn push(base: &str, token: &str, ops: &[Value]) -> reqwest::StatusCode {
    reqwest::Client::new()
        .post(format!("{base}/api/v1/sync/ops"))
        .bearer_auth(token)
        .json(&json!({ "device_id": "device-a", "ops": ops }))
        .send()
        .await
        .unwrap()
        .status()
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_insert_materialises_row(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-pl-ins").await;
    let playlist_cid = "pl-1111";

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "playlist",
            playlist_cid,
            None,
            "insert",
            json!({ "name": "Soirée", "description": "weekend", "color_id": "rose", "icon_id": "flame" }),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(status.is_success(), "push failed: {status}");

    let row: (String, Option<String>, String, String, i64) = sqlx::query_as(
        "SELECT name, description, color_id, icon_id, profile_id FROM playlist WHERE canonical_id = $1",
    )
    .bind(playlist_cid)
    .fetch_one(&pool)
    .await
    .expect("playlist row not materialised");

    assert_eq!(row.0, "Soirée");
    assert_eq!(row.1.as_deref(), Some("weekend"));
    assert_eq!(row.2, "rose");
    assert_eq!(row.3, "flame");

    // Profile auto-provisioned under the expected canonical id +
    // user_id.
    let profile_user: (i64,) = sqlx::query_as("SELECT user_id FROM profile WHERE id = $1")
        .bind(row.4)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(profile_user.0, auth.user_id);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_insert_replay_is_idempotent(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-pl-replay").await;
    let playlist_cid = "pl-replay";
    let op_id = Uuid::new_v4();

    let body = op(
        op_id,
        1,
        "playlist",
        playlist_cid,
        None,
        "insert",
        json!({ "name": "Initial", "color_id": "violet", "icon_id": "music" }),
        Some(PROFILE_CID),
    );

    assert!(push(&auth.base, &auth.token, std::slice::from_ref(&body))
        .await
        .is_success());
    // Replay: same operation_id → durable log absorbs via ON CONFLICT,
    // apply path runs only on freshly-inserted rows, so the playlist
    // stays at exactly one row.
    assert!(push(&auth.base, &auth.token, std::slice::from_ref(&body))
        .await
        .is_success());

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM playlist WHERE canonical_id = $1")
        .bind(playlist_cid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 1);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_set_name_updates_field(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-pl-rename").await;
    let playlist_cid = "pl-rename";

    let insert = op(
        Uuid::new_v4(),
        1,
        "playlist",
        playlist_cid,
        None,
        "insert",
        json!({ "name": "Old name" }),
        Some(PROFILE_CID),
    );
    let rename = op(
        Uuid::new_v4(),
        2,
        "playlist",
        playlist_cid,
        Some("name"),
        "set",
        json!({ "value": "New name" }),
        Some(PROFILE_CID),
    );

    assert!(push(&auth.base, &auth.token, &[insert, rename])
        .await
        .is_success());

    let name: (String,) = sqlx::query_as("SELECT name FROM playlist WHERE canonical_id = $1")
        .bind(playlist_cid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(name.0, "New name");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_delete_removes_row(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-pl-del").await;
    let playlist_cid = "pl-del";

    let insert = op(
        Uuid::new_v4(),
        1,
        "playlist",
        playlist_cid,
        None,
        "insert",
        json!({ "name": "Doomed" }),
        Some(PROFILE_CID),
    );
    let delete = op(
        Uuid::new_v4(),
        2,
        "playlist",
        playlist_cid,
        None,
        "delete",
        Value::Null,
        Some(PROFILE_CID),
    );

    assert!(push(&auth.base, &auth.token, &[insert, delete])
        .await
        .is_success());

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM playlist WHERE canonical_id = $1")
        .bind(playlist_cid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_insert_tracks_materialises_rows_without_snapshot(pool: PgPool) {
    // Pre-1.j.b desktop wire — emits `payload.track_ids` only,
    // no `payload.snapshots`. Rows must land in `playlist_track`
    // with NULL snapshot fields. Position is auto-assigned starting
    // at 0.
    let auth = spawn_authenticated(pool.clone(), "apply-pl-tracks-ins").await;
    let playlist_cid = "pl-tracks-bare";

    let insert_playlist = op(
        Uuid::new_v4(),
        1,
        "playlist",
        playlist_cid,
        None,
        "insert",
        json!({ "name": "Mix" }),
        Some(PROFILE_CID),
    );
    let insert_tracks = op(
        Uuid::new_v4(),
        2,
        "playlist",
        playlist_cid,
        Some("tracks"),
        "insert",
        json!({ "track_ids": [101, 102, 103] }),
        Some(PROFILE_CID),
    );

    assert!(
        push(&auth.base, &auth.token, &[insert_playlist, insert_tracks])
            .await
            .is_success()
    );

    let rows: Vec<(i64, i32, Option<String>)> = sqlx::query_as(
        "SELECT pt.track_id, pt.position, pt.snapshot_title
           FROM playlist_track pt
           JOIN playlist p ON p.id = pt.playlist_id
          WHERE p.canonical_id = $1
          ORDER BY pt.position ASC",
    )
    .bind(playlist_cid)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0], (101, 0, None));
    assert_eq!(rows[1], (102, 1, None));
    assert_eq!(rows[2], (103, 2, None));
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_insert_tracks_carries_snapshot(pool: PgPool) {
    // Post-1.j.b desktop wire — payload now ships `snapshots` keyed
    // by track id (as string). Snapshot fields must land on the row
    // so the public share preview can render the title + artist.
    let auth = spawn_authenticated(pool.clone(), "apply-pl-tracks-snap").await;
    let playlist_cid = "pl-tracks-snap";

    let insert_playlist = op(
        Uuid::new_v4(),
        1,
        "playlist",
        playlist_cid,
        None,
        "insert",
        json!({ "name": "Soirée" }),
        Some(PROFILE_CID),
    );
    let insert_tracks = op(
        Uuid::new_v4(),
        2,
        "playlist",
        playlist_cid,
        Some("tracks"),
        "insert",
        json!({
            "track_ids": [201, 202],
            "snapshots": {
                "201": { "title": "Daft", "artist": "Punk", "duration_ms": 222000 },
                "202": { "title": "Around the World", "duration_ms": 280000 }
            }
        }),
        Some(PROFILE_CID),
    );

    assert!(
        push(&auth.base, &auth.token, &[insert_playlist, insert_tracks])
            .await
            .is_success()
    );

    // Tuple type alias keeps clippy's "very complex type" lint
    // satisfied while still letting the test read like a wire dump.
    type SnapshotRow = (i64, Option<String>, Option<String>, Option<i64>);
    let rows: Vec<SnapshotRow> = sqlx::query_as(
        "SELECT pt.track_id, pt.snapshot_title, pt.snapshot_artist, pt.snapshot_duration_ms
           FROM playlist_track pt
           JOIN playlist p ON p.id = pt.playlist_id
          WHERE p.canonical_id = $1
          ORDER BY pt.position ASC",
    )
    .bind(playlist_cid)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0],
        (201, Some("Daft".into()), Some("Punk".into()), Some(222000))
    );
    assert_eq!(
        rows[1],
        (202, Some("Around the World".into()), None, Some(280000))
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_delete_tracks_drops_rows(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-pl-tracks-del").await;
    let playlist_cid = "pl-tracks-del";

    let insert_playlist = op(
        Uuid::new_v4(),
        1,
        "playlist",
        playlist_cid,
        None,
        "insert",
        json!({ "name": "Doomed" }),
        Some(PROFILE_CID),
    );
    let insert_tracks = op(
        Uuid::new_v4(),
        2,
        "playlist",
        playlist_cid,
        Some("tracks"),
        "insert",
        json!({ "track_ids": [301, 302, 303] }),
        Some(PROFILE_CID),
    );
    let delete_tracks = op(
        Uuid::new_v4(),
        3,
        "playlist",
        playlist_cid,
        Some("tracks"),
        "delete",
        json!({ "track_ids": [302] }),
        Some(PROFILE_CID),
    );

    assert!(push(
        &auth.base,
        &auth.token,
        &[insert_playlist, insert_tracks, delete_tracks]
    )
    .await
    .is_success());

    let remaining: Vec<(i64,)> = sqlx::query_as(
        "SELECT pt.track_id FROM playlist_track pt
           JOIN playlist p ON p.id = pt.playlist_id
          WHERE p.canonical_id = $1
          ORDER BY pt.position ASC",
    )
    .bind(playlist_cid)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        remaining.iter().map(|(t,)| *t).collect::<Vec<_>>(),
        vec![301, 303],
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_set_tracks_reorders_position(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-pl-tracks-reorder").await;
    let playlist_cid = "pl-tracks-reorder";

    let insert_playlist = op(
        Uuid::new_v4(),
        1,
        "playlist",
        playlist_cid,
        None,
        "insert",
        json!({ "name": "Reorderable" }),
        Some(PROFILE_CID),
    );
    let insert_tracks = op(
        Uuid::new_v4(),
        2,
        "playlist",
        playlist_cid,
        Some("tracks"),
        "insert",
        json!({ "track_ids": [401, 402, 403] }),
        Some(PROFILE_CID),
    );
    // Move track 403 from position 2 to position 0.
    let reorder = op(
        Uuid::new_v4(),
        3,
        "playlist",
        playlist_cid,
        Some("tracks"),
        "set",
        json!({ "track_id": 403, "position": 0 }),
        Some(PROFILE_CID),
    );

    assert!(push(
        &auth.base,
        &auth.token,
        &[insert_playlist, insert_tracks, reorder]
    )
    .await
    .is_success());

    let pos_403: i32 = sqlx::query_scalar(
        "SELECT pt.position FROM playlist_track pt
           JOIN playlist p ON p.id = pt.playlist_id
          WHERE p.canonical_id = $1 AND pt.track_id = $2",
    )
    .bind(playlist_cid)
    .bind(403i64)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        pos_403, 0,
        "reorder must move the row to the target position"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_insert_tracks_without_parent_is_skipped(pool: PgPool) {
    // Tracks op for a playlist whose `insert` op didn't land first
    // (or got dropped). The apply pipeline should `Skipped` rather
    // than error so the durable log keeps the op for a future
    // replay. Push still returns success (apply outcome is
    // telemetry-only).
    let auth = spawn_authenticated(pool.clone(), "apply-pl-tracks-orphan").await;

    let orphan_tracks = op(
        Uuid::new_v4(),
        1,
        "playlist",
        "pl-orphan",
        Some("tracks"),
        "insert",
        json!({ "track_ids": [501, 502] }),
        Some(PROFILE_CID),
    );

    let status = push(&auth.base, &auth.token, &[orphan_tracks]).await;
    assert!(
        status.is_success(),
        "skipped apply should still 2xx the push: {status}",
    );

    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM playlist_track WHERE track_id IN (501, 502)")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count.0, 0, "no rows when the parent playlist is missing");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn library_insert_materialises_row(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-lib-ins").await;
    let library_cid = "lib-soundtracks";

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            library_cid,
            None,
            "insert",
            json!({ "name": "Bandes-son" }),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(status.is_success());

    let row: (String, String, String) =
        sqlx::query_as("SELECT name, color_id, icon_id FROM library WHERE canonical_id = $1")
            .bind(library_cid)
            .fetch_one(&pool)
            .await
            .expect("library row not materialised");

    assert_eq!(row.0, "Bandes-son");
    // Library defaults differ from playlist (emerald / library).
    assert_eq!(row.1, "emerald");
    assert_eq!(row.2, "library");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn rating_set_then_delete(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-rating").await;
    let file_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    let set = op(
        Uuid::new_v4(),
        1,
        "track_rating",
        file_hash,
        None,
        "set",
        json!({ "value": 196 }),
        None, // rating ops don't need profile_canonical_id
    );
    assert!(push(&auth.base, &auth.token, &[set]).await.is_success());

    let r: (i64,) = sqlx::query_as(
        "SELECT rating FROM user_track_rating WHERE user_id = $1 AND file_hash = $2",
    )
    .bind(auth.user_id)
    .bind(file_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(r.0, 196);

    let clear = op(
        Uuid::new_v4(),
        2,
        "track_rating",
        file_hash,
        None,
        "delete",
        Value::Null,
        None,
    );
    assert!(push(&auth.base, &auth.token, &[clear]).await.is_success());

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM user_track_rating WHERE user_id = $1 AND file_hash = $2",
    )
    .bind(auth.user_id)
    .bind(file_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 0);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn rating_out_of_range_is_400(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-rating-bad").await;

    let bad = op(
        Uuid::new_v4(),
        1,
        "track_rating",
        "deadbeef",
        None,
        "set",
        json!({ "value": 999 }),
        None,
    );
    let status = push(&auth.base, &auth.token, &[bad]).await;
    // Apply errors short-circuit the push handler with 500 — see the
    // rollback path in `api::sync::push_ops`. A future iteration may
    // map ApplyError::InvalidPayload to 400; pinning the current
    // behaviour here so the change is intentional.
    assert!(status.is_server_error());

    // Durable log was rolled back too.
    let count: (i64,) = sqlx::query_scalar("SELECT COUNT(*) FROM sync_op WHERE user_id = $1")
        .bind(auth.user_id)
        .fetch_one(&pool)
        .await
        .map(|c: i64| (c,))
        .unwrap();
    assert_eq!(count.0, 0);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_insert_with_wrong_type_for_optional_string_is_rejected(pool: PgPool) {
    // A desktop bug that pushes a number where a string is expected
    // for `description` must error explicitly, not silently coerce
    // to NULL — otherwise a syntax glitch on the client side reads
    // as "clear this field" on the server.
    let auth = spawn_authenticated(pool.clone(), "apply-wrong-type").await;

    let bad = op(
        Uuid::new_v4(),
        1,
        "playlist",
        "pl-wrong-type",
        None,
        "insert",
        json!({ "name": "ok", "description": 42 }),
        Some(PROFILE_CID),
    );

    let status = push(&auth.base, &auth.token, &[bad]).await;
    assert!(status.is_server_error());

    // Durable log rolled back too — no half-applied state.
    let log_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sync_op WHERE user_id = $1")
        .bind(auth.user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(log_count.0, 0);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn liked_track_insert_then_delete(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-liked").await;
    let file_hash = "feedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedfacefeedface";

    let like = op(
        Uuid::new_v4(),
        1,
        "liked_track",
        file_hash,
        None,
        "insert",
        Value::Null,
        None,
    );
    assert!(push(&auth.base, &auth.token, &[like]).await.is_success());

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM user_liked_track WHERE user_id = $1 AND file_hash = $2",
    )
    .bind(auth.user_id)
    .bind(file_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 1);

    let unlike = op(
        Uuid::new_v4(),
        2,
        "liked_track",
        file_hash,
        None,
        "delete",
        Value::Null,
        None,
    );
    assert!(push(&auth.base, &auth.token, &[unlike]).await.is_success());

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM user_liked_track WHERE user_id = $1 AND file_hash = $2",
    )
    .bind(auth.user_id)
    .bind(file_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 0);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn missing_profile_canonical_id_skips_apply_but_keeps_log(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-no-profile").await;
    let playlist_cid = "pl-orphan";

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "playlist",
            playlist_cid,
            None,
            "insert",
            json!({ "name": "Orphan" }),
            None, // no profile_canonical_id
        )],
    )
    .await;
    assert!(status.is_success());

    // Log row stored.
    let log_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM sync_op WHERE user_id = $1")
        .bind(auth.user_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(log_count.0, 1);

    // Entity NOT materialised.
    let row_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM playlist WHERE canonical_id = $1")
        .bind(playlist_cid)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row_count.0, 0);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn two_canonical_ids_land_in_distinct_profiles(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-multi-profile").await;

    let pa = op(
        Uuid::new_v4(),
        1,
        "playlist",
        "pl-a",
        None,
        "insert",
        json!({ "name": "Profile A" }),
        Some(PROFILE_CID),
    );
    let pb = op(
        Uuid::new_v4(),
        2,
        "playlist",
        "pl-b",
        None,
        "insert",
        json!({ "name": "Profile B" }),
        Some(PROFILE_CID_B),
    );

    assert!(push(&auth.base, &auth.token, &[pa, pb]).await.is_success());

    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM profile WHERE user_id = $1 AND canonical_id IS NOT NULL",
    )
    .bind(auth.user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 2);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn other_user_cannot_see_apply_rows(pool: PgPool) {
    let two = spawn_two_authenticated(pool.clone(), "apply-alice", "apply-bob").await;

    // Alice creates a playlist via apply.
    let alice_pl = op(
        Uuid::new_v4(),
        1,
        "playlist",
        "pl-alice",
        None,
        "insert",
        json!({ "name": "Alice's mix" }),
        Some(PROFILE_CID),
    );
    assert!(push(&two.base, &two.a.token, &[alice_pl])
        .await
        .is_success());

    // Bob's profile / playlist counts stay zero — the apply pipeline
    // routes by user_id and canonical_id together.
    let bob_profiles: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM profile WHERE user_id = $1 AND canonical_id IS NOT NULL",
    )
    .bind(two.b.user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bob_profiles.0, 0);

    let bob_playlists: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM playlist p \
         JOIN profile pr ON pr.id = p.profile_id \
         WHERE pr.user_id = $1",
    )
    .bind(two.b.user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(bob_playlists.0, 0);
}

// ================================================================
// Phase 4.d.0.2 — track sync entity tests
// ================================================================

/// Push a library insert via sync_ops so the apply pipeline
/// materialises the library row (so track ops can resolve
/// `library_canonical_id` to a server `library.id`). Returns the
/// canonical id passed in for chaining.
async fn materialise_library(
    base: &str,
    token: &str,
    library_cid: &str,
    name: &str,
) -> reqwest::StatusCode {
    push(
        base,
        token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            library_cid,
            None,
            "insert",
            json!({ "name": name }),
            Some(PROFILE_CID),
        )],
    )
    .await
}

/// Build the canonical track-insert payload. Pulls every required
/// field plus the album + artist metadata that 4.d.0.2 added.
/// `file_hash` rides as a payload field — `entity_id` is the
/// file_path (see `apply.rs::track` module banner).
#[allow(clippy::too_many_arguments)]
fn track_insert_payload(
    library_cid: &str,
    title: &str,
    file_hash: &str,
    album_title: Option<&str>,
    album_artist_name: Option<&str>,
    is_compilation: bool,
    artists: &[&str],
) -> Value {
    json!({
        "library_canonical_id": library_cid,
        "title": title,
        "file_hash": file_hash,
        "file_size": 12_345_678,
        "duration_ms": 320_000,
        "track_number": 1,
        "disc_number": 1,
        "year": 2001,
        "bitrate": 1_411_000,
        "sample_rate": 44_100,
        "channels": 2,
        "bit_depth": 16,
        "codec": "flac",
        "added_at": 1_700_000_000_000_i64,
        "album_title": album_title,
        "album_artist_name": album_artist_name,
        "is_compilation": is_compilation,
        "artists": artists,
    })
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_creates_track_album_and_artists(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-track-ins").await;
    let library_cid = "lib-track-ins";
    let file_path = "/music/discovery/01.flac";
    let file_hash = "blake3-aaaaaaaa";

    assert!(
        materialise_library(&auth.base, &auth.token, library_cid, "Library A")
            .await
            .is_success()
    );

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "One More Time",
                file_hash,
                Some("Discovery"),
                Some("Daft Punk"),
                false,
                &["Daft Punk"],
            ),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(status.is_success(), "push failed: {status}");

    // Track row materialised + linked to its album.
    let track: (String, String, i64, Option<i64>, Option<String>) = sqlx::query_as(
        "SELECT title, file_path, file_size, album_id, file_hash FROM track WHERE file_path = $1",
    )
    .bind(file_path)
    .fetch_one(&pool)
    .await
    .expect("track row not materialised");
    assert_eq!(
        track.4.as_deref(),
        Some(file_hash),
        "file_hash from payload must land in the row"
    );
    assert_eq!(track.0, "One More Time");
    assert_eq!(track.1, "/music/discovery/01.flac");
    assert_eq!(track.2, 12_345_678);
    assert!(
        track.3.is_some(),
        "track.album_id must be set when album_title is provided"
    );

    // Album row materialised with the resolved album_artist_id.
    let album: (String, Option<i64>, Option<i64>, bool) = sqlx::query_as(
        "SELECT canonical_title, album_artist_id, year, is_compilation
           FROM album WHERE id = $1",
    )
    .bind(track.3.unwrap())
    .fetch_one(&pool)
    .await
    .expect("album row not materialised");
    assert_eq!(album.0, "Discovery");
    assert!(
        album.1.is_some(),
        "album.album_artist_id must resolve to an artist row"
    );
    assert_eq!(album.2, Some(2001));
    assert!(!album.3, "is_compilation must be false here");

    // Artist row(s) — both the album artist AND the contributor
    // list dedupe to a single row because they match the same name.
    let artist_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artist WHERE name = $1")
        .bind("Daft Punk")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        artist_count, 1,
        "album_artist + contributor with the same name MUST dedupe to one row"
    );

    // track_artist link minted with the correct position.
    let link: (i64, i64, i32) = sqlx::query_as(
        "SELECT ta.track_id, ta.artist_id, ta.position
           FROM track_artist ta WHERE ta.track_id = (SELECT id FROM track WHERE file_path = $1)",
    )
    .bind(file_path)
    .fetch_one(&pool)
    .await
    .expect("track_artist link not minted");
    assert_eq!(link.2, 0, "single artist must land at position 0");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_multi_artist_preserves_order(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-track-multi").await;
    let library_cid = "lib-multi";
    let file_path = "/music/ram/05.flac";

    materialise_library(&auth.base, &auth.token, library_cid, "L").await;

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Get Lucky",
                "blake3-bbbbbbbb",
                Some("Random Access Memories"),
                Some("Daft Punk"),
                false,
                &["Daft Punk", "Pharrell Williams", "Nile Rodgers"],
            ),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(status.is_success(), "push failed: {status}");

    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT a.name, ta.position
           FROM track_artist ta
           JOIN artist a ON a.id = ta.artist_id
          WHERE ta.track_id = (SELECT id FROM track WHERE file_path = $1)
          ORDER BY ta.position ASC",
    )
    .bind(file_path)
    .fetch_all(&pool)
    .await
    .expect("track_artist select");
    assert_eq!(
        rows,
        vec![
            ("Daft Punk".to_owned(), 0),
            ("Pharrell Williams".to_owned(), 1),
            ("Nile Rodgers".to_owned(), 2),
        ],
        "multi-artist position MUST match the wire-shape array order"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_compilation_with_null_album_artist(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-track-comp").await;
    let library_cid = "lib-comp";
    let file_path = "/music/now32/03.mp3";

    materialise_library(&auth.base, &auth.token, library_cid, "L").await;

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Mr. Brightside",
                "blake3-cccccccc",
                Some("Now That's What I Call Music! 32"),
                None, // compilation → no single album artist
                true,
                &["The Killers"],
            ),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(status.is_success(), "push failed: {status}");

    let album: (Option<i64>, bool) = sqlx::query_as(
        "SELECT album_artist_id, is_compilation FROM album
          WHERE canonical_title = $1",
    )
    .bind("Now That's What I Call Music! 32")
    .fetch_one(&pool)
    .await
    .expect("compilation album row");
    assert!(
        album.0.is_none(),
        "compilation album_artist_id must be NULL"
    );
    assert!(album.1, "is_compilation flag must be true");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_replay_is_idempotent_and_dedups_album_artist(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-track-replay").await;
    let library_cid = "lib-replay";
    let file_path = "/music/disco/02.flac";

    materialise_library(&auth.base, &auth.token, library_cid, "L").await;

    let body = op(
        Uuid::new_v4(),
        2,
        "track",
        file_path,
        None,
        "insert",
        track_insert_payload(
            library_cid,
            "Around the World",
            "blake3-dddddddd",
            Some("Discovery"),
            Some("Daft Punk"),
            false,
            &["Daft Punk"],
        ),
        Some(PROFILE_CID),
    );

    assert!(push(&auth.base, &auth.token, std::slice::from_ref(&body))
        .await
        .is_success());
    assert!(push(&auth.base, &auth.token, std::slice::from_ref(&body))
        .await
        .is_success());

    // Single track row + single album row + single artist row +
    // single track_artist link, despite the re-emit.
    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM track WHERE file_path = $1),
            (SELECT COUNT(*) FROM album WHERE canonical_title = 'Discovery'),
            (SELECT COUNT(*) FROM artist WHERE name = 'Daft Punk'),
            (SELECT COUNT(*) FROM track_artist
              WHERE track_id = (SELECT id FROM track WHERE file_path = $1))",
    )
    .bind(file_path)
    .fetch_one(&pool)
    .await
    .expect("post-replay counts");
    assert_eq!(counts, (1, 1, 1, 1), "replay must not duplicate any row");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_without_library_canonical_is_rejected(pool: PgPool) {
    // Missing `library_canonical_id` is a structural payload bug,
    // not an ordering hiccup. The push handler MUST reject the
    // batch (rolls back the durable insert) — silently dropping
    // it would leave the desktop thinking its op landed.
    let auth = spawn_authenticated(pool.clone(), "apply-track-no-lib").await;
    let file_path = "/music/naked.mp3";

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "track",
            file_path,
            None,
            "insert",
            json!({
                "title": "Naked",
                "file_hash": "blake3-eeeeeeee",
                "file_size": 100,
                "duration_ms": 200,
            }),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(
        !status.is_success(),
        "missing library_canonical_id MUST fail the push (got {status})"
    );

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM track WHERE file_path = $1")
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0, "no track row may leak from a rejected push");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_with_unknown_library_skips_but_logs(pool: PgPool) {
    // The library hasn't been materialised yet — the apply path
    // surfaces Skipped (not InvalidPayload) so the op stays in
    // the durable log and replays once the library's own insert
    // lands. Push still succeeds (the durable log is the contract).
    let auth = spawn_authenticated(pool.clone(), "apply-track-skip").await;
    let file_path = "/music/t.mp3";
    let op_id = Uuid::new_v4();

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            op_id,
            1,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                "lib-not-yet-synced",
                "T",
                "blake3-ffffffff",
                None,
                None,
                false,
                &[],
            ),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(status.is_success(), "push must succeed even on Skipped");

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM track WHERE file_path = $1")
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count.0, 0,
        "Skipped op MUST NOT materialise a track row — replay after library insert lands does"
    );

    // Durability: the op MUST land in `sync_op` so a later replay
    // (after the library's own insert) materialises the track.
    // Without this assertion, a refactor that drops the op on the
    // floor would silently break the Skipped-then-replay contract.
    let logged: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_op
          WHERE user_id = $1 AND operation_id = $2
            AND entity = 'track' AND entity_id = $3",
    )
    .bind(auth.user_id)
    .bind(op_id)
    .bind(file_path)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        logged, 1,
        "Skipped track op MUST be persisted in sync_op for replay"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_dedups_duplicate_artists(pool: PgPool) {
    // A desktop that ships a duplicated artist (e.g., a tag with
    // the same name twice after the `";"` split) must NOT trip
    // the `(track_id, artist_id)` PK on the second
    // `replace_track_artists` insert. `artists_from_payload`
    // dedupes first-seen-order; the test pins that contract.
    let auth = spawn_authenticated(pool.clone(), "apply-track-dup").await;
    let library_cid = "lib-dup";
    let file_path = "/music/dup.mp3";

    materialise_library(&auth.base, &auth.token, library_cid, "L").await;

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Twice",
                "hash-dup",
                None,
                None,
                false,
                &["Daft Punk", "Daft Punk", "Pharrell Williams", "Daft Punk"],
            ),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(
        status.is_success(),
        "duplicate-artist payload MUST NOT trip the PK (got {status})"
    );

    // First-seen wins → "Daft Punk" at position 0, "Pharrell
    // Williams" at position 1. Subsequent "Daft Punk" entries
    // dropped silently.
    let rows: Vec<(String, i32)> = sqlx::query_as(
        "SELECT a.name, ta.position
           FROM track_artist ta
           JOIN artist a ON a.id = ta.artist_id
          WHERE ta.track_id = (SELECT id FROM track WHERE file_path = $1)
          ORDER BY ta.position ASC",
    )
    .bind(file_path)
    .fetch_all(&pool)
    .await
    .expect("track_artist select");
    assert_eq!(
        rows,
        vec![
            ("Daft Punk".to_owned(), 0),
            ("Pharrell Williams".to_owned(), 1),
        ],
        "dedupe MUST preserve first-seen order"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_then_replay_after_library_lands_materialises(pool: PgPool) {
    // The Skipped semantic only works if a later replay (after
    // the library lands) actually materialises the row. This test
    // proves that — push the track first, then the library, then
    // re-push the track via a new lamport_ts (the desktop's
    // reconnect loop would do this).
    let auth = spawn_authenticated(pool.clone(), "apply-track-replay-skip").await;
    let library_cid = "lib-late";
    let file_path = "/music/late.mp3";

    // Track first, before library exists → Skipped.
    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Late",
                "blake3-gggggggg",
                None,
                None,
                false,
                &[]
            ),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    // Library lands.
    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "library",
            library_cid,
            None,
            "insert",
            json!({ "name": "L" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    // Re-push the same track op (the desktop's catch-up pull will
    // hand it back through apply on the next reconnect).
    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            3,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Late",
                "blake3-gggggggg",
                None,
                None,
                false,
                &[]
            ),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM track WHERE file_path = $1")
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 1, "track row must materialise on the replay");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_delete_removes_row_and_cascades_track_artist(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "apply-track-del").await;
    let library_cid = "lib-del";
    let file_path = "/music/doomed.mp3";

    materialise_library(&auth.base, &auth.token, library_cid, "L").await;

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Doomed",
                "blake3-hhhhhhhh",
                Some("Doomed Album"),
                Some("Doomed Artist"),
                false,
                &["Doomed Artist"],
            ),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let track_id: i64 = sqlx::query_scalar("SELECT id FROM track WHERE file_path = $1")
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .expect("track id");

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            3,
            "track",
            file_path,
            None,
            "delete",
            json!({ "library_canonical_id": library_cid }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    // Track gone, track_artist cascaded, album survives as orphan
    // (the per-library album entity has independent meaning).
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM track WHERE id = $1),
            (SELECT COUNT(*) FROM track_artist WHERE track_id = $1),
            (SELECT COUNT(*) FROM album WHERE canonical_title = 'Doomed Album')",
    )
    .bind(track_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts.0, 0, "track row must be gone");
    assert_eq!(counts.1, 0, "track_artist links must cascade away");
    assert_eq!(counts.2, 1, "album row must survive (entity, not join)");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_re_emit_after_tag_edit_updates_in_place(pool: PgPool) {
    // The desktop's tag-editor flow: user edits the title, lofty
    // rewrites the audio file's metadata frames, BLAKE3 hash
    // changes, file_path stays the same. The re-emit op carries
    // the SAME entity_id (file_path) but a NEW `file_hash` +
    // `title` in payload.
    //
    // This is the exact scenario H1 from the CR was about — using
    // file_hash as the ON CONFLICT key would fall through to
    // INSERT and trip `UNIQUE (library_id, file_path)`. With the
    // fix (ON CONFLICT on file_path), the second push lands as an
    // UPDATE in place.
    let auth = spawn_authenticated(pool.clone(), "apply-track-reemit").await;
    let library_cid = "lib-reemit";
    let file_path = "/music/song.mp3";

    materialise_library(&auth.base, &auth.token, library_cid, "L").await;

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Old Title",
                "hash-before",
                None,
                None,
                false,
                &[]
            ),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    // Re-emit with new title + new file_hash, same file_path.
    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            3,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "New Title",
                "hash-after",
                None,
                None,
                false,
                &[]
            ),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let row: (String, String, Option<String>) =
        sqlx::query_as("SELECT title, file_path, file_hash FROM track WHERE file_path = $1")
            .bind(file_path)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.0, "New Title");
    assert_eq!(
        row.1, file_path,
        "file_path must NOT change on tag-edit re-emit"
    );
    assert_eq!(
        row.2.as_deref(),
        Some("hash-after"),
        "file_hash from the latest payload must overwrite the prior value"
    );

    // Still only one row at this file_path.
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM track WHERE file_path = $1")
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 1, "tag-edit re-emit must NOT create a second row");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_rejects_empty_album_title(pool: PgPool) {
    // Empty-string `album_title` is a structural payload bug. The
    // apply layer rejects it as InvalidPayload (push 400/500
    // family) rather than letting it hit the `length(...) > 0`
    // CHECK on `album.canonical_title` — that would surface as a
    // 500 (DB error), and the desktop would retry the broken op
    // forever.
    let auth = spawn_authenticated(pool.clone(), "apply-track-empty-album").await;
    let library_cid = "lib-empty-album";
    let file_path = "/music/song.mp3";

    materialise_library(&auth.base, &auth.token, library_cid, "L").await;

    let status = push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "track",
            file_path,
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Track",
                "hash-1",
                Some(""), // empty album_title → reject
                None,
                false,
                &[],
            ),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(
        !status.is_success(),
        "empty album_title MUST fail the push (got {status})"
    );

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM track WHERE file_path = $1")
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0, "no track row may leak from a rejected push");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_set_op_is_unknown(pool: PgPool) {
    // `set` ops on the `track` entity are intentionally Unknown
    // — the desktop's tag-editor save path re-emits a full
    // INSERT (the upsert handles the merge). Pin the contract so
    // a future commit that silently routes `set` through
    // `insert` is caught by CI.
    let auth = spawn_authenticated(pool.clone(), "apply-track-set").await;
    let library_cid = "lib-set";
    let file_path = "/music/song.mp3";

    materialise_library(&auth.base, &auth.token, library_cid, "L").await;

    assert!(
        push(
            &auth.base,
            &auth.token,
            &[op(
                Uuid::new_v4(),
                2,
                "track",
                file_path,
                Some("title"),
                "set",
                json!({ "library_canonical_id": library_cid, "value": "New Title" }),
                Some(PROFILE_CID),
            )],
        )
        .await
        .is_success(),
        "Unknown ops still durably log; push must succeed"
    );

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM track WHERE file_path = $1")
        .bind(file_path)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count.0, 0,
        "set ops MUST NOT materialise a track row (no insert side-effect)"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_insert_cross_tenant_library_canonical_skips(pool: PgPool) {
    // Bob pushes a track op naming a library_canonical_id that
    // belongs to Alice's profile. The library lookup is scoped
    // to (profile_id, canonical_id) — Bob's profile resolution
    // produces a DIFFERENT profile_id than Alice's, so the
    // lookup misses and the op surfaces as Skipped. Bob's row
    // count stays 0; Alice's library is untouched.
    //
    // Future careless refactor that drops the per-profile scope
    // from `lookup_library_id` would silently let Bob's track
    // ops materialise into Alice's library — this test catches
    // that regression.
    let two =
        spawn_two_authenticated(pool.clone(), "apply-track-tenant-a", "apply-track-tenant-b").await;
    let library_cid = "lib-alice";

    // Alice materialises her library.
    let alice_lib_status = push(
        &two.base,
        &two.a.token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            library_cid,
            None,
            "insert",
            json!({ "name": "Alice Library" }),
            Some(PROFILE_CID),
        )],
    )
    .await;
    assert!(alice_lib_status.is_success());

    // Bob pushes a track op naming Alice's library_canonical_id.
    // The profile auto-provisioning gives Bob a fresh profile
    // (different from Alice's), so the per-profile library
    // lookup misses → Skipped.
    let bob_track_status = push(
        &two.base,
        &two.b.token,
        &[op(
            Uuid::new_v4(),
            1,
            "track",
            "/music/cross.mp3",
            None,
            "insert",
            track_insert_payload(
                library_cid,
                "Stolen Track",
                "hash-stolen",
                None,
                None,
                false,
                &[],
            ),
            Some(PROFILE_CID_B),
        )],
    )
    .await;
    assert!(bob_track_status.is_success(), "Skipped path still 200s");

    // Bob's profile has no track rows (the op was Skipped).
    let bob_track_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM track t
           JOIN library l ON l.id = t.library_id
           JOIN profile p ON p.id = l.profile_id
          WHERE p.user_id = $1",
    )
    .bind(two.b.user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        bob_track_count.0, 0,
        "Bob MUST NOT materialise a track row via Alice's library_canonical_id"
    );

    // Alice's library is untouched (no track rows leaked into it).
    let alice_track_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM track t
           JOIN library l ON l.id = t.library_id
           JOIN profile p ON p.id = l.profile_id
          WHERE p.user_id = $1",
    )
    .bind(two.a.user_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alice_track_count.0, 0, "Alice's library must remain empty");
}
