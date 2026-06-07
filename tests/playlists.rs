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

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn create_then_list_then_get_under_profile(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    // Empty list initially.
    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
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
        .bearer_auth(&auth.token)
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
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
        ))
        .bearer_auth(&auth.token)
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
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
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
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    for blank in ["", "   ", "\t\n "] {
        let resp = reqwest::Client::new()
            .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
            .bearer_auth(&auth.token)
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
        .bearer_auth(&auth.token)
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
    let two = spawn_two_authenticated(pool, "test-user-a", "test-user-b").await;
    let base = two.base.clone();
    let a = &two.a;
    let b = &two.b;
    let _user_a = a.user_id;
    let _user_b = b.user_id;
    let profile_a = mint_profile(&base, &a.token, "A's profile").await;

    let resp = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_a}/playlists"))
        .bearer_auth(&b.token)
        .json(&json!({ "name": "stolen" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let list: Vec<Value> = reqwest::Client::new()
        .get(format!("{base}/api/v1/profiles/{profile_a}/playlists"))
        .bearer_auth(&a.token)
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
    let two = spawn_two_authenticated(pool, "test-user-a", "test-user-b").await;
    let base = two.base.clone();
    let a = &two.a;
    let b = &two.b;
    let _user_a = a.user_id;
    let _user_b = b.user_id;
    let profile_a = mint_profile(&base, &a.token, "A").await;
    let profile_b = mint_profile(&base, &b.token, "B").await;

    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_a}/playlists"))
        .bearer_auth(&a.token)
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
        .bearer_auth(&b.token)
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
        .bearer_auth(&b.token)
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
            .bearer_auth(&b.token)
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
        .bearer_auth(&b.token)
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
        .bearer_auth(&b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // User A's playlist is still there, unmodified.
    let one: Value = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/playlists/{playlist_id}"
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
    assert_eq!(one["name"], "A's playlist");
}

/// PATCH round-trips and the response carries the new value.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn update_renames_in_place(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
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
        .bearer_auth(&auth.token)
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
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let created: Value = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
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
        .bearer_auth(&auth.token)
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
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
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
            .bearer_auth(&auth.token)
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
        .bearer_auth(&auth.token)
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
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
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
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// Deleting a profile must cascade through to its playlists.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn profile_delete_cascades_to_playlists(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let _user_id = auth.user_id;

    // Two profiles so the delete-last-profile guard doesn't block us.
    let p1 = mint_profile(&auth.base, &auth.token, "to delete").await;
    let p2 = mint_profile(&auth.base, &auth.token, "to keep").await;

    let playlist_id = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{p1}/playlists"))
        .bearer_auth(&auth.token)
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
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // Via p1 (deleted): trivially 404.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{p1}/playlists/{playlist_id}"
        ))
        .bearer_auth(&auth.token)
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
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------
// Phase 1.j.c — owner-facing `/tracks` read (Sprint 4.c.4)
// ---------------------------------------------------------------
//
// `playlist_track` is populated by the apply pipeline today; tests
// here seed the rows directly via the harness pool so the assertion
// surface stays on the owner endpoint's contract (ownership chain +
// ordering + nullable snapshot pass-through) instead of leaking the
// apply pipeline's own behaviour into every assertion.

/// Direct INSERT into `playlist_track` — bypasses the apply pipeline
/// because these tests target the owner endpoint, not the sync drain.
/// `added_at` is a real epoch-millis value so the response field
/// can be asserted on (a hard-coded `0` would mask a future
/// projection drift where the column order in `fetch_for_owner`'s
/// SELECT gets reshuffled).
async fn seed_playlist_track(
    pool: &PgPool,
    playlist_id: i64,
    track_id: i64,
    position: i32,
    added_at: i64,
    snapshot: Option<(&str, Option<&str>, Option<i64>)>,
) {
    let (title, artist, duration_ms) = match snapshot {
        Some((t, a, d)) => (Some(t.to_string()), a.map(|s| s.to_string()), d),
        None => (None, None, None),
    };
    sqlx::query(
        "INSERT INTO playlist_track
              (playlist_id, track_id, position, added_at,
               snapshot_title, snapshot_artist, snapshot_duration_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(playlist_id)
    .bind(track_id)
    .bind(position)
    .bind(added_at)
    .bind(title)
    .bind(artist)
    .bind(duration_ms)
    .execute(pool)
    .await
    .expect("seed playlist_track");
}

/// Empty playlist returns `[]` (200), NOT 404 — the row exists, it
/// just has no tracks.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_tracks_empty_playlist_returns_empty_array(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let id: i64 = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
        .json(&json!({ "name": "Empty" }))
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
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}/tracks"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let tracks: Vec<Value> = resp.json().await.unwrap();
    assert!(
        tracks.is_empty(),
        "fresh playlist must return [], got {tracks:?}",
    );
}

/// Tracks come back in `(position ASC, track_id ASC)` order, with
/// the snapshot fields + `added_at` passed through verbatim —
/// including NULL snapshots (the owner is allowed to see pre-1.j.b
/// rows; the share-preview filter doesn't apply here). Also
/// exercises the `track_id ASC` tiebreaker by seeding two rows that
/// share a position — a missing tiebreaker would let the SQL
/// shuffle them.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_tracks_returns_position_order_with_snapshots(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let base = auth.base.clone();
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let id: i64 = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
        .json(&json!({ "name": "Soirée" }))
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

    // Insert in REVERSE position order to prove ORDER BY actually
    // fires (a missing ORDER BY would surface the insert order).
    // `added_at` values are distinct + non-zero so the response
    // field can be asserted on (a hard-coded 0 would mask a future
    // projection drift in `fetch_for_owner`'s SELECT).
    //
    // Rows at positions 2 + 3 deliberately share the same position
    // to exercise the `track_id ASC` tiebreaker — without it the
    // SQL would be free to swap their order.
    seed_playlist_track(
        &auth.pool,
        id,
        904,
        2,
        1_700_000_000_003,
        Some(("Track 4", Some("Artist W"), Some(190_000))),
    )
    .await;
    seed_playlist_track(
        &auth.pool,
        id,
        903,
        2,
        1_700_000_000_002,
        Some(("Track 3", Some("Artist Z"), Some(180_000))),
    )
    .await;
    // Row whose snapshot is NULL — owner read MUST still return it.
    seed_playlist_track(&auth.pool, id, 902, 1, 1_700_000_000_001, None).await;
    seed_playlist_track(
        &auth.pool,
        id,
        901,
        0,
        1_700_000_000_000,
        Some(("Track 1", Some("Artist A"), Some(210_000))),
    )
    .await;

    let tracks: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}/tracks"
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
    assert_eq!(tracks.len(), 4);

    assert_eq!(tracks[0]["track_id"].as_i64().unwrap(), 901);
    assert_eq!(tracks[0]["position"].as_i64().unwrap(), 0);
    assert_eq!(tracks[0]["added_at"].as_i64().unwrap(), 1_700_000_000_000);
    assert_eq!(tracks[0]["snapshot_title"], "Track 1");
    assert_eq!(tracks[0]["snapshot_artist"], "Artist A");
    assert_eq!(tracks[0]["snapshot_duration_ms"].as_i64().unwrap(), 210_000);

    assert_eq!(tracks[1]["track_id"].as_i64().unwrap(), 902);
    assert_eq!(tracks[1]["position"].as_i64().unwrap(), 1);
    assert_eq!(tracks[1]["added_at"].as_i64().unwrap(), 1_700_000_000_001);
    assert!(
        tracks[1]["snapshot_title"].is_null(),
        "pre-1.j.b row MUST surface to the owner with NULL snapshot",
    );
    assert!(tracks[1]["snapshot_artist"].is_null());
    assert!(tracks[1]["snapshot_duration_ms"].is_null());

    // Tiebreaker proof: 903 (lower track_id) MUST come before 904
    // even though both share position=2 and 904 was inserted FIRST.
    assert_eq!(tracks[2]["track_id"].as_i64().unwrap(), 903);
    assert_eq!(tracks[2]["position"].as_i64().unwrap(), 2);
    assert_eq!(tracks[2]["added_at"].as_i64().unwrap(), 1_700_000_000_002);
    assert_eq!(tracks[3]["track_id"].as_i64().unwrap(), 904);
    assert_eq!(tracks[3]["position"].as_i64().unwrap(), 2);
    assert_eq!(tracks[3]["added_at"].as_i64().unwrap(), 1_700_000_000_003);
}

/// Tenant-isolation: user B cannot list user A's playlist tracks
/// through user A's profile id (the proxy attack). 404, not 403 —
/// blur with non-existence to avoid leaking that the playlist
/// exists under a different owner.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_tracks_foreign_tenant_returns_404(pool: PgPool) {
    let two = spawn_two_authenticated(pool, "test-user-a", "test-user-b").await;
    let base = two.base.clone();
    let a = &two.a;
    let b = &two.b;
    let profile_a = mint_profile(&base, &a.token, "A").await;
    let profile_b = mint_profile(&base, &b.token, "B").await;

    let playlist_id: i64 = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_a}/playlists"))
        .bearer_auth(&a.token)
        .json(&json!({ "name": "A's playlist" }))
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

    seed_playlist_track(
        &two.pool,
        playlist_id,
        1001,
        0,
        1_700_000_000_000,
        Some(("A's track", Some("A's artist"), Some(100_000))),
    )
    .await;

    // Cover BOTH proxy attacks:
    //   - via user A's profile id (the real owner's chain) → wrong user
    //   - via user B's profile id (user B's own chain) → wrong profile
    // Both must 404 with the same shape so the response doesn't
    // leak existence.
    for proxy_profile in [profile_a, profile_b] {
        let resp = reqwest::Client::new()
            .get(format!(
                "{base}/api/v1/profiles/{proxy_profile}/playlists/{playlist_id}/tracks"
            ))
            .bearer_auth(&b.token)
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "user B listed A's tracks via profile {proxy_profile}"
        );
    }

    // Owner still gets the row through their own chain — proves
    // the foreign-tenant 404s above weren't because the row was
    // missing.
    let tracks: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/playlists/{playlist_id}/tracks"
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
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0]["track_id"].as_i64().unwrap(), 1001);
}

/// Unknown playlist id is 404 (not 500, not 200 with `[]`). Covers
/// the early-return path inside `fetch_for_owner` when the
/// ownership SELECT returns no row.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_tracks_unknown_playlist_is_404(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/999999/tracks"
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

/// 401 gate — the JWT middleware MUST reject anonymous requests
/// before they reach the handler. Symmetric with the 401 assertions
/// elsewhere in this suite.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_tracks_without_auth_is_401(pool: PgPool) {
    let auth = spawn_authenticated(pool, "test-user").await;
    let base = auth.base.clone();
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;

    let id: i64 = reqwest::Client::new()
        .post(format!("{base}/api/v1/profiles/{profile_id}/playlists"))
        .bearer_auth(&auth.token)
        .json(&json!({ "name": "Locked" }))
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
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/playlists/{id}/tracks"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
