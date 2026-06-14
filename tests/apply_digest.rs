//! RFC-003 Phase A.2.3 — payload_hash binding + metadata_digest_version
//! bump + /api/v1/sync/digest endpoint. The digest snapshot is how
//! two replicas detect that their materialised state has diverged
//! after a sync. Tests focus on three contracts: (1) every write
//! stamps a payload_hash, (2) digest_version monotonically bumps on
//! every write, (3) the endpoint emits a stable set_hash for a
//! stable row set.

mod support;

use serde_json::{json, Value};
use sqlx::PgPool;
use support::spawn_authenticated;
use uuid::Uuid;

const PROFILE_CID: &str = "prof-aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";

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
async fn library_insert_binds_payload_hash_and_bumps_digest(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "digest-lib-ins").await;

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            "lib-1",
            None,
            "insert",
            json!({ "name": "Bandes-son" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let row: (Option<Vec<u8>>,) =
        sqlx::query_as("SELECT payload_hash FROM library WHERE canonical_id = $1")
            .bind("lib-1")
            .fetch_one(&pool)
            .await
            .unwrap();
    let hash = row.0.expect("library payload_hash must be populated");
    assert_eq!(hash.len(), 32, "BLAKE3 hash must be 32 bytes");

    let library_version: (i64,) = sqlx::query_as(
        "SELECT version FROM metadata_digest_version
          WHERE entity = $1 AND profile_id IN (SELECT id FROM profile WHERE canonical_id = $2)",
    )
    .bind("library")
    .bind(PROFILE_CID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        library_version.0, 1,
        "library digest must bump to 1 on first insert"
    );

    let profile_version: (i64,) = sqlx::query_as(
        "SELECT version FROM metadata_digest_version
          WHERE entity = $1 AND profile_id IN (SELECT id FROM profile WHERE canonical_id = $2)",
    )
    .bind("profile")
    .bind(PROFILE_CID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        profile_version.0, 1,
        "profile digest must bump on auto-provision"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn library_set_field_recomputes_hash_and_bumps_digest(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "digest-lib-set").await;

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            "lib-1",
            None,
            "insert",
            json!({ "name": "Bandes-son" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let hash_after_insert: Vec<u8> =
        sqlx::query_scalar("SELECT payload_hash FROM library WHERE canonical_id = $1")
            .bind("lib-1")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            2,
            "library",
            "lib-1",
            Some("name"),
            "set",
            json!({ "value": "Live Albums" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let hash_after_set: Vec<u8> =
        sqlx::query_scalar("SELECT payload_hash FROM library WHERE canonical_id = $1")
            .bind("lib-1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_ne!(
        hash_after_insert, hash_after_set,
        "set_field must change payload_hash"
    );

    let library_version: (i64,) = sqlx::query_as(
        "SELECT version FROM metadata_digest_version
          WHERE entity = $1 AND profile_id IN (SELECT id FROM profile WHERE canonical_id = $2)",
    )
    .bind("library")
    .bind(PROFILE_CID)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        library_version.0 >= 2,
        "library digest must bump on set_field (was {})",
        library_version.0
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn digest_endpoint_returns_stable_set_hash_for_library(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "digest-endpoint").await;

    assert!(push(
        &auth.base,
        &auth.token,
        &[
            op(
                Uuid::new_v4(),
                1,
                "library",
                "lib-a",
                None,
                "insert",
                json!({ "name": "A" }),
                Some(PROFILE_CID),
            ),
            op(
                Uuid::new_v4(),
                2,
                "library",
                "lib-b",
                None,
                "insert",
                json!({ "name": "B" }),
                Some(PROFILE_CID),
            ),
        ],
    )
    .await
    .is_success());

    let digest: Value = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/sync/digest?entity=library&profile_canonical_id={}",
            auth.base, PROFILE_CID
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(digest["members"].as_array().unwrap().len(), 2);
    assert_eq!(digest["members"][0]["canonical_id"], "lib-a");
    assert_eq!(digest["members"][1]["canonical_id"], "lib-b");
    assert_eq!(
        digest["set_hash"].as_str().unwrap().len(),
        64,
        "set_hash must be 32-byte BLAKE3 hex (64 chars)"
    );
    assert_eq!(digest["version"], 2, "two inserts ⇒ version 2");
    assert!(digest["max_hlc"].is_object(), "max_hlc must be present");

    let digest2: Value = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/sync/digest?entity=library&profile_canonical_id={}",
            auth.base, PROFILE_CID
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(digest["set_hash"], digest2["set_hash"]);
    assert_eq!(digest["version"], digest2["version"]);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn digest_endpoint_rejects_profile_id_on_user_scoped_entity(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "digest-misuse").await;

    let res = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/sync/digest?entity=liked_track&profile_canonical_id={}",
            auth.base, PROFILE_CID
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn digest_endpoint_user_scoped_liked_round_trip(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "digest-liked").await;

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "liked_track",
            "blake3-hash-zzz",
            None,
            "insert",
            json!({}),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let digest: Value = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/sync/digest?entity=liked_track",
            auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(digest["members"].as_array().unwrap().len(), 1);
    assert_eq!(digest["members"][0]["canonical_id"], "blake3-hash-zzz");
    assert_eq!(digest["version"], 1);
}
