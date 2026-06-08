//! End-to-end tests for
//! `/api/v1/profiles/{profile_id}/libraries/{library_id}/artists[/{id}/tracks]`
//! (Phase 4.d.0.4).
//!
//! Same direct-SQL pre-population as `tests/albums.rs` — no CRUD
//! write surface for `artist` / `track_artist`, the apply pipeline
//! owns those rows in production. The drill-down test covers the
//! multi-artist invariant: a track linked to two artists surfaces
//! under both, scoped by `library_id` to stay tenant-correct.

mod support;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use support::{spawn_authenticated, spawn_two_authenticated};

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

#[allow(clippy::too_many_arguments)]
async fn insert_track(
    pool: &PgPool,
    library_id: i64,
    file_path: &str,
    title: &str,
    disc_number: Option<i64>,
    track_number: Option<i64>,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO track
            (library_id, file_path, file_size, title, duration_ms,
             disc_number, track_number, added_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) RETURNING id",
    )
    .bind(library_id)
    .bind(file_path)
    .bind(1024_i64)
    .bind(title)
    .bind(180_000_i64)
    .bind(disc_number)
    .bind(track_number)
    .bind(BASE_NOW_MS)
    .fetch_one(pool)
    .await
    .expect("insert track")
}

async fn insert_album(
    pool: &PgPool,
    library_id: i64,
    canonical_title: &str,
    album_artist_id: Option<i64>,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO album
            (library_id, canonical_title, album_artist_id, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $4) RETURNING id",
    )
    .bind(library_id)
    .bind(canonical_title)
    .bind(album_artist_id)
    .bind(BASE_NOW_MS)
    .fetch_one(pool)
    .await
    .expect("insert album")
}

async fn set_track_album(pool: &PgPool, track_id: i64, album_id: i64) {
    sqlx::query("UPDATE track SET album_id = $2 WHERE id = $1")
        .bind(track_id)
        .bind(album_id)
        .execute(pool)
        .await
        .expect("set track.album_id");
}

async fn insert_track_artist(
    pool: &PgPool,
    track_id: i64,
    artist_id: i64,
    library_id: i64,
    position: i32,
) {
    sqlx::query(
        "INSERT INTO track_artist (track_id, artist_id, library_id, position)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(track_id)
    .bind(artist_id)
    .bind(library_id)
    .bind(position)
    .execute(pool)
    .await
    .expect("insert track_artist");
}

// ──────────────────────────────────────────────────────────────────
// list_artists
// ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_artists_empty_returns_200_empty(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/artists",
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
async fn list_artists_orders_by_updated_at_desc(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let old = insert_artist(&pool, library_id, "Old Artist", BASE_NOW_MS).await;
    let mid = insert_artist(&pool, library_id, "Mid Artist", BASE_NOW_MS + 1_000).await;
    let new = insert_artist(&pool, library_id, "New Artist", BASE_NOW_MS + 2_000).await;

    let body: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/artists",
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
    assert_eq!(ids, vec![new, mid, old]);
    // picture_hash defaults to NULL until the server-side
    // artist-picture pipeline ships.
    assert!(body[0]["picture_hash"].is_null());
}

/// Tied `updated_at` rows must come back in `id ASC` order — the
/// apply pipeline upserts a whole sync round in one transaction, so
/// several artists can land at the same epoch millisecond. Without
/// the tiebreaker the ordering shuffles between requests and the
/// web client's list keeps re-keying its virtual scroller.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_artists_id_ascending_breaks_updated_at_ties(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let second = insert_artist(&pool, library_id, "Beta", BASE_NOW_MS).await;
    let first = insert_artist(&pool, library_id, "Alpha", BASE_NOW_MS).await;
    let third = insert_artist(&pool, library_id, "Gamma", BASE_NOW_MS).await;

    let body: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/artists",
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
    assert_eq!(ids, vec![second, first, third]);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_artists_foreign_library_returns_404(pool: PgPool) {
    let two = spawn_two_authenticated(pool.clone(), "a", "b").await;
    let base = two.base.clone();
    let profile_a = mint_profile(&base, &two.a.token, "A").await;
    let library_a = mint_library(&base, &two.a.token, profile_a, "A's lib").await;
    let profile_b = mint_profile(&base, &two.b.token, "B").await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_b}/libraries/{library_a}/artists",
        ))
        .bearer_auth(&two.b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// ──────────────────────────────────────────────────────────────────
// list_artist_tracks (drill-down)
// ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_artist_tracks_returns_every_contributor_link(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let solo = insert_artist(&pool, library_id, "Solo", BASE_NOW_MS).await;
    let collab = insert_artist(&pool, library_id, "Collab", BASE_NOW_MS).await;

    let track_a = insert_track(&pool, library_id, "/a.flac", "A", Some(1), Some(1)).await;
    let track_duet = insert_track(&pool, library_id, "/b.flac", "B", Some(1), Some(2)).await;
    let track_solo_only = insert_track(&pool, library_id, "/c.flac", "C", Some(1), Some(3)).await;
    // Track A: only solo. Track B: solo + collab. Track C: only solo.
    insert_track_artist(&pool, track_a, solo, library_id, 0).await;
    insert_track_artist(&pool, track_duet, solo, library_id, 0).await;
    insert_track_artist(&pool, track_duet, collab, library_id, 1).await;
    insert_track_artist(&pool, track_solo_only, solo, library_id, 0).await;

    // Solo gets all three tracks.
    let solo_tracks: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/artists/{solo}/tracks",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let solo_ids: Vec<i64> = solo_tracks
        .iter()
        .map(|v| v["id"].as_i64().unwrap())
        .collect();
    assert_eq!(solo_ids, vec![track_a, track_duet, track_solo_only]);

    // Collab gets just the duet — same track surfaces under both
    // contributors.
    let collab_tracks: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/artists/{collab}/tracks",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let collab_ids: Vec<i64> = collab_tracks
        .iter()
        .map(|v| v["id"].as_i64().unwrap())
        .collect();
    assert_eq!(collab_ids, vec![track_duet]);
}

/// Artist drill-down surfaces `album_id` so the web client can
/// deep-link a contributed track to its album page without an
/// extra `/tracks/{id}` round-trip (Phase 4.d.0.4 wire shape).
/// Mixes album-linked + orphan tracks so the `Option<i64>`
/// projection round-trips both states.
#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_artist_tracks_surfaces_album_id_when_linked(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let artist_id = insert_artist(&pool, library_id, "Tycho", BASE_NOW_MS).await;
    let album_id = insert_album(&pool, library_id, "Awake", Some(artist_id)).await;

    // Track 1: linked to the album. Track 2: orphan (no album).
    let linked = insert_track(&pool, library_id, "/a.flac", "A", Some(1), Some(1)).await;
    let orphan = insert_track(&pool, library_id, "/b.flac", "B", Some(1), Some(2)).await;
    set_track_album(&pool, linked, album_id).await;
    insert_track_artist(&pool, linked, artist_id, library_id, 0).await;
    insert_track_artist(&pool, orphan, artist_id, library_id, 0).await;

    let body: Vec<Value> = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/artists/{artist_id}/tracks",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body.len(), 2);
    let by_id: std::collections::HashMap<i64, &Value> = body
        .iter()
        .map(|r| (r["id"].as_i64().unwrap(), r))
        .collect();
    assert_eq!(by_id[&linked]["album_id"].as_i64().unwrap(), album_id);
    assert!(
        by_id[&orphan]["album_id"].is_null(),
        "orphan track must surface album_id=null"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_artist_tracks_empty_artist_returns_200_empty(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "Lib").await;

    let artist_id = insert_artist(&pool, library_id, "Empty", BASE_NOW_MS).await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_id}/artists/{artist_id}/tracks",
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
async fn list_artist_tracks_foreign_user_returns_404(pool: PgPool) {
    let two = spawn_two_authenticated(pool.clone(), "a", "b").await;
    let base = two.base.clone();
    let profile_a = mint_profile(&base, &two.a.token, "A").await;
    let library_a = mint_library(&base, &two.a.token, profile_a, "A's lib").await;
    let artist_a = insert_artist(&pool, library_a, "Hidden", BASE_NOW_MS).await;
    let track_a = insert_track(&pool, library_a, "/h.flac", "H", None, None).await;
    insert_track_artist(&pool, track_a, artist_a, library_a, 0).await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_a}/libraries/{library_a}/artists/{artist_a}/tracks",
        ))
        .bearer_auth(&two.b.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn list_artist_tracks_wrong_library_id_returns_404(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "u").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_a = mint_library(&auth.base, &auth.token, profile_id, "A").await;
    let library_b = mint_library(&auth.base, &auth.token, profile_id, "B").await;
    let artist_a = insert_artist(&pool, library_a, "X", BASE_NOW_MS).await;

    let resp = reqwest::Client::new()
        .get(format!(
            "{base}/api/v1/profiles/{profile_id}/libraries/{library_b}/artists/{artist_a}/tracks",
            base = &auth.base
        ))
        .bearer_auth(&auth.token)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
