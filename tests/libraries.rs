//! End-to-end tests for `/api/v1/profiles/{profile_id}/libraries`.
//!
//! Same harness pattern as `tests/profiles.rs`: each test boots the
//! real router against a per-test Postgres provisioned by
//! `#[sqlx::test]`, mints a user + a profile, then exercises the CRUD
//! surface. The whole point of the suite is the tenant isolation
//! battery — every scenario where a foreign user / foreign profile
//! must NOT see / touch / delete a library has its own case.

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::spawn_app;

/// Mint a user via `POST /api/v1/users`. Same helper signature as
/// `tests/profiles.rs::mint_user` — duplicated here on purpose to keep
/// each integration test file self-contained (Cargo compiles each
/// `tests/*.rs` as its own crate, so factoring this out would mean a
/// `support` helper that not every file needs).
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

/// Mint a profile under `user_id` and return its id. Every library
/// test needs at least one profile because `library.profile_id` is a
/// non-null FK.
async fn mint_profile(base: &str, user_id: i64, name: &str) -> i64 {
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": name, "color_id": "emerald" }))
        .send()
        .await
        .expect("profile create failed")
        .error_for_status()
        .expect("non-2xx on profile create")
        .json()
        .await
        .expect("profile create body");
    created["id"].as_i64().expect("profile id missing")
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn libraries_require_x_user_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    // No header — every CRUD verb on the nested resource should bounce 401.
    for (method, path) in [
        ("GET", "/api/v1/profiles/1/libraries"),
        ("POST", "/api/v1/profiles/1/libraries"),
        ("GET", "/api/v1/profiles/1/libraries/1"),
        ("PATCH", "/api/v1/profiles/1/libraries/1"),
        ("DELETE", "/api/v1/profiles/1/libraries/1"),
    ] {
        let req = match method {
            "GET" => reqwest::Client::new().get(format!("{base}{path}")),
            "POST" => reqwest::Client::new()
                .post(format!("{base}{path}"))
                .json(&json!({ "name": "x" })),
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
async fn create_then_list_then_get_under_profile(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    // Empty to start.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty());

    // Create — minimal body (color/icon fall back to defaults).
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": "Bandes-son" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["name"], "Bandes-son");
    assert_eq!(created["color_id"], "emerald", "default color_id");
    assert_eq!(created["icon_id"], "library", "default icon_id");
    assert_eq!(
        created["track_count"].as_i64().unwrap(),
        0,
        "track_count stubbed at 0"
    );

    // List now sees it.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"].as_i64().unwrap(), id);

    // Get by id round-trips.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["id"].as_i64().unwrap(), id);
    assert_eq!(one["name"], "Bandes-son");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_with_explicit_color_and_icon(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({
            "name": "Live",
            "description": "Live recordings 2024-2026",
            "color_id": "crimson",
            "icon_id": "microphone",
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["color_id"], "crimson");
    assert_eq!(created["icon_id"], "microphone");
    assert_eq!(created["description"], "Live recordings 2024-2026");
}

/// Empty or whitespace-only `name` must 400 — the boundary validation
/// rejects the request before the storage round-trip, so a future
/// client bug can't ship blank-shelf rows into the DB.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_rejects_empty_name(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    for blank in ["", "   ", "\t\n "] {
        let resp = reqwest::Client::new()
            .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
            .header("x-user-id", user_id.to_string())
            .json(&json!({ "name": blank }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "name = {blank:?} should 400"
        );
    }

    // And nothing got persisted.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty(), "blank-name request leaked a row");
}

/// Foreign profile id under the calling user must 404 — no leak that
/// the profile exists at all on the box.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_under_foreign_profile_returns_404(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_a = mint_user(&base).await;
    let user_b = mint_user(&base).await;
    let profile_a = mint_profile(&base, user_a, "A's profile").await;

    // User B tries to create a library under user A's profile.
    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_a}/libraries"))
        .header("x-user-id", user_b.to_string())
        .json(&json!({ "name": "stolen" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // And user A's list under their own profile is still empty —
    // confirming nothing was written by the foreign call.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_a}/libraries"))
        .header("x-user-id", user_a.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty(), "foreign POST leaked into user A");
}

/// The tenant isolation battery: user B must NOT see, get, update or
/// delete a library belonging to user A — even through user A's own
/// profile id.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn tenants_are_isolated(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_a = mint_user(&base).await;
    let user_b = mint_user(&base).await;
    let profile_a = mint_profile(&base, user_a, "A's profile").await;
    let profile_b = mint_profile(&base, user_b, "B's profile").await;

    // User A creates a library.
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_a}/libraries"))
        .header("x-user-id", user_a.to_string())
        .json(&json!({ "name": "A's lib" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let lib_id = created["id"].as_i64().unwrap();

    // User B's list under their own profile is empty.
    let list_b: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_b}/libraries"))
        .header("x-user-id", user_b.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_b.is_empty());

    // User B's list under user A's profile is also empty (no leak).
    let list_b_under_a: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_a}/libraries"))
        .header("x-user-id", user_b.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        list_b_under_a.is_empty(),
        "user B saw user A's libraries via user A's profile id"
    );

    // User B can't GET A's library by id (whether through A's profile
    // or B's own profile — both must 404, no existence leak).
    for proxy_profile in [profile_a, profile_b] {
        let resp = reqwest::Client::new()
            .get(format!(
                "{base}/api/v1/profiles/{proxy_profile}/libraries/{lib_id}"
            ))
            .header("x-user-id", user_b.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "user B GET'd A's lib via profile {proxy_profile}"
        );
    }

    // User B can't PATCH A's library.
    let resp = reqwest::Client::new()
        .patch(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{lib_id}"
        ))
        .header("x-user-id", user_b.to_string())
        .json(&json!({ "name": "hijacked" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User B can't DELETE A's library.
    let resp = reqwest::Client::new()
        .delete(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{lib_id}"
        ))
        .header("x-user-id", user_b.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User A's library still exists, unmodified.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{lib_id}"
        ))
        .header("x-user-id", user_a.to_string())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["name"], "A's lib");
}

/// Update via PATCH round-trips and the response carries the new value
/// (verifying the `UPDATE … RETURNING …` path, not a stale read-back).
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_renames_in_place(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
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
        .patch(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": "New name", "color_id": "crimson" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(renamed["name"], "New name");
    assert_eq!(renamed["color_id"], "crimson");
    assert_eq!(renamed["id"].as_i64().unwrap(), id);
}

/// PATCH with a blank `name` (Some("") / whitespace-only) must 400.
/// Mirrors `create_rejects_empty_name` for the update path so a future
/// client bug can't silently blank an existing shelf label.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_rejects_empty_name(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": "Keep me" }))
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

    for blank in ["", "   ", "\t\n "] {
        let resp = reqwest::Client::new()
            .patch(format!(
                "{base}/api/v1/profiles/{profile_id}/libraries/{id}"
            ))
            .header("x-user-id", user_id.to_string())
            .json(&json!({ "name": blank }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "PATCH name = {blank:?} should 400"
        );
    }

    // And the existing name stuck — none of the rejected PATCHes
    // leaked through to the DB.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["name"], "Keep me");
}

/// Partial PATCH leaves omitted fields untouched (the `COALESCE` path
/// on the storage layer).
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_preserves_omitted_fields(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({
            "name": "Keep me",
            "color_id": "ocean",
            "icon_id": "headphones",
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();

    // Only flip color_id; name + icon_id must survive.
    let patched: Value = reqwest::Client::new()
        .patch(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "color_id": "sunset" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(patched["name"], "Keep me");
    assert_eq!(patched["color_id"], "sunset");
    assert_eq!(patched["icon_id"], "headphones");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn delete_returns_204_then_404(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": "doomed" }))
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

    let resp = reqwest::Client::new()
        .delete(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Subsequent GET on the same id is a 404.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Deleting a profile must cascade through to its libraries (the
/// `ON DELETE CASCADE` on `library.profile_id`). Hitting this from the
/// API side guards the migration against a future "I dropped the
/// CASCADE keyword" regression.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn profile_delete_cascades_to_libraries(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;

    // Two profiles so the delete-last-profile guard doesn't block us.
    // p2 stays alive so we can use it as a proxy GET path after p1
    // is deleted — a route through a *still-owned* profile is the
    // only way to prove the library row itself is gone (rather than
    // the request failing because the path's profile no longer exists).
    let p1 = mint_profile(&base, user_id, "one").await;
    let p2 = mint_profile(&base, user_id, "two").await;

    let lib_id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{p1}/libraries"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": "doomed lib" }))
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

    // Delete the profile.
    let resp = reqwest::Client::new()
        .delete(format!("{base}/api/v1/profiles/{p1}"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // The library must be gone too — any further GET surfaces a 404,
    // regardless of which (still-owned) profile id we proxy through.
    // Via p1 (the deleted profile): trivially 404 because the path's
    // profile is gone.
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{p1}/libraries/{lib_id}"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Via p2 (still owned): this is the cascade canary — if CASCADE
    // didn't fire the row would still exist (with `profile_id = p1`),
    // and while the `id = $1 AND profile_id = $2` clause would also
    // reject the lookup, a future change that loosens the scoping
    // would surface here as a leaked 200. Worth the extra request.
    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{p2}/libraries/{lib_id}"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Production-gate sanity: when `WAVEFLOW_DEV_AUTH` is off, the
/// nested route is gated by 503 just like every other `/api/v1/*`
/// endpoint. Mirrors `dev_auth_gate_returns_503_when_disabled` in
/// `tests/profiles.rs` so a future refactor that adds a router but
/// forgets the gate is caught here.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn dev_auth_gate_returns_503_for_libraries_when_disabled(pool: PgPool) {
    let base = support::spawn_app_prod_gate(pool).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/1/libraries"))
        .header("x-user-id", "42")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
