//! RFC-024: lyrics from LRCLIB, listening statistics, mixes and the local
//! fallback for similar songs.
//!
//! LRCLIB is always a fake on a loopback listener of the test's own; the
//! suite never reaches `lrclib.net`. The fake counts what it is asked, which
//! is what most of the lyrics assertions are about.
//!
//! Accounts are inserted as rows rather than through `create_account`, as in
//! `tests/enrichment/main.rs`: these tests need user ids, not a login, and the API
//! is reached through tokens and Subsonic keys drawn at run time.

#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use uuid::Uuid;
use waveflow_server::{
    config::LyricsSettings,
    library::lyrics::LyricsInput,
    remote::lrclib::{LyricsLookup, LyricsProvider},
    services::ServiceError,
    AppState, Config,
};

// ── Fixtures ──

async fn account(state: &AppState, username: &str) -> Uuid {
    let id = Uuid::new_v4();
    let now = waveflow_server::auth::authentication::now_ms();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(id.to_string())
    .bind(username)
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();
    id
}

/// One file of a fixture library.
#[derive(Clone)]
struct File {
    title: String,
    artists: Vec<String>,
    album: String,
    album_artist: String,
    genre: Option<String>,
    year: Option<i64>,
    duration_ms: i64,
    lyrics: Option<&'static str>,
}

fn file(title: &str, artist: &str, genre: Option<&str>, year: Option<i64>) -> File {
    File {
        title: title.to_owned(),
        artists: vec![artist.to_owned()],
        album: format!("{artist} album"),
        album_artist: artist.to_owned(),
        genre: genre.map(str::to_owned),
        year,
        duration_ms: 234_000,
        lyrics: None,
    }
}

/// A private library of `owner`'s holding `files`, and the ids of their
/// tracks in the order given. Every library gets its own range of hashes.
async fn library(
    state: &AppState,
    config: &Config,
    owner: Uuid,
    name: &str,
    files: &[File],
) -> (Uuid, Vec<Uuid>) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let now = waveflow_server::auth::authentication::now_ms();
    let root = config.data_dir.join(name);
    std::fs::create_dir_all(&root).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            name,
            &std::fs::canonicalize(&root).unwrap(),
            waveflow_server::storage::database::LibraryVisibility::Private,
            now,
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state
        .db
        .start_scan_job(scan, files.len() as i64, false)
        .await
        .unwrap();
    for (offset, spec) in files.iter().enumerate() {
        let index = NEXT.fetch_add(1, Ordering::SeqCst) + 10_000;
        let mut input = support::browse_input(
            index,
            &spec.title,
            &spec.album,
            &spec.artists.join(", "),
            Some(offset as i64 + 1),
            Some(1),
        );
        input.artists = spec.artists.clone();
        input.album_artist = Some(spec.album_artist.clone());
        input.genre = spec.genre.clone();
        input.year = spec.year;
        input.duration_ms = spec.duration_ms;
        input.relative_path = format!("{name}/{offset}.flac");
        if let Some(words) = spec.lyrics {
            input.lyrics = vec![LyricsInput {
                source: "embedded",
                lang: "eng".into(),
                synced: false,
                content: words.into(),
            }];
        }
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let mut ids = Vec::new();
    for offset in 0..files.len() {
        let id: String =
            sqlx::query_scalar("SELECT id FROM track WHERE library_id=? AND relative_path=?")
                .bind(library.to_string())
                .bind(format!("{name}/{offset}.flac"))
                .fetch_one(state.db.pool())
                .await
                .unwrap();
        ids.push(id.parse().unwrap());
    }
    (library, ids)
}

async fn token(state: &AppState, user: Uuid, username: &str) -> String {
    state
        .services
        .create_api_token(user, &[], username, "discovery", &[])
        .await
        .unwrap()
        .1
}

async fn subsonic_key(state: &AppState, user: Uuid) -> String {
    let api_key = format!("wfsk_{}", Uuid::new_v4().simple());
    let password = Uuid::new_v4().simple().to_string();
    state
        .db
        .set_subsonic_credential(
            user,
            user,
            &state.secret_box.encrypt(password.as_bytes()).unwrap(),
            &waveflow_server::auth::security::token_hash(&api_key),
            waveflow_server::auth::authentication::now_ms(),
        )
        .await
        .unwrap();
    api_key
}

async fn get_json(router: &Router, uri: &str, token: &str) -> (StatusCode, serde_json::Value) {
    let response = support::send_json(router, "GET", uri, token, None).await;
    let status = response.status();
    let text = support::body_text(response).await;
    (status, serde_json::from_str(&text).unwrap_or_default())
}

/// A listen of `track` by `user`, `ago` milliseconds before now.
async fn listen(state: &AppState, user: Uuid, track: Uuid, ago: i64) {
    let at = waveflow_server::auth::authentication::now_ms() - ago;
    state
        .services
        .scrobble(user, track, true, Some(at))
        .await
        .unwrap();
}

const MINUTE: i64 = 60_000;

const DAY: i64 = 24 * 60 * MINUTE;

// ── A fake LRCLIB ──

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Synced and plain lyrics, for any track.
    Hit,
    /// LRCLIB's 404.
    Miss,
    /// An instrumental record.
    Instrumental,
    /// A 503 with a body that is not LRCLIB's.
    Unavailable,
    /// A `302` to `/followed`, which answers like `Hit`.
    Redirect,
    /// A well-formed answer two mebibytes long.
    Oversized,
}

#[derive(Clone)]
struct Fake {
    mode: Arc<Mutex<Mode>>,
    delay: Duration,
    asked: Arc<AtomicUsize>,
    followed: Arc<AtomicUsize>,
    /// The query of the last request, and its User-Agent.
    last: Arc<Mutex<Option<Asked>>>,
}

/// One request's query parameters, and the agent that sent them.
type Asked = (HashMap<String, String>, String);

const SYNCED: &str = "[00:01.00]If you complain once more\n[00:04.50]You'll meet an army of me";

const PLAIN: &str = "If you complain once more\nYou'll meet an army of me";

async fn answer(
    State(fake): State<Fake>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    fake.asked.fetch_add(1, Ordering::SeqCst);
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let duration: f64 = query
        .get("duration")
        .and_then(|value| value.parse().ok())
        .unwrap_or_default();
    *fake.last.lock().unwrap() = Some((query, agent));
    tokio::time::sleep(fake.delay).await;
    let mode = *fake.mode.lock().unwrap();
    match mode {
        Mode::Hit => record(duration),
        Mode::Miss => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"code":404,"name":"TrackNotFound","message":"Failed to find specified track"}"#,
        )
            .into_response(),
        Mode::Instrumental => json(format!(
            r#"{{"id":1,"duration":{duration},"instrumental":true,"plainLyrics":null,"syncedLyrics":null}}"#
        )),
        Mode::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "<html>down</html>").into_response(),
        Mode::Redirect => (StatusCode::FOUND, [(header::LOCATION, "/followed")]).into_response(),
        Mode::Oversized => json(format!(
            r#"{{"duration":{duration},"plainLyrics":"{}"}}"#,
            "a".repeat(2 * 1024 * 1024)
        )),
    }
}

async fn followed(State(fake): State<Fake>) -> Response {
    fake.followed.fetch_add(1, Ordering::SeqCst);
    record(234.0)
}

fn record(duration: f64) -> Response {
    json(
        serde_json::json!({
            "id": 1,
            "trackName": "Army of Me",
            "artistName": "Björk",
            "albumName": "Post",
            "duration": duration,
            "instrumental": false,
            "plainLyrics": PLAIN,
            "syncedLyrics": SYNCED,
        })
        .to_string(),
    )
}

fn json(body: String) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

mod listening;
mod lyrics;
mod similar;
