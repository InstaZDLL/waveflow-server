//! End-to-end tests for
//! `/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks`.
//!
//! Same harness pattern as `tests/libraries.rs`, extended one tier
//! deeper. The tenant-isolation battery covers every plausible
//! cross-tenant attack on the three-level path: foreign user, foreign
//! profile, foreign library, plus proxy attempts (user B pivoting
//! through user A's profile or library id). Cascade canaries
//! exercise the `track.library_id` and (transitively)
//! `library.profile_id` FK chains.

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_authenticated, spawn_two_authenticated};

async fn mint_profile(base: &str, token: &str, name: &str) -> i64 {
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles"))
        .bearer_auth(token)
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

async fn mint_library(base: &str, token: &str, profile_id: i64, name: &str) -> i64 {
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/libraries"))
        .bearer_auth(token)
        .json(&json!({ "name": name }))
        .send()
        .await
        .expect("library create failed")
        .error_for_status()
        .expect("non-2xx on library create")
        .json()
        .await
        .expect("library create body");
    created["id"].as_i64().expect("library id missing")
}

/// Canonical track payload — the fields the server's create handler
/// requires plus a couple of optionals, kept consistent across tests
/// so a future schema bump only needs editing here. `file_path` is
/// parameterised because the `(library_id, file_path)` UNIQUE index
/// would reject duplicates within the same library.
fn track_body(title: &str, file_path: &str) -> Value {
    json!({
        "title": title,
        "file_path": file_path,
        "file_size": 1234567,
        "duration_ms": 200000,
        "track_number": 1,
        "disc_number": 1,
        "year": 2026,
        "bitrate": 320,
        "sample_rate": 44100,
        "channels": 2,
        "bit_depth": 16,
        "codec": "FLAC",
    })
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_then_list_then_get_under_library(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Bandes-son").await;

    // Empty list initially.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty());

    // Create one.
    let created: Value = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&track_body("Cosmic Dust", "/music/cosmic_dust.flac"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_i64().unwrap();
    assert_eq!(created["title"], "Cosmic Dust");
    assert_eq!(created["library_id"].as_i64().unwrap(), library_id);
    assert_eq!(created["duration_ms"].as_i64().unwrap(), 200000);
    assert!(
        created["rating"].is_null(),
        "freshly inserted track should have no rating"
    );

    // List sees it.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
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
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["id"].as_i64().unwrap(), id);
    assert_eq!(one["title"], "Cosmic Dust");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_rejects_empty_title(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    for blank in ["", "   ", "\t\n "] {
        let mut body = track_body(blank, "/x.flac");
        body["title"] = json!(blank);
        let resp = reqwest::Client::new()
            .post(format!(
                "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
            ))
            .bearer_auth(&auth.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "title = {blank:?} should 400"
        );
    }

    let list: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty(), "blank-title request leaked a row");
}

/// Blank `file_path` must also 400 — preventing rows that would
/// collide on the `(library_id, file_path)` UNIQUE index with a
/// confusing 5xx, and rejecting an obviously invalid identifier
/// before storage.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_rejects_empty_file_path(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let resp = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&track_body("Title", "   "))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// Rating = 256 must be rejected by serde at the deserialization
/// boundary — `Option<u8>` on `UpdateTrackRequest` cuts the value
/// off before the handler runs. 400 (or 422 depending on body
/// parsing) — we accept either as long as it's a client error.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_rejects_out_of_range_rating(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let id = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&track_body("Track", "/track.flac"))
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
        .patch(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}"
        ))
        .bearer_auth(&auth.token)
        .json(&json!({ "rating": 256 }))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_client_error(),
        "rating = 256 must be a client error, got {}",
        resp.status()
    );
}

/// Foreign library id under the calling user must 404.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_under_foreign_library_returns_404(pool: PgPool) {
    let two = spawn_two_authenticated(pool, "test-user-a", "test-user-b").await;
    let base = two.base.clone();
    let a = &two.a;
    let b = &two.b;
    let _user_a = a.user_id;
    let _user_b = b.user_id;
    let profile_a = mint_profile(&base, &a.token, "A").await;
    let library_a = mint_library(&base, &a.token, profile_a, "A's lib").await;
    let profile_b = mint_profile(&base, &b.token, "B").await;

    // User B tries to drop a track into user A's library, proxying
    // through their own profile.
    let resp = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_b}/libraries/{library_a}/tracks"
        ))
        .bearer_auth(&b.token)
        .json(&track_body("stolen", "/stolen.flac"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // And also via user A's profile (user B doesn't own profile_a
    // either).
    let resp = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/tracks"
        ))
        .bearer_auth(&b.token)
        .json(&track_body("stolen", "/stolen2.flac"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User A's library is still empty — nothing was written.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/tracks"
        ))
        .bearer_auth(&a.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list.is_empty(), "foreign POST leaked into user A's library");
}

/// Full tenant isolation battery on the 3-tier path. User B must NOT
/// see, GET, PATCH or DELETE user A's track — no matter which proxy
/// (profile, library) they try.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn tenants_are_isolated(pool: PgPool) {
    let two = spawn_two_authenticated(pool, "test-user-a", "test-user-b").await;
    let base = two.base.clone();
    let a = &two.a;
    let b = &two.b;
    let _user_a = a.user_id;
    let _user_b = b.user_id;
    let profile_a = mint_profile(&base, &a.token, "A").await;
    let library_a = mint_library(&base, &a.token, profile_a, "A's lib").await;
    let profile_b = mint_profile(&base, &b.token, "B").await;
    let library_b = mint_library(&base, &b.token, profile_b, "B's lib").await;

    // User A creates a track.
    let created: Value = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/tracks"
        ))
        .bearer_auth(&a.token)
        .json(&track_body("A's track", "/a.flac"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let track_id = created["id"].as_i64().unwrap();

    // User B's list in their own library is empty.
    let list_b: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_b}/libraries/{library_b}/tracks"
        ))
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(list_b.is_empty());

    // User B can't list user A's tracks via any proxy combination
    // of profile / library ids.
    for (proxy_profile, proxy_library) in [
        (profile_a, library_a),
        (profile_b, library_a),
        (profile_a, library_b),
    ] {
        let list_b_proxy: Vec<Value> = reqwest::Client::new()
            .get(format!(
                "{base}/api/v1/profiles/{proxy_profile}/libraries/{proxy_library}/tracks"
            ))
            .bearer_auth(&b.token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(
            list_b_proxy.is_empty(),
            "user B saw user A's tracks via (profile={proxy_profile}, library={proxy_library})"
        );

        let resp = reqwest::Client::new()
            .get(format!(
                "{base}/api/v1/profiles/{proxy_profile}/libraries/{proxy_library}/tracks/{track_id}"
            ))
            .bearer_auth(&b.token)
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "user B GET'd A's track via (profile={proxy_profile}, library={proxy_library})"
        );
    }

    // User B can't PATCH or DELETE A's track via the proxy that
    // matches A's profile + library.
    let resp = reqwest::Client::new()
        .patch(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/tracks/{track_id}"
        ))
        .bearer_auth(&b.token)
        .json(&json!({ "title": "hijacked" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let resp = reqwest::Client::new()
        .delete(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/tracks/{track_id}"
        ))
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User A's track is still there, unmodified.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/tracks/{track_id}"
        ))
        .bearer_auth(&a.token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(one["title"], "A's track");
}

/// PATCH round-trips and the response carries the new value (the
/// `UPDATE … RETURNING …` path, not a stale read-back).
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_round_trips(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let id = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&track_body("Original", "/original.flac"))
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
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}"
        ))
        .bearer_auth(&auth.token)
        .json(&json!({ "title": "Renamed", "rating": 200 }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(renamed["title"], "Renamed");
    assert_eq!(renamed["rating"].as_i64().unwrap(), 200);
    // Track number from the original create is preserved — the
    // COALESCE path leaves omitted fields untouched.
    assert_eq!(renamed["track_number"].as_i64().unwrap(), 1);
}

/// PATCH `Some("")` title must 400 — same boundary check as create,
/// `None` stays legitimate.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_rejects_empty_title(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let id = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&track_body("Keep me", "/keep.flac"))
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
                "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}"
            ))
            .bearer_auth(&auth.token)
            .json(&json!({ "title": blank }))
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "PATCH title = {blank:?} should 400"
        );
    }

    // Original title is untouched.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}"
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
    assert_eq!(one["title"], "Keep me");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn delete_returns_204_then_404(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let id = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&track_body("doomed", "/doomed.flac"))
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
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{id}"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Deleting a library must cascade through to its tracks
/// (`ON DELETE CASCADE` on `track.library_id`). Hitting this from
/// the API guards the migration against a future "I dropped the
/// CASCADE keyword" regression.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn library_delete_cascades_to_tracks(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    // Two libraries — one to delete with its tracks, one to keep
    // as a still-owned proxy for the post-delete probe.
    let l1 = mint_library(&auth.base, &auth.token, profile_id, "to delete").await;
    let l2 = mint_library(&auth.base, &auth.token, profile_id, "to keep").await;

    let track_id = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{l1}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&track_body("doomed track", "/doomed.flac"))
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

    // Delete the library.
    let resp = reqwest::Client::new()
        .delete(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{l1}"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // GET via the deleted library — trivially 404.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{l1}/tracks/{track_id}"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // GET via the still-owned library — this is the cascade canary.
    // Without CASCADE the track would still exist with
    // `library_id = l1`, and a future relaxation of the
    // `id = $1 AND library_id = $2` scoping would surface a leaked
    // 200 here.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{l2}/tracks/{track_id}"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Deleting a profile must cascade through library → track. Tests
/// the transitive chain so a future "I broke one of the FKs" change
/// surfaces here.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn profile_delete_cascades_to_tracks(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;

    // Two profiles so the delete-last-profile guard doesn't block
    // us; one to delete, one to keep as the post-delete probe path.
    let p1 = mint_profile(&auth.base, &auth.token, "to delete").await;
    let p2 = mint_profile(&auth.base, &auth.token, "to keep").await;
    let l1 = mint_library(&auth.base, &auth.token, p1, "lib").await;
    let l2 = mint_library(&auth.base, &auth.token, p2, "kept lib").await;

    let track_id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{p1}/libraries/{l1}/tracks"))
        .bearer_auth(&auth.token)
        .json(&track_body("doomed track", "/doomed.flac"))
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

    // Delete the profile — should cascade through library to track.
    let resp = reqwest::Client::new()
        .delete(format!("{base}/api/v1/profiles/{p1}"))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Track gone via the still-owned profile + library proxy. If
    // CASCADE didn't fire on either hop the row would still exist
    // with `library_id = l1`.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{p2}/libraries/{l2}/tracks/{track_id}"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// `(library_id, file_path)` UNIQUE — a second create with the same
/// pair under the same owner surfaces as a server 5xx today (the FK
/// violation isn't translated to a friendlier 409). Locking in the
/// current behaviour so a future shift to 409 is an explicit choice
/// rather than an accidental drift.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn duplicate_file_path_under_same_library_fails(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let body = track_body("first", "/dup.flac");
    let resp = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = reqwest::Client::new()
        .post(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "duplicate file_path under the same library should currently 5xx"
    );
}
