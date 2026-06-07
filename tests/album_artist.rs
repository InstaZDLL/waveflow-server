//! Schema-level invariants for `album` + `artist` + `track_artist`
//! and the new `track.album_id` FK (phase 4.d.0.1).
//!
//! This PR ships the schema only — no apply pipeline, no REST
//! endpoints yet. The tests below lock the invariants the apply
//! pipeline (4.d.0.2) and the read endpoints (4.d.0.4) depend on,
//! so a future migration that loosens a cascade or drops a UNIQUE
//! trips here before it sneaks into a release.
//!
//! Direct SQL inserts on purpose — there's no repository layer for
//! these entities yet, and using one would couple this test to
//! whatever apply-side helper lands in 4.d.0.2. Keeping the tests
//! at the SQL boundary means the contract under test is the
//! migration itself.
//!
//! Profile + library are minted through the REST API (via the
//! shared `spawn_authenticated` harness) rather than direct SQL —
//! that's a heavier setup than the schema-invariant scope strictly
//! needs, but every existing integration test in this repo follows
//! the same shape, and `library.profile_id` ultimately chains
//! through `users::find_or_provision_by_external_id` which lives
//! behind the JWT path. Bypassing it would couple this test to
//! private DB helpers and break the harness symmetry future
//! reviewers expect.

mod support;

use serde_json::{json, Value};
use sqlx::PgPool;
use support::spawn_authenticated;

/// Fixed sentinel for `created_at` / `updated_at` inserts. These
/// tests don't assert on time, so freezing the value keeps the
/// assertions stable AND removes the temptation to reach for
/// `Date::now()`-style nondeterminism. Mid-November 2023 if anyone
/// reads the milliseconds.
const FIXED_NOW_MS: i64 = 1_700_000_000_000;

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

/// Insert an artist row directly. Returns the new id. `now_ms` is
/// a fixed sentinel — these tests don't assert on time so a
/// constant keeps the assertions stable without leaking
/// `Date.now()`-style nondeterminism through the suite.
async fn insert_artist(pool: &PgPool, library_id: i64, name: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO artist (library_id, name, created_at, updated_at)
         VALUES ($1, $2, $3, $3) RETURNING id",
    )
    .bind(library_id)
    .bind(name)
    .bind(FIXED_NOW_MS)
    .fetch_one(pool)
    .await
    .expect("insert artist")
}

/// Insert an album row directly. `album_artist_id` is `Option<i64>`
/// so the compilation path (`NULL`) is testable.
async fn insert_album(
    pool: &PgPool,
    library_id: i64,
    canonical_title: &str,
    album_artist_id: Option<i64>,
    is_compilation: bool,
) -> Result<i64, sqlx::Error> {
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
    .bind(FIXED_NOW_MS)
    .fetch_one(pool)
    .await
}

/// Insert a track row directly. `album_id` is `Option<i64>` so
/// orphan tracks (no album) are testable.
async fn insert_track(
    pool: &PgPool,
    library_id: i64,
    file_path: &str,
    title: &str,
    album_id: Option<i64>,
) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "INSERT INTO track
            (library_id, file_path, file_size, title, duration_ms,
             album_id, added_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
    )
    .bind(library_id)
    .bind(file_path)
    .bind(1024_i64)
    .bind(title)
    .bind(180_000_i64)
    .bind(album_id)
    .bind(FIXED_NOW_MS)
    .fetch_one(pool)
    .await
    .expect("insert track")
}

// ────────────────────────────────────────────────────────────────
// Album natural key: (library_id, canonical_title, album_artist_id)
// with NULLS NOT DISTINCT for the compilation collapse.
// ────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn album_natural_key_collapses_compilation_with_null_artist(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    // First compilation row with NULL album_artist_id — must succeed.
    let first =
        insert_album(&pool, library_id, "Now That's What I Call Music", None, true).await;
    assert!(
        first.is_ok(),
        "first compilation insert must succeed (got {first:?})"
    );

    // Second insert with the SAME (library_id, canonical_title) AND
    // NULL album_artist_id MUST fail — NULLS NOT DISTINCT collapses
    // the two NULL keys, the natural-key UNIQUE catches the duplicate.
    let second =
        insert_album(&pool, library_id, "Now That's What I Call Music", None, true).await;
    let Err(err) = second else {
        panic!("duplicate compilation insert must violate UNIQUE NULLS NOT DISTINCT, got Ok");
    };
    let err_str = err.to_string();
    assert!(
        err_str.contains("duplicate key") || err_str.contains("unique"),
        "expected unique-violation error, got: {err_str}"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn album_natural_key_allows_same_title_under_different_artists(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let artist_a = insert_artist(&pool, library_id, "Artist A").await;
    let artist_b = insert_artist(&pool, library_id, "Artist B").await;

    // Two albums titled "Greatest Hits" under different artists
    // are TWO different albums — the natural key includes
    // album_artist_id, so this is NOT a duplicate.
    let a = insert_album(&pool, library_id, "Greatest Hits", Some(artist_a), false).await;
    let b = insert_album(&pool, library_id, "Greatest Hits", Some(artist_b), false).await;
    assert!(a.is_ok(), "first 'Greatest Hits' insert failed: {a:?}");
    assert!(
        b.is_ok(),
        "same-title-different-artist insert must succeed: {b:?}"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn album_natural_key_separates_compilation_from_attributed(pool: PgPool) {
    // Edge case worth pinning: a library can have BOTH a
    // compilation row (album_artist_id = NULL) AND an
    // artist-attributed row with the same title. They're distinct
    // entities — "Greatest Hits" the various-artists comp vs
    // "Greatest Hits" by some specific artist. The natural key
    // separates them on album_artist_id, so the un-equal
    // attributed insert MUST succeed.
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let comp = insert_album(&pool, library_id, "Greatest Hits", None, true).await;
    assert!(comp.is_ok(), "compilation insert failed: {comp:?}");

    let artist = insert_artist(&pool, library_id, "Specific Artist").await;
    let attributed =
        insert_album(&pool, library_id, "Greatest Hits", Some(artist), false).await;
    assert!(
        attributed.is_ok(),
        "compilation + attributed row with same title MUST coexist: {attributed:?}",
    );
}

// ────────────────────────────────────────────────────────────────
// Artist uniqueness: (library_id, name).
// ────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn artist_unique_per_library_name(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_a = mint_library(&auth.base, &auth.token, profile_id, "lib-A").await;
    let library_b = mint_library(&auth.base, &auth.token, profile_id, "lib-B").await;

    insert_artist(&pool, library_a, "Daft Punk").await;

    // Same name in same library → conflict.
    let dup = sqlx::query(
        "INSERT INTO artist (library_id, name, created_at, updated_at)
         VALUES ($1, $2, $3, $3)",
    )
    .bind(library_a)
    .bind("Daft Punk")
    .bind(FIXED_NOW_MS)
    .execute(&pool)
    .await;
    assert!(
        dup.is_err(),
        "duplicate artist within a library must conflict"
    );

    // Same name in a different library → allowed (per-library
    // scope, matching `track.library_id`).
    let cross = sqlx::query(
        "INSERT INTO artist (library_id, name, created_at, updated_at)
         VALUES ($1, $2, $3, $3)",
    )
    .bind(library_b)
    .bind("Daft Punk")
    .bind(FIXED_NOW_MS)
    .execute(&pool)
    .await;
    assert!(
        cross.is_ok(),
        "same artist name in a different library must be allowed (per-library scope): {cross:?}",
    );
}

// ────────────────────────────────────────────────────────────────
// Cascade behaviour. The CHAIN is library → album / artist /
// track / track_artist. Deleting a library must reclaim every
// dependent row in one tx.
// ────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn library_delete_cascades_to_album_artist_and_track_artist(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-doomed").await;

    let artist_id = insert_artist(&pool, library_id, "Daft Punk").await;
    let album_id = insert_album(&pool, library_id, "Discovery", Some(artist_id), false)
        .await
        .expect("album insert");
    let track_id = insert_track(&pool, library_id, "/music/one.mp3", "One More Time", Some(album_id))
        .await;
    sqlx::query(
        "INSERT INTO track_artist (track_id, artist_id, position) VALUES ($1, $2, 0)",
    )
    .bind(track_id)
    .bind(artist_id)
    .execute(&pool)
    .await
    .expect("track_artist insert");

    // Delete the library. ON DELETE CASCADE on track.library_id +
    // album.library_id + artist.library_id MUST reclaim everything.
    sqlx::query("DELETE FROM library WHERE id = $1")
        .bind(library_id)
        .execute(&pool)
        .await
        .expect("library delete");

    let counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM album WHERE id = $1),
            (SELECT COUNT(*) FROM artist WHERE id = $2),
            (SELECT COUNT(*) FROM track WHERE id = $3),
            (SELECT COUNT(*) FROM track_artist WHERE track_id = $3 OR artist_id = $2)",
    )
    .bind(album_id)
    .bind(artist_id)
    .bind(track_id)
    .fetch_one(&pool)
    .await
    .expect("post-delete counts");
    assert_eq!(counts, (0, 0, 0, 0), "library delete left orphans");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_album_id_goes_null_on_album_delete(pool: PgPool) {
    // The album → track FK is `SET NULL` (not CASCADE) on purpose:
    // a stray album scrub shouldn't take the audio files with it.
    // The tracks survive as orphans, discoverable via
    // `WHERE album_id IS NULL`.
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let artist_id = insert_artist(&pool, library_id, "Some Artist").await;
    let album_id = insert_album(&pool, library_id, "Some Album", Some(artist_id), false)
        .await
        .expect("album insert");
    let track_id =
        insert_track(&pool, library_id, "/music/a.mp3", "Some Track", Some(album_id)).await;

    sqlx::query("DELETE FROM album WHERE id = $1")
        .bind(album_id)
        .execute(&pool)
        .await
        .expect("album delete");

    let (album_count, track_album_id): (i64, Option<i64>) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM album WHERE id = $1),
            (SELECT album_id FROM track WHERE id = $2)",
    )
    .bind(album_id)
    .bind(track_id)
    .fetch_one(&pool)
    .await
    .expect("post-delete state");
    assert_eq!(album_count, 0, "album row must be gone");
    assert!(
        track_album_id.is_none(),
        "track.album_id must be NULL after album delete (got {track_album_id:?})"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn album_album_artist_id_goes_null_on_artist_delete(pool: PgPool) {
    // Similar to the track ↔ album rule: when the album artist
    // is scrubbed, the album row survives with NULL
    // `album_artist_id`. The next sync re-establishes the link
    // when the artist re-appears.
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let artist_id = insert_artist(&pool, library_id, "Doomed Artist").await;
    let album_id = insert_album(&pool, library_id, "Their Album", Some(artist_id), false)
        .await
        .expect("album insert");

    sqlx::query("DELETE FROM artist WHERE id = $1")
        .bind(artist_id)
        .execute(&pool)
        .await
        .expect("artist delete");

    let (artist_count, album_artist_id): (i64, Option<i64>) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM artist WHERE id = $1),
            (SELECT album_artist_id FROM album WHERE id = $2)",
    )
    .bind(artist_id)
    .bind(album_id)
    .fetch_one(&pool)
    .await
    .expect("post-delete state");
    assert_eq!(artist_count, 0, "artist row must be gone");
    assert!(
        album_artist_id.is_none(),
        "album.album_artist_id must be NULL after artist delete (got {album_artist_id:?})"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_delete_cascades_to_track_artist(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let artist_id = insert_artist(&pool, library_id, "Solo Artist").await;
    let track_id = insert_track(&pool, library_id, "/music/x.mp3", "X", None).await;
    sqlx::query(
        "INSERT INTO track_artist (track_id, artist_id, position) VALUES ($1, $2, 0)",
    )
    .bind(track_id)
    .bind(artist_id)
    .execute(&pool)
    .await
    .expect("track_artist insert");

    sqlx::query("DELETE FROM track WHERE id = $1")
        .bind(track_id)
        .execute(&pool)
        .await
        .expect("track delete");

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM track_artist WHERE track_id = $1")
            .bind(track_id)
            .fetch_one(&pool)
            .await
            .expect("track_artist count");
    assert_eq!(count, 0, "track delete must cascade into track_artist");
}

// ────────────────────────────────────────────────────────────────
// Multi-artist join semantics.
// ────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_artist_pk_prevents_duplicate_pairing(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let artist_id = insert_artist(&pool, library_id, "Solo Artist").await;
    let track_id = insert_track(&pool, library_id, "/music/y.mp3", "Y", None).await;

    sqlx::query(
        "INSERT INTO track_artist (track_id, artist_id, position) VALUES ($1, $2, 0)",
    )
    .bind(track_id)
    .bind(artist_id)
    .execute(&pool)
    .await
    .expect("first track_artist insert");

    let dup = sqlx::query(
        "INSERT INTO track_artist (track_id, artist_id, position) VALUES ($1, $2, 1)",
    )
    .bind(track_id)
    .bind(artist_id)
    .execute(&pool)
    .await;
    assert!(
        dup.is_err(),
        "duplicate (track_id, artist_id) must violate the PK"
    );
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_artist_position_preserves_multi_artist_order(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let track_id = insert_track(&pool, library_id, "/music/z.mp3", "Z", None).await;
    let a = insert_artist(&pool, library_id, "Primary").await;
    let b = insert_artist(&pool, library_id, "Featured").await;
    let c = insert_artist(&pool, library_id, "Producer").await;

    // Insert in REVERSE order to prove SELECT ORDER BY position
    // surfaces them in the right one — without the ORDER BY the
    // SELECT would return them in insertion order and the
    // assertion would still pass.
    for (artist_id, position) in [(c, 2_i32), (b, 1), (a, 0)] {
        sqlx::query(
            "INSERT INTO track_artist (track_id, artist_id, position) VALUES ($1, $2, $3)",
        )
        .bind(track_id)
        .bind(artist_id)
        .bind(position)
        .execute(&pool)
        .await
        .expect("track_artist insert");
    }

    let rows: Vec<(i64, i32)> = sqlx::query_as(
        "SELECT artist_id, position FROM track_artist
          WHERE track_id = $1 ORDER BY position ASC",
    )
    .bind(track_id)
    .fetch_all(&pool)
    .await
    .expect("track_artist select");
    assert_eq!(rows, vec![(a, 0), (b, 1), (c, 2)]);
}

// ────────────────────────────────────────────────────────────────
// CHECK constraints.
// ────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn artist_rejects_empty_name(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let res = sqlx::query(
        "INSERT INTO artist (library_id, name, created_at, updated_at)
         VALUES ($1, '', $2, $2)",
    )
    .bind(library_id)
    .bind(FIXED_NOW_MS)
    .execute(&pool)
    .await;
    assert!(res.is_err(), "empty artist name must fail the CHECK");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn album_rejects_empty_canonical_title(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let res = insert_album(&pool, library_id, "", None, false).await;
    assert!(res.is_err(), "empty canonical_title must fail the CHECK");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn track_artist_rejects_negative_position(pool: PgPool) {
    let auth = spawn_authenticated(pool.clone(), "test-user").await;
    let profile_id = mint_profile(&auth.base, &auth.token, "Alice").await;
    let library_id = mint_library(&auth.base, &auth.token, profile_id, "lib-1").await;

    let track_id = insert_track(&pool, library_id, "/music/n.mp3", "Neg", None).await;
    let artist_id = insert_artist(&pool, library_id, "Neg Artist").await;

    let res = sqlx::query(
        "INSERT INTO track_artist (track_id, artist_id, position) VALUES ($1, $2, -1)",
    )
    .bind(track_id)
    .bind(artist_id)
    .execute(&pool)
    .await;
    assert!(res.is_err(), "negative position must fail the CHECK");
}
