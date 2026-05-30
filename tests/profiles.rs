//! End-to-end tests for `/api/v1/users` + `/api/v1/profiles`.
//!
//! Every test boots the real router (no axum-test mocks) against a
//! per-test Postgres database from `#[sqlx::test]`, mints a user via
//! `POST /api/v1/users`, then exercises the CRUD surface with that
//! user id in the `X-User-Id` header. The shared `mint_user` helper
//! does the bootstrap and returns the id so each test focuses on
//! its scenario.

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::spawn_app;

/// Bootstrap: mint a user, return its id. Every test under
/// `/api/v1/profiles` needs one because the FK on `profile.user_id`
/// rejects orphaned writes.
async fn mint_user(base: &str) -> i64 {
    let body: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/users"))
        .send()
        .await
        .expect("user create failed")
        .error_for_status()
        .expect("non-2xx on user create")
        .json()
        .await
        .expect("user create body");
    body["id"].as_i64().expect("user id missing from response")
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_user_returns_201_with_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/users"))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = resp.json().await.unwrap();
    assert!(body["id"].as_i64().unwrap() > 0);
}

/// Phase 1.d.1 seed: `external_id` accepted, persisted, returned via
/// `id`. The actual round-trip back through a query lives in the
/// JWT middleware tests (1.d.1-PR2) — here we just exercise the
/// handler accepting the payload without 500'ing.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_user_accepts_external_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/users"))
        .json(&json!({ "external_id": "auth-provider-uuid-abc-123" }))
        .send()
        .await
        .expect("request failed");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = resp.json().await.unwrap();
    assert!(body["id"].as_i64().unwrap() > 0);
}

/// Blank `external_id` (empty or whitespace-only after trim) must
/// 400 — otherwise it would slip past the UNIQUE constraint and sit
/// in the DB as a non-NULL-but-blank row that no JWT could ever
/// match. Same boundary-validation rule as the rest of 1.b.5.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_user_rejects_blank_external_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    for blank in ["", "   ", "\t\n "] {
        let resp = reqwest::Client::new()
            .post(format!("{base}/api/v1/users"))
            .json(&json!({ "external_id": blank }))
            .send()
            .await
            .expect("request failed");
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "external_id = {blank:?} should 400"
        );
    }
}

/// Two POSTs with the same `external_id` must collide on the UNIQUE
/// constraint — the second one gets 409, not a transient 500.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_user_rejects_duplicate_external_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    let body = json!({ "external_id": "duplicate-sub" });
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/users"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/users"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

/// An explicit `null` for `external_id` is equivalent to omitting it
/// — same behaviour as the no-body case. Locks in the contract so a
/// future serde change doesn't silently flip "explicit null" into a
/// validation error.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_user_accepts_explicit_null_external_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/users"))
        .json(&json!({ "external_id": null }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn profiles_require_x_user_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    // No header — every CRUD verb should bounce 401.
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
async fn malformed_x_user_id_rejected(pool: PgPool) {
    let base = spawn_app(pool).await;

    for header in ["", "abc", "0", "-1"] {
        let resp = reqwest::Client::new()
            .get(format!("{base}/api/v1/profiles"))
            .header("x-user-id", header)
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "X-User-Id = {header:?} should 401"
        );
    }
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_then_list_then_get(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;

    // Empty to start.
    let body: Value = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(body.as_array().unwrap().is_empty());

    // Create.
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
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
        .get(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
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
        .get(format!("{base}/api/v1/profiles/{id}"))
        .header("x-user-id", user_id.to_string())
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
    let base = spawn_app(pool).await;
    let user_a = mint_user(&base).await;
    let user_b = mint_user(&base).await;

    // User A creates a profile.
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_a.to_string())
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

    // User B's list is empty.
    let list_b: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_b.to_string())
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
        .header("x-user-id", user_b.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // And user B can't delete A's profile.
    let resp = reqwest::Client::new()
        .delete(format!("{base}/api/v1/profiles/{id}"))
        .header("x-user-id", user_b.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User A still sees their profile.
    let list_a: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_a.to_string())
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
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
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
        .patch(format!("{base}/api/v1/profiles/{id}"))
        .header("x-user-id", user_id.to_string())
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
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
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
        .delete(format!("{base}/api/v1/profiles/{id}"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // Profile still there.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
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
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;

    // Two profiles → deleting one leaves one → 204.
    let mut ids = Vec::new();
    for name in ["one", "two"] {
        let id = reqwest::Client::new()
            .post(format!("{base}/api/v1/profiles"))
            .header("x-user-id", user_id.to_string())
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
        .delete(format!("{base}/api/v1/profiles/{}", ids[0]))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"].as_i64().unwrap(), ids[1]);
}

/// With `WAVEFLOW_DEV_AUTH` unset (production default), every
/// `/api/v1/*` request must short-circuit to 503 — even a "valid"
/// X-User-Id header. The probe routes (`/health`, `/ready`,
/// `/openapi.json`, `/reference`) stay reachable because they don't
/// carry tenant data.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn dev_auth_gate_returns_503_when_disabled(pool: PgPool) {
    let base = support::spawn_app_prod_gate(pool).await;

    // Health stays up.
    let resp = reqwest::Client::new()
        .get(format!("{base}/health"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // POST /api/v1/users — gated.
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/users"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

    // GET /api/v1/profiles with a header — still gated; the 503
    // wins over the auth shim so an attacker can't tell the shim
    // exists.
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles"))
        .header("x-user-id", "42")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_with_unknown_user_id_returns_409(pool: PgPool) {
    let base = spawn_app(pool).await;

    // Skip mint_user — use a hard-coded id no users row will have.
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .header("x-user-id", "99999")
        .json(&json!({ "name": "x", "color_id": "emerald" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
