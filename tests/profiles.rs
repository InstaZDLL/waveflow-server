//! End-to-end tests for `/api/v1/profiles`.
//!
//! Every test boots the real router (no axum-test mocks) against a
//! per-test Postgres database from `#[sqlx::test]`, calls
//! `spawn_authenticated` to provision a user via the lazy-provision
//! JWT path, then exercises the CRUD surface with the resulting
//! Bearer token. Phase 1.d.2 retired the `X-User-Id` shim + the
//! bootstrap `POST /api/v1/users` endpoint, so JWT is the only path
//! these tests exercise.

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_app_with_jwt, spawn_authenticated, spawn_two_authenticated, JwksHarness};

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn missing_bearer_returns_401(pool: PgPool) {
    let harness = JwksHarness::spawn().await;
    let base = spawn_app_with_jwt(pool, harness.verifier_arc()).await;

    // No Authorization — every CRUD verb bounces 401.
    for (method, path) in [
        ("GET", "/api/v1/profiles"),
        ("POST", "/api/v1/profiles"),
        ("GET", "/api/v1/profiles/1"),
        ("PATCH", "/api/v1/profiles/1"),
        ("DELETE", "/api/v1/profiles/1"),
    ] {
        let req = match method {
            "GET" => reqwest::Client::new().get(format!("{base}{path}")),
            "POST" => reqwest::Client::new()
                .post(format!("{base}{path}"))
                .json(&json!({ "name": "x", "color_id": "y" })),
            "PATCH" => reqwest::Client::new()
                .patch(format!("{base}{path}"))
                .json(&json!({ "name": "x" })),
            "DELETE" => reqwest::Client::new().delete(format!("{base}{path}")),
            _ => unreachable!(),
        };
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {path}");
    }
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_then_list_then_get(pool: PgPool) {
    let auth = spawn_authenticated(pool, "profiles-create-list-get").await;

    // Empty to start.
    let body: Value = reqwest::Client::new()
        .get(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(body.as_array().unwrap().is_empty());

    // Create.
    let created: Value = reqwest::Client::new()
        .post(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({ "name": "Alice", "color_id": "emerald" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["name"], "Alice");
    assert_eq!(created["color_id"], "emerald");
    assert!(
        created.get("data_dir").is_none(),
        "data_dir leaked into response"
    );

    // List now sees it.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"].as_i64().unwrap(), id);

    // Get by id round-trips the same shape.
    let one: Value = reqwest::Client::new()
        .get(format!("{}/api/v1/profiles/{id}", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["id"].as_i64().unwrap(), id);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn tenants_are_isolated(pool: PgPool) {
    let two = spawn_two_authenticated(pool, "profiles-tenant-a", "profiles-tenant-b").await;
    let base = two.base.clone();
    let a = &two.a;
    let b = &two.b;

    // User A creates a profile (on A's app instance).
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .bearer_auth(&a.token)
        .json(&json!({ "name": "A's profile", "color_id": "emerald" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();

    // User B's list is empty (B sees the same DB but the WHERE
    // user_id = $1 filter excludes A's row).
    let list_b: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_b.is_empty(), "user B saw user A's profiles");

    // User B can't fetch A's profile by id either — 404, not 200
    // (no data leak), not 403 (no existence leak).
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{id}"))
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // And user B can't delete A's profile.
    let resp = reqwest::Client::new()
        .delete(format!("{base}/api/v1/profiles/{id}"))
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User A still sees their profile.
    let list_a: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .bearer_auth(&a.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list_a.len(), 1);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_renames_in_place(pool: PgPool) {
    let auth = spawn_authenticated(pool, "profiles-rename").await;

    let id = reqwest::Client::new()
        .post(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({ "name": "Old name", "color_id": "emerald" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    let renamed: Value = reqwest::Client::new()
        .patch(format!("{}/api/v1/profiles/{id}", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({ "name": "New name" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(renamed["name"], "New name");
    assert_eq!(renamed["id"].as_i64().unwrap(), id);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn delete_blocks_last_profile(pool: PgPool) {
    let auth = spawn_authenticated(pool, "profiles-delete-last").await;

    let id = reqwest::Client::new()
        .post(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .json(&json!({ "name": "only one", "color_id": "emerald" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    // Deleting the last profile must 409 — the storage invariant
    // refuses to leave the user with zero profiles.
    let resp = reqwest::Client::new()
        .delete(format!("{}/api/v1/profiles/{id}", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Profile still there.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn delete_succeeds_when_more_than_one(pool: PgPool) {
    let auth = spawn_authenticated(pool, "profiles-delete-more").await;

    // Two profiles → deleting one leaves one → 204.
    let mut ids = Vec::new();
    for name in ["one", "two"] {
        let id = reqwest::Client::new()
            .post(format!("{}/api/v1/profiles", auth.base))
            .bearer_auth(&auth.token)
            .json(&json!({ "name": name, "color_id": "emerald" }))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<Value>()
            .await
            .unwrap()["id"]
            .as_i64()
            .unwrap();
        ids.push(id);
    }

    let resp = reqwest::Client::new()
        .delete(format!("{}/api/v1/profiles/{}", auth.base, ids[0]))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{}/api/v1/profiles", auth.base))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"].as_i64().unwrap(), ids[1]);
}
