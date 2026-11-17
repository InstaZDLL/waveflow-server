//! Covers stored as `bin` before `waveflow-core` v2.1.0.
//!
//! The core typed a picture by its declared MIME type, and lofty has none for
//! WebP: a WebP cover was written as `<hash>.bin` under a row saying `bin`,
//! which no route could serve. The core now reads the bytes, but a row that
//! exists is never written again. These tests build that old store by hand —
//! a scan, then the row and the file put back the way the old core left them —
//! and hold the repair `retype_bin_artwork` makes of it.

use super::artwork_sweep::age_beyond_the_grace;
use super::*;

/// A 1×1 lossless WebP, whole: `detect_image_format` reads the `RIFF`/`WEBP`
/// header, and the route serves the bytes as they are.
const WEBP: &[u8] = &[
    0x52, 0x49, 0x46, 0x46, 0x1a, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38, 0x4c,
    0x0d, 0x00, 0x00, 0x00, 0x2f, 0x00, 0x00, 0x00, 0x10, 0x07, 0x10, 0x11, 0x11, 0x88, 0x88, 0xfe,
    0x07, 0x00,
];

/// A scanned library whose one track takes a WebP folder cover, and the hash
/// of that cover. The hash comes from the scan, never from this test.
async fn scanned_webp_cover(name: &str) -> (tempfile::TempDir, waveflow_server::AppState, String) {
    let (temp, state, owner, library, _root) = incremental_library(name, |music| {
        write_test_wav(&music.join("One.wav"));
        std::fs::write(music.join("cover.webp"), WEBP).unwrap();
    })
    .await;
    run_scan(&state, owner, library).await;
    let (hash, format): (String, String) = sqlx::query_as("SELECT hash, format FROM artwork")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(format, "webp", "the scan stores a WebP as one");
    (temp, state, hash)
}

/// Puts the row back the way the old core wrote it.
async fn row_says_bin(state: &waveflow_server::AppState, hash: &str) {
    sqlx::query("UPDATE artwork SET format = 'bin' WHERE hash = ?")
        .bind(hash)
        .execute(state.db.pool())
        .await
        .unwrap();
}

async fn stored_format(state: &waveflow_server::AppState, hash: &str) -> String {
    sqlx::query_scalar("SELECT format FROM artwork WHERE hash = ?")
        .bind(hash)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_webp_stored_as_bin_is_served_once_retyped() {
    let (_temp, state, hash) = scanned_webp_cover("retype-bin").await;
    let dir = &state.artwork_dir;
    let webp = dir.join(format!("{hash}.webp"));
    let bin = dir.join(format!("{hash}.bin"));
    std::fs::rename(&webp, &bin).unwrap();
    row_says_bin(&state, &hash).await;

    // The defect: the format the row names has no MIME type, so the route
    // that serves covers finds nothing to answer with.
    assert!(waveflow_server::media::read_artwork(dir, &hash, "bin")
        .await
        .is_none());

    assert_eq!(state.services.retype_bin_artwork().await.unwrap(), 1);
    assert_eq!(stored_format(&state, &hash).await, "webp");
    assert!(webp.exists(), "the bytes are under their real name");
    assert!(!bin.exists(), "renamed, not copied: nothing is left behind");
    let (mime, bytes) = waveflow_server::media::read_artwork(dir, &hash, "webp")
        .await
        .expect("the cover is served");
    assert_eq!(mime, "image/webp");
    assert_eq!(bytes, WEBP);

    // Nothing says `bin` any more, so the next boot's pass has nothing to do.
    assert_eq!(state.services.retype_bin_artwork().await.unwrap(), 0);
}

/// A scan under the new core writes the same bytes under their real name and
/// leaves the old row alone: both files are there, and the row names the
/// `bin`. The repair keeps the typed one and removes the other.
#[tokio::test]
async fn a_bin_beside_its_typed_copy_goes_once_the_row_follows() {
    let (_temp, state, hash) = scanned_webp_cover("retype-leftover").await;
    let dir = &state.artwork_dir;
    let webp = dir.join(format!("{hash}.webp"));
    let bin = dir.join(format!("{hash}.bin"));
    std::fs::copy(&webp, &bin).unwrap();
    row_says_bin(&state, &hash).await;

    assert_eq!(state.services.retype_bin_artwork().await.unwrap(), 1);
    assert_eq!(stored_format(&state, &hash).await, "webp");
    assert!(webp.exists());
    assert!(!bin.exists(), "the leftover goes with the repair");
}

/// A typed file cut short is no copy of the cover: the `bin` is its only
/// whole copy, so it takes the typed name rather than being removed.
#[tokio::test]
async fn a_typed_file_cut_short_is_replaced_by_the_bin() {
    let (_temp, state, hash) = scanned_webp_cover("retype-truncated").await;
    let dir = &state.artwork_dir;
    let webp = dir.join(format!("{hash}.webp"));
    let bin = dir.join(format!("{hash}.bin"));
    std::fs::rename(&webp, &bin).unwrap();
    std::fs::write(&webp, &WEBP[..12]).unwrap();
    row_says_bin(&state, &hash).await;

    assert_eq!(state.services.retype_bin_artwork().await.unwrap(), 1);
    assert_eq!(stored_format(&state, &hash).await, "webp");
    assert_eq!(std::fs::read(&webp).unwrap(), WEBP, "the whole cover");
    assert!(!bin.exists());
}

/// A `bin` the repair could not remove — the process stopped after the row
/// followed, or the removal failed — is collected by the sweep, which
/// recognises the name. One whose row still says `bin` is the cover itself.
#[tokio::test]
async fn the_sweep_collects_a_bin_its_row_no_longer_names() {
    let (_temp, state, hash) = scanned_webp_cover("retype-sweep").await;
    let dir = &state.artwork_dir;
    let bin = dir.join(format!("{hash}.bin"));
    std::fs::copy(dir.join(format!("{hash}.webp")), &bin).unwrap();
    age_beyond_the_grace(&bin);

    // The row still says `bin`: the file is the cover, and nothing is wrong.
    row_says_bin(&state, &hash).await;
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.covers_removed, 0);
    assert_eq!(swept.unknown, 0, "the sweep knows the name");
    assert_eq!(swept.dead_links, 0, "the row's file is there");
    assert!(bin.exists());

    // The row names the typed file: the `bin` is a leftover.
    sqlx::query("UPDATE artwork SET format = 'webp' WHERE hash = ?")
        .bind(&hash)
        .execute(state.db.pool())
        .await
        .unwrap();
    let swept = state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(swept.covers_removed, 1);
    assert!(!bin.exists());
    assert!(dir.join(format!("{hash}.webp")).exists());
}

/// A pass stopped between the rename and the row leaves a row saying `bin`
/// and no `bin` file. The typed file is adopted, since its bytes agree.
#[tokio::test]
async fn a_bin_already_renamed_is_adopted_by_the_next_pass() {
    let (_temp, state, hash) = scanned_webp_cover("retype-adopt").await;
    row_says_bin(&state, &hash).await;

    assert_eq!(state.services.retype_bin_artwork().await.unwrap(), 1);
    assert_eq!(stored_format(&state, &hash).await, "webp");
}

/// The pass never invents a format: a `bin` that is no image this server can
/// serve stays exactly as it was, row and file.
#[tokio::test]
async fn a_bin_that_is_no_servable_image_stays_as_it_is() {
    let (_temp, state, hash) = scanned_webp_cover("retype-unknown").await;
    let dir = &state.artwork_dir;
    let bin = dir.join(format!("{hash}.bin"));
    std::fs::remove_file(dir.join(format!("{hash}.webp"))).unwrap();
    std::fs::write(&bin, b"GIF89a, which this server does not serve").unwrap();
    row_says_bin(&state, &hash).await;

    assert_eq!(state.services.retype_bin_artwork().await.unwrap(), 0);
    assert_eq!(stored_format(&state, &hash).await, "bin");
    assert!(
        bin.exists(),
        "a file the pass cannot type is not its to move"
    );
}
