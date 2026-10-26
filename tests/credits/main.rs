//! Splitting a joined artist credit, by decision, and "Needs attention"
//! (RFC-015).
//!
//! The accounts here are rows written by hand and reach the routes through an
//! API token, as in `tests/catalog/listings.rs`: nothing in this target authenticates
//! with a password, so nothing in it hands a literal to a hash.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::json;
use support::*;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::auth::authentication::now_ms;
use waveflow_server::services::{
    AttentionCategory, BrowsePage, CreditDecisionKind, TrackMetadataPatch,
};
use waveflow_server::storage::catalog::CatalogTrackInput;
use waveflow_server::storage::database::{LibraryRole, LibraryVisibility};
use waveflow_server::AppState;

/// One credited row: track, role, position, name, sort form.
type Row = (String, String, i64, String, Option<String>);

struct Shelf {
    state: AppState,
    config: waveflow_server::Config,
    owner: Uuid,
    library: Uuid,
    scan: Uuid,
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
    let library = state
        .db
        .create_library(
            owner,
            name,
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    Shelf {
        state,
        config,
        owner,
        library,
        scan,
        _temp: temp,
    }
}

/// A file crediting `artist`, on its own album unless one is named, with no
/// album-artist tag and no compilation flag: the album then hangs off the
/// track's own credit, which is the case a split could have moved.
fn file(index: usize, artist: &str, album: Option<&str>) -> CatalogTrackInput {
    let mut input = catalog_input(index, artist);
    input.title = format!("Track {index}");
    input.album = album.map(str::to_owned);
    input.is_compilation = false;
    input.genre = None;
    input
}

impl Shelf {
    async fn apply(&self, input: &CatalogTrackInput) {
        let existing: Option<String> =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id = ? AND relative_path = ?")
                .bind(self.library.to_string())
                .bind(&input.relative_path)
                .fetch_optional(self.state.db.pool())
                .await
                .unwrap();
        self.state
            .db
            .apply_catalog_track(
                self.library,
                self.scan,
                input,
                existing.map(|id| Uuid::parse_str(&id).unwrap()),
                false,
            )
            .await
            .unwrap();
    }

    /// The end of a scan, in the scanner's order: sort names first, because
    /// the derivations index each artist with its sort name.
    async fn settle(&self) {
        self.state
            .db
            .consolidate_sort_names(self.library)
            .await
            .unwrap();
        self.state
            .db
            .consolidate_catalog_derivations(self.library)
            .await
            .unwrap();
    }

    async fn artist(&self, name: &str) -> Uuid {
        let id: String =
            sqlx::query_scalar("SELECT id FROM artist WHERE library_id = ? AND name = ?")
                .bind(self.library.to_string())
                .bind(name)
                .fetch_one(self.state.db.pool())
                .await
                .unwrap_or_else(|_| panic!("no artist named {name:?}"));
        Uuid::parse_str(&id).unwrap()
    }

    async fn artist_names(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT name FROM artist WHERE library_id = ? ORDER BY name")
            .bind(self.library.to_string())
            .fetch_all(self.state.db.pool())
            .await
            .unwrap()
    }

    async fn split(&self, name: &str) -> Uuid {
        let artist = self.artist(name).await;
        self.state
            .services
            .decide_credit(self.owner, self.library, artist, CreditDecisionKind::Split)
            .await
            .unwrap()
            .id
    }

    async fn undo(&self, decision: Uuid) {
        self.state
            .services
            .remove_credit_decision(self.owner, self.library, decision)
            .await
            .unwrap();
    }

    /// Every credited row of the library, by file, role and position.
    async fn rows(&self) -> Vec<Row> {
        sqlx::query_as(
            "SELECT t.relative_path, tp.role, tp.position, ar.name, tp.sort_name \
             FROM track_participant tp JOIN track t ON t.id = tp.track_id \
             JOIN artist ar ON ar.id = tp.artist_id \
             WHERE t.library_id = ? ORDER BY t.relative_path, tp.role, tp.position",
        )
        .bind(self.library.to_string())
        .fetch_all(self.state.db.pool())
        .await
        .unwrap()
    }

    /// One file's names in one role, in order.
    async fn credited(&self, index: usize, role: &str) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT ar.name FROM track_participant tp JOIN track t ON t.id = tp.track_id \
             JOIN artist ar ON ar.id = tp.artist_id \
             WHERE t.library_id = ? AND t.relative_path = ? AND tp.role = ? \
             ORDER BY tp.position",
        )
        .bind(self.library.to_string())
        .bind(format!("track-{index}.flac"))
        .bind(role)
        .fetch_all(self.state.db.pool())
        .await
        .unwrap()
    }

    async fn token(&self, user: Uuid, name: &str) -> String {
        self.state
            .services
            .create_api_token(user, &[], name, "credit decisions", &[])
            .await
            .unwrap()
            .1
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

mod page;
mod splits;
