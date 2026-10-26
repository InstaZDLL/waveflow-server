//! What a scan reads off the files, and what it does with it.
//!
//! Split out of `v2_foundations.rs`.

use sqlx::Row;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::storage::catalog::ApplyOutcome;
use waveflow_server::storage::catalog::LibraryRecord;
use waveflow_server::storage::database::AccountRole;
use waveflow_server::storage::database::LibraryVisibility;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod artwork_sweep;
mod exclusions;
mod incremental;
mod indexing;
mod lifecycle;
mod purges;
mod scheduling;

// ── An incremental scan that reads only what changed (RFC-018) ──
/// A library of one owner's, holding the files `files` writes into its root.
async fn incremental_library(
    name: &str,
    files: impl FnOnce(&std::path::Path),
) -> (
    tempfile::TempDir,
    waveflow_server::AppState,
    uuid::Uuid,
    LibraryRecord,
    std::path::PathBuf,
) {
    let (temp, config, state) = test_app().await;
    // Inserted rather than created: a scan needs an owner, not a login.
    let owner = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(owner.to_string())
    .bind(name)
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    files(&music);
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: name.into(),
        root_path: root.clone(),
    };
    (temp, state, owner, library, root)
}

async fn job(
    state: &waveflow_server::AppState,
    owner: uuid::Uuid,
    scan: uuid::Uuid,
) -> waveflow_server::storage::catalog::ScanJobRecord {
    state
        .db
        .scan_job_for_user(owner, scan)
        .await
        .unwrap()
        .unwrap()
}

async fn full_hash(state: &waveflow_server::AppState) -> Option<String> {
    sqlx::query_scalar("SELECT full_hash FROM track")
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

// ── Libraries and scanning: exclusions, links, missing tracks (RFC-021) ──
/// An account inserted straight into the table: these tests need an owner, a
/// manager or a listener, never a login.
async fn insert_account(state: &waveflow_server::AppState, name: &str, role: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', ?, 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(role)
    .bind(now_ms())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    id
}

async fn count(state: &waveflow_server::AppState, query: &'static str) -> i64 {
    sqlx::query_scalar(query)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

async fn available_paths(state: &waveflow_server::AppState) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT relative_path FROM track WHERE is_available = 1 ORDER BY relative_path",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap()
}

/// Waits for one job to end, whichever way.
async fn settled(
    state: &waveflow_server::AppState,
    owner: uuid::Uuid,
    scan: uuid::Uuid,
) -> waveflow_server::storage::catalog::ScanJobRecord {
    for _ in 0..500 {
        let record = job(state, owner, scan).await;
        if record.status == "completed" || record.status == "failed" {
            return record;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("scan timed out");
}

/// A FLAC file tagged with a title, an album and an artist, its tone chosen by
/// `frequency` so no two fixtures share their bytes.
fn write_tagged_flac(path: &std::path::Path, title: &str, album: &str, frequency: usize) {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency={frequency}:duration=0.1"),
            "-metadata",
            &format!("title={title}"),
            "-metadata",
            &format!("album={album}"),
            "-metadata",
            "artist=Lifecycle Artist",
            "-c:a",
            "flac",
        ])
        .arg(path)
        .output()
        .expect("FFmpeg is required for tagged fixtures");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Two tracks in an album, one in another, scanned.
async fn library_with_albums(
    name: &str,
) -> (
    tempfile::TempDir,
    waveflow_server::AppState,
    uuid::Uuid,
    LibraryRecord,
    std::path::PathBuf,
) {
    let fixture = incremental_library(name, |music| {
        for (index, album) in ["First", "First", "Second"].into_iter().enumerate() {
            std::fs::create_dir_all(music.join(album)).unwrap();
            write_tagged_flac(
                &music.join(format!("{album}/{index}.flac")),
                &format!("Track {index}"),
                album,
                300 + index * 110,
            );
        }
    })
    .await;
    scan_once(&fixture.1, fixture.2, fixture.3.clone()).await;
    fixture
}

async fn track_id(state: &waveflow_server::AppState, path: &str) -> String {
    sqlx::query_scalar("SELECT id FROM track WHERE relative_path = ?")
        .bind(path)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}
