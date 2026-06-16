//! RFC-003 Phase B.2 — `GET /api/v1/sync/entity` round-trip
//! coverage.
//!
//! The desktop's backfill orchestrator fetches a row's full
//! canonical state by `(entity, canonical_id)` whenever a digest
//! mismatch surfaces. These tests focus on three contracts:
//!
//! - Round-trip: pushing an op stamps the row, fetching it
//!   returns the canonical fields + the same `payload_hash`
//!   (hex) the digest endpoint would emit.
//! - Scope discipline matches the digest endpoint: profile-
//!   scoped entities (`library` / `playlist` / `track`) require
//!   `profile_canonical_id`; user-scoped (`liked_track` /
//!   `track_rating`) reject it.
//! - Per-entity edge cases: track composite canonical splits on
//!   the U+001F separator, cross-tenant isolation never leaks a
//!   row, unstamped rows are invisible (treated as 404).

mod support;

use serde_json::{json, Value};
use sqlx::PgPool;
use support::spawn_authenticated;
use uuid::Uuid;

const PROFILE_CID: &str = "prof-bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";

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
        .json(&json!({ "device_id": "device-b2", "ops": ops }))
        .send()
        .await
        .unwrap()
        .status()
}

async fn fetch(
    base: &str,
    token: &str,
    params: &[(&str, &str)],
) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{base}/api/v1/sync/entity"))
        .bearer_auth(token)
        .query(params)
        .send()
        .await
        .unwrap()
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn library_round_trips_with_canonical_fields_and_hash(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-lib").await;

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            "lib-canon-1",
            None,
            "insert",
            json!({ "name": "Bandes-son", "description": "best of", "color_id": "azure", "icon_id": "library" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let res = fetch(
        &auth.base,
        &auth.token,
        &[
            ("entity", "library"),
            ("canonical_id", "lib-canon-1"),
            ("profile_canonical_id", PROFILE_CID),
        ],
    )
    .await;
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let body: Value = res.json().await.unwrap();
    assert_eq!(body["entity"], "library");
    assert_eq!(body["canonical_id"], "lib-canon-1");
    assert_eq!(body["fields"]["name"], "Bandes-son");
    assert_eq!(body["fields"]["description"], "best of");
    assert_eq!(body["fields"]["color_id"], "azure");
    assert_eq!(body["fields"]["icon_id"], "library");
    assert_eq!(
        body["payload_hash"].as_str().unwrap().len(),
        64,
        "payload_hash must be 32-byte BLAKE3 hex (64 chars)",
    );
    assert!(body["hlc"]["wall"].as_i64().is_some());

    // Library / playlist responses never carry the track-only
    // auxiliary fields.
    assert!(body.get("library_canonical_id").is_none_or(|v| v.is_null()));
    assert!(body.get("file_path").is_none_or(|v| v.is_null()));
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn playlist_round_trips_with_canonical_fields(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-pl").await;

    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "playlist",
            "pl-canon-1",
            None,
            "insert",
            json!({ "name": "Mix", "color_id": "violet" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let body: Value = fetch(
        &auth.base,
        &auth.token,
        &[
            ("entity", "playlist"),
            ("canonical_id", "pl-canon-1"),
            ("profile_canonical_id", PROFILE_CID),
        ],
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(body["fields"]["name"], "Mix");
    assert_eq!(body["fields"]["color_id"], "violet");
    // Omitted-on-insert description was defaulted to NULL by the
    // apply pipeline; the canonical form keeps it present as
    // `null` (not absent) so the desktop's hash matches.
    assert_eq!(body["fields"]["description"], Value::Null);
    // icon_id defaulted to "music" by apply.
    assert_eq!(body["fields"]["icon_id"], "music");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_uses_composite_canonical_and_echoes_split(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-track").await;

    // 1. Library first so the track can attach.
    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            "lib-track-1",
            None,
            "insert",
            json!({ "name": "Records" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    // 2. Track. file_path is the entity_id; library_canonical_id
    // rides on the payload.
    let file_path = "/music/Daft Punk/Get Lucky.flac";
    let track_payload = json!({
        "library_canonical_id": "lib-track-1",
        "title": "Get Lucky",
        "file_hash": "blake3-track-abc",
        "file_size": 12_345_678i64,
        "duration_ms": 369_000i64,
        "added_at": 1_700_000_000_000_i64,
        "year": 2013i64,
        "is_compilation": false,
        "artists": ["Daft Punk", "Pharrell Williams"],
        "album_title": "Random Access Memories",
        "album_artist_name": "Daft Punk",
    });
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
            track_payload,
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    // 3. Fetch via the composite canonical. reqwest's `.query()`
    // URL-encodes the U+001F separator + the file path as a
    // single value.
    let composite = format!("lib-track-1\u{001F}{file_path}");
    let body: Value = fetch(
        &auth.base,
        &auth.token,
        &[
            ("entity", "track"),
            ("canonical_id", &composite),
            ("profile_canonical_id", PROFILE_CID),
        ],
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();

    assert_eq!(body["entity"], "track");
    assert_eq!(body["canonical_id"], composite);
    assert_eq!(body["library_canonical_id"], "lib-track-1");
    assert_eq!(body["file_path"], file_path);
    assert_eq!(body["fields"]["title"], "Get Lucky");
    assert_eq!(body["fields"]["file_hash"], "blake3-track-abc");
    assert_eq!(body["fields"]["album_title"], "Random Access Memories");
    assert_eq!(body["fields"]["album_artist_name"], "Daft Punk");
    let artists = body["fields"]["artists"].as_array().unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0], "Daft Punk");
    assert_eq!(artists[1], "Pharrell Williams");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_missing_composite_separator_returns_400(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-track-bad").await;
    let res = fetch(
        &auth.base,
        &auth.token,
        &[
            ("entity", "track"),
            ("canonical_id", "no-separator"),
            ("profile_canonical_id", PROFILE_CID),
        ],
    )
    .await;
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn liked_track_round_trips_with_empty_fields_map(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-liked").await;
    let file_hash = "blake3-liked-zzz";
    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "liked_track",
            file_hash,
            None,
            "insert",
            json!({}),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let body: Value = fetch(
        &auth.base,
        &auth.token,
        &[("entity", "liked_track"), ("canonical_id", file_hash)],
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(body["entity"], "liked_track");
    assert_eq!(body["canonical_id"], file_hash);
    assert_eq!(body["fields"], json!({}));
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_rating_round_trips_with_single_key(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-rating").await;
    let file_hash = "blake3-rating-yyy";
    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "track_rating",
            file_hash,
            None,
            "set",
            json!({ "value": 200 }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let body: Value = fetch(
        &auth.base,
        &auth.token,
        &[("entity", "track_rating"), ("canonical_id", file_hash)],
    )
    .await
    .error_for_status()
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(body["fields"]["rating"], 200);
    // User-scoped: no profile_canonical_id required on the URL,
    // none echoed in the response either.
    assert!(body.get("library_canonical_id").is_none_or(|v| v.is_null()));
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn profile_scoped_entity_requires_profile_canonical_id(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-no-canon").await;
    let res = fetch(
        &auth.base,
        &auth.token,
        &[("entity", "library"), ("canonical_id", "whatever")],
    )
    .await;
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn user_scoped_entity_rejects_profile_canonical_id(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-bad-canon").await;
    let res = fetch(
        &auth.base,
        &auth.token,
        &[
            ("entity", "liked_track"),
            ("canonical_id", "blake3-x"),
            ("profile_canonical_id", PROFILE_CID),
        ],
    )
    .await;
    assert_eq!(res.status(), reqwest::StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn unknown_canonical_id_returns_404(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "entity-404").await;
    // Push something for THIS user so the profile auto-provisions
    // (otherwise the 404 fires on the profile resolve step, not
    // the row lookup we want to exercise).
    assert!(push(
        &auth.base,
        &auth.token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            "lib-real",
            None,
            "insert",
            json!({ "name": "real" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let res = fetch(
        &auth.base,
        &auth.token,
        &[
            ("entity", "library"),
            ("canonical_id", "lib-doesnt-exist"),
            ("profile_canonical_id", PROFILE_CID),
        ],
    )
    .await;
    assert_eq!(res.status(), reqwest::StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn cross_tenant_profile_canonical_returns_404(pool: PgPool) {
    // User A pushes a library; user B tries to fetch it via A's
    // profile_canonical_id. The profile_id resolve step must
    // refuse to expose the row (per the digest endpoint's same
    // contract).
    let user_a = spawn_authenticated(pool.clone(), "entity-tenant-a").await;
    let user_b = spawn_authenticated(pool.clone(), "entity-tenant-b").await;

    assert!(push(
        &user_a.base,
        &user_a.token,
        &[op(
            Uuid::new_v4(),
            1,
            "library",
            "lib-a-1",
            None,
            "insert",
            json!({ "name": "private" }),
            Some(PROFILE_CID),
        )],
    )
    .await
    .is_success());

    let res = fetch(
        &user_b.base,
        &user_b.token,
        &[
            ("entity", "library"),
            ("canonical_id", "lib-a-1"),
            ("profile_canonical_id", PROFILE_CID),
        ],
    )
    .await;
    assert_eq!(res.status(), reqwest::StatusCode::NOT_FOUND);
}
