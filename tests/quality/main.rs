//! The real quality of a file, measured from its audio (RFC-016).
//!
//! Every file here is made by FFmpeg from noise, so its spectrum is known: a
//! full band, the wall LAME leaves at 128 kbps, the filter of a 44.1 kHz
//! source resampled to 96 kHz, a 16-bit signal in a 24-bit container. The
//! scanner indexes them for real, and the analysis measures them through the
//! same FFmpeg the server runs.
//!
//! The accounts are rows written by hand and reach the routes through an API
//! token, as in `tests/credits/`: nothing here hands a literal to a hash.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use http_body_util::BodyExt;
use support::*;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::services::{AnalysisState, AttentionCategory, BrowsePage, QualityVerdict};
use waveflow_server::storage::catalog::LibraryRecord;
use waveflow_server::storage::database::{LibraryRole, LibraryVisibility};
use waveflow_server::AppState;

struct Shelf {
    state: AppState,
    config: waveflow_server::Config,
    owner: Uuid,
    library: LibraryRecord,
    music: PathBuf,
    _temp: tempfile::TempDir,
}

async fn account(state: &AppState, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();
    id
}

async fn shelf(name: &str) -> Shelf {
    let (temp, config, state) = test_app().await;
    let owner = account(&state, name).await;
    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let id = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    Shelf {
        state,
        config,
        owner,
        library: LibraryRecord {
            id,
            name: name.into(),
            root_path: root,
        },
        music,
        _temp: temp,
    }
}

/// Runs FFmpeg with these arguments, failing the test on any error.
fn ffmpeg(args: &[&str]) {
    let output = std::process::Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y"])
        .args(args)
        .output()
        .expect("FFmpeg must be on PATH");
    assert!(
        output.status.success(),
        "ffmpeg {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// White noise: a full band, which no lossy encoder keeps.
fn noise(rate: u32, seconds: u32, seed: u32) -> String {
    format!("anoisesrc=d={seconds}:c=white:a=0.3:r={rate}:s={seed}")
}

/// A genuine 16-bit FLAC: the full band, and every bit it declares.
fn genuine(path: &Path, seed: u32) {
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 6, seed),
        "-ac",
        "2",
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// A FLAC made from a 128 kbps MP3 of the same noise.
fn from_mp3(path: &Path, scratch: &Path) {
    let mp3 = scratch.join(format!(
        "{}.mp3",
        path.file_stem().unwrap().to_string_lossy()
    ));
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 6, 7),
        "-ac",
        "2",
        "-c:a",
        "libmp3lame",
        "-b:a",
        "128k",
        mp3.to_str().unwrap(),
    ]);
    ffmpeg(&[
        "-i",
        mp3.to_str().unwrap(),
        "-sample_fmt",
        "s16",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

/// A 16-bit signal written into a 24-bit container: its low eight bits are
/// zero in every sample.
///
/// Through a 16-bit WAV, in two runs: in one filter graph FFmpeg negotiates
/// the 16-bit step away and the noise keeps all 24 bits.
fn padded(path: &Path, scratch: &Path) {
    let wav = scratch.join("padded-16.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        &noise(44_100, 6, 17),
        "-ac",
        "2",
        "-c:a",
        "pcm_s16le",
        wav.to_str().unwrap(),
    ]);
    ffmpeg(&[
        "-i",
        wav.to_str().unwrap(),
        "-sample_fmt",
        "s32",
        "-bits_per_raw_sample",
        "24",
        "-c:a",
        "flac",
        path.to_str().unwrap(),
    ]);
}

impl Shelf {
    async fn scan(&self) {
        run_scan(&self.state, self.owner, self.library.clone()).await;
    }

    async fn analyze(&self) -> usize {
        self.state.services.analyze_pending_quality().await.unwrap()
    }

    async fn track(&self, relative: &str) -> Uuid {
        let id: String =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id = ? AND relative_path = ?")
                .bind(self.library.id.to_string())
                .bind(relative)
                .fetch_one(self.state.db.pool())
                .await
                .unwrap_or_else(|_| panic!("no track at {relative:?}"));
        Uuid::parse_str(&id).unwrap()
    }

    async fn verdict(&self, relative: &str) -> QualityVerdict {
        let track = self.track(relative).await;
        self.state
            .services
            .track_quality(self.owner, track)
            .await
            .unwrap()
            .quality
            .verdict
    }

    async fn rows(&self) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM track_quality")
            .fetch_one(self.state.db.pool())
            .await
            .unwrap()
    }

    /// A token for the account named `username`, minted by the owner.
    async fn token(&self, username: &str) -> String {
        self.state
            .services
            .create_api_token(self.owner, &[], username, "quality", &[])
            .await
            .unwrap()
            .1
    }

    async fn join(&self, user: Uuid, role: LibraryRole) {
        self.state
            .db
            .add_library_member(self.owner, self.library.id, user, role, now_ms())
            .await
            .unwrap();
    }
}

/// The analysis's state, as the library's owner reads it.
async fn analysis(shelf: &Shelf) -> waveflow_server::services::QualityAnalysis {
    shelf
        .state
        .services
        .quality_summary(shelf.owner, shelf.library.id)
        .await
        .unwrap()
        .analysis
}

mod analysis;
mod routes;
