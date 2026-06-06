//! Integration tests for the background self-heal scanner
//! (Phase 1.i.1).
//!
//! The scanner is exercised via [`waveflow_server::artwork_jobs::run_once`]
//! — the public entry point that performs exactly one cycle without
//! waiting on the tokio interval. Production binaries spawn the
//! interval loop via `artwork_jobs::spawn`; we drive `run_once`
//! directly so the assertion isn't racing the clock.
//!
//! We don't hit the HTTP layer here — the scanner is a pure DB +
//! storage operation, and a partial cache is faster to synthesise
//! by hand than by deleting a row out from under a real upload.

mod support;

use std::io::Cursor;

use image::{ImageBuffer, ImageFormat, Rgb};
use sqlx::PgPool;
use waveflow_server::{artwork_jobs, storage::ArtworkStorage};

/// Reuse the test-only synthetic JPEG factory: a diagonal gradient
/// the image crate can decode (the 1.h.1 `FAKE_JPEG` byte string
/// would be rejected by the pipeline's decode call).
fn synth_jpeg(width: u32, height: u32) -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(width, height, |x, y| {
        let r = ((x * 255) / width.max(1)) as u8;
        let g = ((y * 255) / height.max(1)) as u8;
        Rgb([r, g, 128])
    });
    let mut buf = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Jpeg)
        .expect("encode synth jpeg");
    buf
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn run_once_repairs_a_partial_cache(pool: PgPool) {
    // Bootstrap: a tempdir-backed LocalFileSystem ArtworkStorage.
    let dir = tempfile::tempdir().unwrap();
    let storage = ArtworkStorage::local(dir.path()).expect("local storage");

    // Synthesise a "partial cache": a metadata_artwork row with the
    // parent bytes durably in object_store, but only ONE variant
    // row in `metadata_artwork_variant`. Same shape as a partial-
    // write incident left behind by a prior upload that crashed
    // between landing the parent and committing both variants.
    let source = synth_jpeg(800, 600);
    let parent_hash = blake3::hash(&source).to_hex().to_string();
    storage
        .put(&parent_hash, source.clone().into())
        .await
        .expect("seed parent bytes");
    sqlx::query("INSERT INTO metadata_artwork (hash, mime, byte_size) VALUES ($1, $2, $3)")
        .bind(&parent_hash)
        .bind("image/jpeg")
        .bind(source.len() as i64)
        .execute(&pool)
        .await
        .expect("seed parent row");
    // One variant only — simulate the lost-write.
    sqlx::query(
        "INSERT INTO metadata_artwork_variant
              (parent_hash, variant, hash, mime, byte_size, width, height)
         VALUES ($1, 'preview', $2, 'image/jpeg', $3, $4, $5)",
    )
    .bind(&parent_hash)
    .bind("0".repeat(64)) // placeholder hash; the scanner doesn't read it back
    .bind(1024i64)
    .bind(480i32)
    .bind(270i32)
    .execute(&pool)
    .await
    .expect("seed partial variant");

    // Pre-condition: only one variant row for this parent.
    let pre: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM metadata_artwork_variant WHERE parent_hash = $1",
    )
    .bind(&parent_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pre, 1, "test must start from a partial state");

    // Drive one scan cycle. The scanner finds the parent in
    // `list_partial_parents` (variant count < 2), fetches the
    // bytes, re-runs the pipeline, and inserts the missing
    // `thumb` variant.
    let repaired = artwork_jobs::run_once(&pool, &storage, 50)
        .await
        .expect("scan cycle should not surface a top-level error");
    assert_eq!(repaired, 1, "scanner should report one parent repaired");

    // Post-condition: both variants present, with a `thumb` row
    // that didn't exist before.
    let post: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM metadata_artwork_variant WHERE parent_hash = $1",
    )
    .bind(&parent_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(post, 2, "scanner must fill the missing variant");

    let thumb_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM metadata_artwork_variant
          WHERE parent_hash = $1 AND variant = $2",
    )
    .bind(&parent_hash)
    .bind("thumb")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(thumb_count, 1, "`thumb` variant must have been inserted");
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn run_once_repairs_parent_with_no_variants(pool: PgPool) {
    // Edge case requested in CR: the parent row + parent bytes
    // both landed, but every variant insert was lost (e.g. the
    // upload returned 500 right after `storage.put` but before any
    // variant write). The scanner should treat "zero variants" the
    // same way it treats "one variant" and regenerate the full set.
    let dir = tempfile::tempdir().unwrap();
    let storage = ArtworkStorage::local(dir.path()).expect("local storage");

    let source = synth_jpeg(800, 600);
    let parent_hash = blake3::hash(&source).to_hex().to_string();
    storage
        .put(&parent_hash, source.clone().into())
        .await
        .expect("seed parent bytes");
    sqlx::query("INSERT INTO metadata_artwork (hash, mime, byte_size) VALUES ($1, $2, $3)")
        .bind(&parent_hash)
        .bind("image/jpeg")
        .bind(source.len() as i64)
        .execute(&pool)
        .await
        .expect("seed parent row");

    // Confirm we're starting from zero variant rows for this hash.
    let pre: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM metadata_artwork_variant WHERE parent_hash = $1",
    )
    .bind(&parent_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pre, 0, "test must start with no variant rows");

    let repaired = artwork_jobs::run_once(&pool, &storage, 50)
        .await
        .expect("scan cycle should not surface a top-level error");
    assert_eq!(repaired, 1, "scanner should report one parent repaired");

    let post: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)::BIGINT FROM metadata_artwork_variant WHERE parent_hash = $1",
    )
    .bind(&parent_hash)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(post, 2, "scanner must fill both variants from scratch");

    // Spot-check that each canonical variant name landed (`thumb`
    // + `preview` ordering is verified elsewhere; here we just need
    // the set to be complete).
    for variant in ["thumb", "preview"] {
        let n: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM metadata_artwork_variant
              WHERE parent_hash = $1 AND variant = $2",
        )
        .bind(&parent_hash)
        .bind(variant)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(n, 1, "variant {variant} must have been inserted");
    }
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn run_once_is_a_noop_when_cache_is_complete(pool: PgPool) {
    // No partial caches in the DB → the scanner should report zero
    // repairs and complete without touching storage.
    let dir = tempfile::tempdir().unwrap();
    let storage = ArtworkStorage::local(dir.path()).expect("local storage");

    let repaired = artwork_jobs::run_once(&pool, &storage, 50)
        .await
        .expect("empty cycle should succeed");
    assert_eq!(repaired, 0);
}

#[sqlx::test(migrator = "waveflow_server::db::MIGRATOR")]
async fn run_once_skips_parents_with_missing_bytes(pool: PgPool) {
    // A partial cache where the parent ROW exists but the parent
    // bytes were lost from object_store too. The scanner can't
    // regenerate variants without the source — we expect it to log +
    // count the failure but NOT abort the cycle, returning `0`
    // repaired and leaving the broken parent for the next pass.
    //
    // Also asserts the starvation guard: the failed repair stamps
    // `last_repair_failure_at`, and a second cycle immediately after
    // the first must NOT see this hash in `list_partial_parents`
    // anymore (still inside the 1-hour backoff window). Without the
    // guard, an irrecoverable parent would dominate every cycle's
    // batch and starve recoverable parents behind it.
    let dir = tempfile::tempdir().unwrap();
    let storage = ArtworkStorage::local(dir.path()).expect("local storage");

    let phantom_hash = "f".repeat(64);
    sqlx::query("INSERT INTO metadata_artwork (hash, mime, byte_size) VALUES ($1, $2, $3)")
        .bind(&phantom_hash)
        .bind("image/jpeg")
        .bind(1024i64)
        .execute(&pool)
        .await
        .unwrap();

    let repaired = artwork_jobs::run_once(&pool, &storage, 50)
        .await
        .expect("missing-bytes failure should be swallowed inside the cycle");
    assert_eq!(repaired, 0, "broken parent contributes 0 to the count");

    // Stamp must have landed (epoch-millis BIGINT — schema
    // convention every other timestamp column on this server uses).
    let stamped: Option<i64> =
        sqlx::query_scalar("SELECT last_repair_failure_at FROM metadata_artwork WHERE hash = $1")
            .bind(&phantom_hash)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        stamped.is_some(),
        "failed repair must stamp last_repair_failure_at",
    );

    // A second cycle right after the first must NOT pick the row up
    // again — the backoff window keeps it out of the candidate set.
    let cutoff_ms = chrono::Utc::now().timestamp_millis() - 3_600_000;
    let recheck = waveflow_server::db::artwork::list_partial_parents(&pool, 2, 50, cutoff_ms)
        .await
        .unwrap();
    assert!(
        !recheck.iter().any(|h| h == &phantom_hash),
        "backoff must hide the freshly-failed parent from the next cycle",
    );
}
