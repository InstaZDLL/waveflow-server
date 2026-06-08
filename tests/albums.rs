//! End-to-end tests for
//! `/api/v1/profiles/{profile_id}/libraries/{library_id}/albums[/{id}/tracks]`
//! (Phase 4.d.0.4).
//!
//! Direct SQL inserts pre-populate the `album` / `artist` /
//! `track_artist` / `track.album_id` rows — the API has no write
//! surface for these tables (album rows materialise from the sync
//! apply pipeline, not from a CRUD form). Bypassing the apply path
//! keeps the test scope tight on the read surface introduced here.
//!
//! Tenant isolation battery mirrors the shape of `tracks.rs`: a
//! second authenticated caller proves a foreign user cannot pivot
//! through their own profile id to reach user A's library.

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_authenticated, spawn_two_authenticated};

/// Fixed sentinel for `created_at` / `updated_at`. The tests don't
/// assert on time, so freezing the value removes nondeterminism. A
/// per-row offset lets us pin the `updated_at DESC` ordering without
/// time-of-day flakiness.
const BASE_NOW_MS: i64 = 1_700_000_000_000;

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

async fn insert_artist(pool: &PgPool, library_id: i64, name: &str, now_ms: i64) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO artist (library_id, name, created_at, updated_at)
         VALUES ($1, $2, $3, $3) RETURNING id",
    )
    .bind(library_id)
    .bind(name)
    .bind(now_ms)
    .fetch_one(pool)
    .await
    .expect("insert artist")
}

async fn insert_album(
    pool: &PgPool,
    library_id: i64,
    canonical_title: &str,
    album_artist_id: Option<i64>,
    is_compilation: bool,
    now_ms: i64,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO album
            (library_id, canonical_title, album_artist_id,
             is_compilation, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $5) RETURNING id",
    )
    .bind(library_id)
    .bind(canonical_title)
    .bind(album_artist_id)
    .bind(is_compilation)
    .bind(now_ms)
    .fetch_one(pool)
    .await
    .expect("insert album")
}

#[allow(clippy::too_many_arguments)]
async fn insert_track(
    pool: &PgPool,
    library_id: i64,
    file_path: &str,
    title: &str,
    album_id: Option<i64>,
    disc_number: Option<i64>,
    track_number: Option<i64>,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO track
            (library_id, file_path, file_size, title, duration_ms,
             album_id, disc_number, track_number, added_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING id",
    )
    .bind(library_id)
    .bind(file_path)
    .bind(1024_i64)
    .bind(title)
    .bind(180_000_i64)
    .bind(album_id)
    .bind(disc_number)
    .bind(track_number)
    .bind(BASE_NOW_MS)
    .fetch_one(pool)
    .await
    .expect("insert track")
}

// ──────────────────────────────────────────────────────────────────
// list_albums
// ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_albums_empty_returns_200_empty(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/albums",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Vec<Value> = resp.json().await.unwrap();
    assert!(body.is_empty());
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_albums_returns_rows_ordered_by_updated_at_desc(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let artist_id = insert_artist(&pool, library_id, "Daft Punk", BASE_NOW_MS).await;
    // Insert oldest → newest so a wrong ORDER BY surfaces in the
    // assertion order.
    let old = insert_album(
        &pool,
        library_id,
        "Discovery",
        Some(artist_id),
        false,
        BASE_NOW_MS,
    )
    .await;
    let mid = insert_album(
        &pool,
        library_id,
        "Random Access Memories",
        Some(artist_id),
        false,
        BASE_NOW_MS + 1_000,
    )
    .await;
    let new = insert_album(
        &pool,
        library_id,
        "Homework",
        Some(artist_id),
        false,
        BASE_NOW_MS + 2_000,
    )
    .await;

    let body: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/albums",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<i64> = body.iter().map(|v| v["id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![new, mid, old], "must order by updated_at DESC");
    // album_artist_name is joined-through so the web client doesn't
    // have to resolve N names.
    assert_eq!(body[0]["album_artist_name"], "Daft Punk");
    assert_eq!(body[0]["album_artist_id"].as_i64().unwrap(), artist_id);
    assert!(!body[0]["is_compilation"].as_bool().unwrap());
}

/// `id ASC` is the documented tiebreaker when `updated_at` ties —
/// the apply pipeline upserts a whole sync round in one transaction,
/// so several rows can land at the same epoch millisecond. Without
/// the tiebreaker the ordering would shuffle on each request. This
/// test guards against a refactor that drops `, id ASC` from the
/// ORDER BY.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_albums_id_ascending_breaks_updated_at_ties(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    // Three rows at the SAME updated_at — inserted out of natural
    // order so a wrong tiebreaker would surface.
    let second = insert_album(&pool, library_id, "Beta", None, false, BASE_NOW_MS).await;
    let first = insert_album(&pool, library_id, "Alpha", None, false, BASE_NOW_MS).await;
    let third = insert_album(&pool, library_id, "Gamma", None, false, BASE_NOW_MS).await;

    let body: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/albums",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<i64> = body.iter().map(|v| v["id"].as_i64().unwrap()).collect();
    // `id ASC` on the tie — BIGSERIAL allocates monotonically so
    // (second < first < third) by insertion order.
    assert_eq!(ids, vec![second, first, third]);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_albums_compilation_has_null_album_artist(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    insert_album(
        &pool,
        library_id,
        "Now That's What I Call Music",
        None,
        true,
        BASE_NOW_MS,
    )
    .await;

    let body: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/albums",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body.len(), 1);
    assert!(body[0]["album_artist_id"].is_null());
    assert!(body[0]["album_artist_name"].is_null());
    assert!(body[0]["is_compilation"].as_bool().unwrap());
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_albums_foreign_library_returns_404(pool: PgPool) {
    let two = spawn_two_authenticated(pool.clone(), "a", "b").await;
    let base = two.base.clone();
    let profile_a = mint_profile(&base, &two.a.token, "A").await;
    let library_a = mint_library(&base, &two.a.token, profile_a, "A's lib").await;
    let profile_b = mint_profile(&base, &two.b.token, "B").await;

    // User B pivots through their own profile id to user A's library.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_b}/libraries/{library_a}/albums",
        ))
        .bearer_auth(&two.b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // And through user A's profile id (still not B's).
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/albums",
        ))
        .bearer_auth(&two.b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ──────────────────────────────────────────────────────────────────
// list_album_tracks (drill-down)
// ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_album_tracks_orders_by_disc_then_track_number(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let artist_id = insert_artist(&pool, library_id, "The Beatles", BASE_NOW_MS).await;
    let album_id = insert_album(
        &pool,
        library_id,
        "The Beatles (White Album)",
        Some(artist_id),
        false,
        BASE_NOW_MS,
    )
    .await;
    // Disc 2 / track 1 ("Birthday") inserted BEFORE Disc 1 / track 1
    // ("Back in the U.S.S.R.") so a wrong ORDER BY surfaces.
    let d2t1 = insert_track(
        &pool,
        library_id,
        "/m/d2t1.flac",
        "Birthday",
        Some(album_id),
        Some(2),
        Some(1),
    )
    .await;
    let d1t1 = insert_track(
        &pool,
        library_id,
        "/m/d1t1.flac",
        "Back in the U.S.S.R.",
        Some(album_id),
        Some(1),
        Some(1),
    )
    .await;
    let d1t2 = insert_track(
        &pool,
        library_id,
        "/m/d1t2.flac",
        "Dear Prudence",
        Some(album_id),
        Some(1),
        Some(2),
    )
    .await;

    let body: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/albums/{album_id}/tracks",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<i64> = body.iter().map(|v| v["id"].as_i64().unwrap()).collect();
    assert_eq!(
        ids,
        vec![d1t1, d1t2, d2t1],
        "must order by (disc_number, track_number)"
    );
    // Every row carries the path-album id back — `TrackResponse.album_id`
    // surfaces the materialised link so the artist-drill-down sibling
    // endpoint can deep-link tracks to their album (Phase 4.d.0.4).
    for row in &body {
        assert_eq!(row["album_id"].as_i64().unwrap(), album_id);
    }
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_album_tracks_empty_album_returns_200_empty(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let album_id = insert_album(&pool, library_id, "Lone Album", None, false, BASE_NOW_MS).await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/albums/{album_id}/tracks",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Vec<Value> = resp.json().await.unwrap();
    assert!(body.is_empty());
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_album_tracks_missing_album_returns_404(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/albums/9999/tracks",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_album_tracks_foreign_user_returns_404(pool: PgPool) {
    let two = spawn_two_authenticated(pool.clone(), "a", "b").await;
    let base = two.base.clone();
    let profile_a = mint_profile(&base, &two.a.token, "A").await;
    let library_a = mint_library(&base, &two.a.token, profile_a, "A's lib").await;
    let album_a = insert_album(&pool, library_a, "Secret", None, false, BASE_NOW_MS).await;
    let _track_a = insert_track(
        &pool,
        library_a,
        "/secret.flac",
        "Secret Track",
        Some(album_a),
        None,
        None,
    )
    .await;

    // B → A's album via A's profile id: foreign user 404.
    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/albums/{album_a}/tracks",
        ))
        .bearer_auth(&two.b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_album_tracks_wrong_library_id_returns_404(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_a = mint_library(&auth.base, &auth.token, profile_id, "A").await;
    let library_b = mint_library(&auth.base, &auth.token, profile_id, "B").await;
    // Album lives in library_a but caller mounts it under library_b
    // — composite scope must reject.
    let album_a = insert_album(&pool, library_a, "X", None, false, BASE_NOW_MS).await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_b}/albums/{album_a}/tracks",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
