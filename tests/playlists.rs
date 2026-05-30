//! End-to-end tests for `/api/v1/profiles/{profile_id}/playlists`.
//!
//! Same harness pattern as `tests/libraries.rs` — a playlist sits at
//! the same depth as a library (direct child of a profile), so the
//! tenant-isolation battery is essentially the library suite
//! re-applied: 401 gate, default color / icon fall-back, foreign
//! profile 404 on POST, the proxy-attack battery covering all
//! `(proxy_profile, target_playlist)` combinations, partial PATCH
//! preservation, CASCADE from profile, prod-gate 503. Plus the
//! 1.b.5c-specific assertions on `is_smart=0` and `cover_is_auto=1`
//! defaults — those are sticky-flag invariants whose drift would
//! quietly break a future server-side smart-playlist or auto-cover
//! pipeline (cf. CR finding on waveflow#188 that flipped
//! `cover_is_auto` from 0 → 1).

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::spawn_app;

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
async fn playlists_require_x_user_id(pool: PgPool) {
    let base = spawn_app(pool).await;

    for (method, path) in [
        ("GET", "/api/v1/profiles/1/playlists"),
        ("POST", "/api/v1/profiles/1/playlists"),
        ("GET", "/api/v1/profiles/1/playlists/1"),
        ("PATCH", "/api/v1/profiles/1/playlists/1"),
        ("DELETE", "/api/v1/profiles/1/playlists/1"),
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

    // Empty list initially.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
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
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": "Soirée" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["name"], "Soirée");
    assert_eq!(created["color_id"], "violet", "default color_id");
    assert_eq!(created["icon_id"], "music", "default icon_id");

    // Sticky-flag invariants — drift here would break a future
    // smart-playlist or auto-cover pipeline silently.
    assert_eq!(
        created["is_smart"].as_i64().unwrap(),
        0,
        "freshly created playlist must be custom (is_smart=0)"
    );
    assert!(
        created["smart_rules"].is_null(),
        "custom playlist must have smart_rules=NULL"
    );
    assert_eq!(
        created["cover_is_auto"].as_i64().unwrap(),
        1,
        "no-manual-cover playlist must be auto-managed (cover_is_auto=1, cf. waveflow#188 CR)"
    );
    assert!(
        created["cover_hash"].is_null(),
        "fresh playlist must have no cover_hash yet"
    );
    assert_eq!(
        created["track_count"].as_i64().unwrap(),
        0,
        "track_count stubbed at 0 until playlist_track ships"
    );
    assert_eq!(
        created["total_duration_ms"].as_i64().unwrap(),
        0,
        "total_duration_ms stubbed at 0 until playlist_track ships"
    );

    // List sees it.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"].as_i64().unwrap(), id);

    // Get round-trips.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["id"].as_i64().unwrap(), id);
    assert_eq!(one["name"], "Soirée");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_with_explicit_color_and_icon(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({
            "name": "Focus",
            "description": "Lo-fi pour bosser",
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
    assert_eq!(created["color_id"], "ocean");
    assert_eq!(created["icon_id"], "headphones");
    assert_eq!(created["description"], "Lo-fi pour bosser");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_rejects_empty_name(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    for blank in ["", "   ", "\t\n "] {
        let resp = reqwest::Client::new()
            .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
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

    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty(), "blank-name request leaked a row");
}

/// Foreign profile id under the calling user must 404.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_under_foreign_profile_returns_404(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_a = mint_user(&base).await;
    let user_b = mint_user(&base).await;
    let profile_a = mint_profile(&base, user_a, "A's profile").await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_a}/playlists"))
        .header("x-user-id", user_b.to_string())
        .json(&json!({ "name": "stolen" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_a}/playlists"))
        .header("x-user-id", user_a.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty(), "foreign POST leaked into user A");
}

/// Full tenant isolation battery: user B must NOT see, get, update,
/// or delete user A's playlist — neither through their own profile
/// nor through user A's profile.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn tenants_are_isolated(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_a = mint_user(&base).await;
    let user_b = mint_user(&base).await;
    let profile_a = mint_profile(&base, user_a, "A").await;
    let profile_b = mint_profile(&base, user_b, "B").await;

    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_a}/playlists"))
        .header("x-user-id", user_a.to_string())
        .json(&json!({ "name": "A's playlist" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let playlist_id = created["id"].as_i64().unwrap();

    // User B's list under their own profile is empty.
    let list_b: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_b}/playlists"))
        .header("x-user-id", user_b.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_b.is_empty());

    // User B can't list user A's playlists via user A's profile id.
    let list_b_proxy: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_a}/playlists"))
        .header("x-user-id", user_b.to_string())
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        list_b_proxy.is_empty(),
        "user B saw user A's playlists via user A's profile id"
    );

    // User B can't GET A's playlist via either proxy.
    for proxy_profile in [profile_a, profile_b] {
        let resp = reqwest::Client::new()
            .get(format!(
                "{base}/api/v1/profiles/{proxy_profile}/playlists/{playlist_id}"
            ))
            .header("x-user-id", user_b.to_string())
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "user B GET'd A's playlist via profile {proxy_profile}"
        );
    }

    // User B can't PATCH A's playlist.
    let resp = reqwest::Client::new()
        .patch(format!(
            "{base}/api/v1/profiles/{profile_a}/playlists/{playlist_id}"
        ))
        .header("x-user-id", user_b.to_string())
        .json(&json!({ "name": "hijacked" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User B can't DELETE A's playlist.
    let resp = reqwest::Client::new()
        .delete(format!(
            "{base}/api/v1/profiles/{profile_a}/playlists/{playlist_id}"
        ))
        .header("x-user-id", user_b.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User A's playlist is still there, unmodified.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/playlists/{playlist_id}"
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
    assert_eq!(one["name"], "A's playlist");
}

/// PATCH round-trips and the response carries the new value.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_renames_in_place(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": "Old name" }))
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
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
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

/// Partial PATCH preserves omitted fields (the `COALESCE` path).
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_preserves_omitted_fields(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
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

    let patched: Value = reqwest::Client::new()
        .patch(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
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

/// PATCH blank name must 400 — same boundary check as create.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_rejects_empty_name(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
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
                "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
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

    // Original name is untouched.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
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

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn delete_returns_204_then_404(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;
    let profile_id = mint_profile(&base, user_id, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
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
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Deleting a profile must cascade through to its playlists.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn profile_delete_cascades_to_playlists(pool: PgPool) {
    let base = spawn_app(pool).await;
    let user_id = mint_user(&base).await;

    // Two profiles so the delete-last-profile guard doesn't block us.
    let p1 = mint_profile(&base, user_id, "to delete").await;
    let p2 = mint_profile(&base, user_id, "to keep").await;

    let playlist_id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{p1}/playlists"))
        .header("x-user-id", user_id.to_string())
        .json(&json!({ "name": "doomed playlist" }))
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
        .delete(format!("{base}/api/v1/profiles/{p1}"))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Via p1 (deleted): trivially 404.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{p1}/playlists/{playlist_id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Via p2 (still owned): real cascade canary. If CASCADE didn't
    // fire the row would still exist with `profile_id = p1`.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{p2}/playlists/{playlist_id}"
        ))
        .header("x-user-id", user_id.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Production-gate sanity.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn dev_auth_gate_returns_503_for_playlists_when_disabled(pool: PgPool) {
    let base = support::spawn_app_prod_gate(pool).await;

    let resp = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/1/playlists"))
        .header("x-user-id", "42")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
