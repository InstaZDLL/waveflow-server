//! The loop a member attaches to a track. RFC-009.
//!
//! No routes yet: this drives `DomainServices` directly, which is the whole of
//! what the domain half of the canvas is. What the store does with bytes, what
//! it charges for them, and what it refuses are decided here rather than at the
//! surface, so they are tested here.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::auth::security;
use waveflow_server::config::CanvasLimits;
use waveflow_server::storage::catalog::LibraryRecord;
use waveflow_server::storage::database::AccountRole;
use waveflow_server::storage::database::LibraryVisibility;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

/// An app whose canvas limits this test chose.
///
/// Tuned before `initialize`, never after: `DomainServices` copies these values
/// out of `Config` when it is built, so a test that raises a ceiling on the
/// returned `Config` silently exercises the old one.
async fn canvas_app(
    tune: impl FnOnce(&mut CanvasLimits),
) -> (
    tempfile::TempDir,
    waveflow_server::Config,
    waveflow_server::AppState,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    tune(&mut config.canvas);
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

struct Fixture {
    owner: uuid::Uuid,
    library: uuid::Uuid,
    tracks: Vec<uuid::Uuid>,
    token: String,
}

/// An owner, an open library, and `count` scanned tracks in it.
async fn fixture(
    config: &waveflow_server::Config,
    state: &waveflow_server::AppState,
    name: &str,
    count: usize,
) -> Fixture {
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account(name, &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    for index in 0..count {
        write_test_wav(&music.join(format!("Track {index}.wav")));
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        state,
        owner,
        LibraryRecord {
            id: library,
            name: name.into(),
            root_path: root,
        },
    )
    .await;
    // A canvas spends the operator's disk, so it follows the deposit rules: a
    // server that was merely upgraded must not have become one. Its own flag,
    // not the upload one — the fixture leaves `accepts_uploads` off precisely
    // so that every test here runs on a library that takes no audio.
    state
        .db
        .set_library_accepts_canvas(owner, library, true, now_ms())
        .await
        .unwrap();
    let mut tracks: Vec<uuid::Uuid> = state
        .db
        .list_tracks_for_user(owner, library)
        .await
        .unwrap()
        .into_iter()
        .map(|track| track.id)
        .collect();
    tracks.sort();
    assert_eq!(tracks.len(), count);
    let token = login_token(
        &waveflow_server::app(config, state.clone()),
        name,
        "correct horse battery staple",
    )
    .await;
    Fixture {
        owner,
        library,
        tracks,
        token,
    }
}

/// A real container, because the server reads the bytes rather than the name.
///
/// `mpeg4` rather than `libx264` or `libvpx`: it is built into FFmpeg itself,
/// so this fixture does not depend on which encoders a machine happens to ship.
fn canvas_bytes(dir: &std::path::Path, name: &str, seconds: &str, colour: &str) -> Vec<u8> {
    let path = dir.join(name);
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            &format!("color=c={colour}:s=64x64:r=10:d={seconds}"),
            "-c:v",
            "mpeg4",
        ])
        .arg(&path)
        .output()
        .expect("ffmpeg must be on PATH");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::read(&path).unwrap()
}

fn stored_files(config: &waveflow_server::Config) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(&config.canvas_dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

mod ingest;
mod routes;
mod sweep;
