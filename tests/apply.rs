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
