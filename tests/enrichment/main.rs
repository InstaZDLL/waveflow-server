//! Metadata enrichment's cache and engine — RFC-011 decisions 3, 5, 6 and 9.
//!
//! Every test points the engine at a fake Last.fm on a loopback listener of its
//! own; the suite never reaches a real provider. The fake counts what it is
//! asked, which is what most of these assertions are about: enrichment's
//! failures are requests made that should not have been, or answers cached that
//! were never given.

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.

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
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use waveflow_server::{
    config::MetadataSettings,
    remote::enrichment::{Endpoints, Enrichment, Outcome, Provider},
    remote::portrait::AddressPolicy,
};

mod artist_info;
mod banners;
mod image_search;
mod lastfm;
mod portraits;
mod similar_songs;

#[derive(Clone, Copy)]
enum Mode {
    /// A biography and two neighbours.
    Hit,
    /// Last.fm's code 6 to both methods: it knows nothing of this artist.
    Miss,
    /// A failure status with a body that is not Last.fm's.
    Unavailable,
    /// A `302` to `/followed`, which answers like `Hit`.
    Redirect,
    /// A well-formed answer three mebibytes long.
    Oversized,
}

#[derive(Clone)]
struct Fake {
    mode: Mode,
    delay: Duration,
    info: Arc<AtomicUsize>,
    similar: Arc<AtomicUsize>,
    followed: Arc<AtomicUsize>,
}

struct Listener {
    fake: Fake,
    base_url: String,
}

async fn fake_lastfm(mode: Mode, delay: Duration) -> Listener {
    let fake = Fake {
        mode,
        delay,
        info: Arc::default(),
        similar: Arc::default(),
        followed: Arc::default(),
    };
    let router = Router::new()
        .route("/2.0/", get(answer))
        .route("/followed", get(followed))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Listener {
        fake,
        base_url: format!("http://{address}/2.0/"),
    }
}

async fn answer(
    State(fake): State<Fake>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let method = query.get("method").map(String::as_str).unwrap_or_default();
    match method {
        "artist.getinfo" => fake.info.fetch_add(1, Ordering::SeqCst),
        "artist.getsimilar" => fake.similar.fetch_add(1, Ordering::SeqCst),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    tokio::time::sleep(fake.delay).await;
    match fake.mode {
        Mode::Hit => found(method),
        Mode::Miss => {
            json(r#"{"error":6,"message":"The artist you supplied could not be found"}"#.to_owned())
        }
        Mode::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "<html>down</html>").into_response(),
        Mode::Redirect => (StatusCode::FOUND, [(header::LOCATION, "/followed")]).into_response(),
        Mode::Oversized => json(format!(
            r#"{{"artist":{{"name":"Björk","bio":{{"summary":"{}"}}}}}}"#,
            "a".repeat(3 * 1024 * 1024)
        )),
    }
}

async fn followed(State(fake): State<Fake>) -> Response {
    fake.followed.fetch_add(1, Ordering::SeqCst);
    found("artist.getinfo")
}

fn found(method: &str) -> Response {
    json(if method == "artist.getinfo" {
        r#"{"artist":{"name":"Björk","bio":{"summary":"Icelandic singer.","content":"Icelandic singer, at length."}}}"#.to_owned()
    } else {
        r#"{"similarartists":{"artist":[
            {"name":"Sigur Rós","match":"0.9","image":[]},
            {"name":"Múm","match":"0.75","image":[]}
        ]}}"#
            .to_owned()
    })
}

fn json(body: String) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// A key made up for the run, so no credential sits in the source and a log
/// search for it cannot match anything but this run's own output.
fn run_key() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn settings(providers: Vec<Provider>, key: &str) -> MetadataSettings {
    MetadataSettings {
        providers,
        lastfm_key: Some(key.to_owned()),
        // Fast enough that pacing never decides a test.
        rate_per_sec: 1_000,
        request_timeout: Duration::from_secs(5),
        ..MetadataSettings::off()
    }
}

fn engine(
    state: &waveflow_server::AppState,
    settings: &MetadataSettings,
    listener: &Listener,
) -> Enrichment {
    Enrichment::with_endpoints(
        state.db.clone(),
        settings,
        state.artwork_dir.clone(),
        Endpoints {
            lastfm: listener.base_url.clone(),
            ..Endpoints::default()
        },
    )
    .unwrap()
}

// ── The readers: getArtistInfo, getArtistInfo2 and the native artist page ──
//
// Accounts are inserted as rows rather than through `create_account`: these
// tests need user ids and libraries, not a login, and a password hashed from a
// literal is what makes CodeQL flag a test. They reach HTTP through an API
// token and a Subsonic key minted at run time.
async fn account(state: &waveflow_server::AppState, username: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
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

/// A private library of `owner`'s holding one album by each of `artists`.
async fn library_of(
    state: &waveflow_server::AppState,
    config: &waveflow_server::Config,
    owner: uuid::Uuid,
    name: &str,
    artists: &[&str],
) {
    let credits: Vec<_> = artists.iter().map(|artist| (*artist, *artist)).collect();
    library_with(state, config, owner, name, &credits).await;
}

/// The same, one track per `(track artist, album artist)` pair, on the album
/// named after its album artist.
async fn library_with(
    state: &waveflow_server::AppState,
    config: &waveflow_server::Config,
    owner: uuid::Uuid,
    name: &str,
    artists: &[(&str, &str)],
) {
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
        .start_scan_job(scan, artists.len() as i64, false)
        .await
        .unwrap();
    for (index, (artist, album_artist)) in artists.iter().enumerate() {
        let mut input = support::browse_input(
            index + name.len() * 100,
            "A song",
            &format!("{album_artist} album"),
            artist,
            Some(1),
            Some(1),
        );
        input.album_artist = Some((*album_artist).into());
        input.relative_path = format!("{name}-{index}.flac");
        input.quick_hash = format!("{:064x}", 70_000 + index + name.len() * 100);
        input.full_hash = format!("{:064x}", 80_000 + index + name.len() * 100);
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
}

async fn artist_id(state: &waveflow_server::AppState, user: uuid::Uuid, name: &str) -> uuid::Uuid {
    state
        .services
        .list_artists(
            user,
            None,
            waveflow_server::services::BrowsePage::new(None, None).unwrap(),
        )
        .await
        .unwrap()
        .into_iter()
        .find(|artist| artist.artist.name == name)
        .unwrap_or_else(|| panic!("{name} is not visible"))
        .artist
        .id
}

/// Björk and Sigur Rós in the owner's library; Múm only in the outsider's.
/// The fake answers Sigur Rós and Múm as Björk's neighbours.
async fn two_members() -> (
    tempfile::TempDir,
    waveflow_server::Config,
    waveflow_server::AppState,
    uuid::Uuid,
    uuid::Uuid,
) {
    let (temp, config, state) = support::test_app().await;
    let owner = account(&state, "enrichment-owner").await;
    let outsider = account(&state, "enrichment-outsider").await;
    library_of(&state, &config, owner, "owned", &["Björk", "Sigur Rós"]).await;
    library_of(&state, &config, outsider, "foreign", &["Múm"]).await;
    (temp, config, state, owner, outsider)
}

/// A Subsonic API key for `user`, minted at run time, with the encrypted
/// password beside it the credential row requires.
async fn subsonic_key(state: &waveflow_server::AppState, user: uuid::Uuid) -> String {
    let api_key = format!("wfsk_{}", run_key());
    let password = run_key();
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

// ── Portraits: Deezer and the narrow fetcher (decision 8) ──
/// A fake Deezer: the artist search, and the picture it points at, served by
/// the same loopback listener under whatever host name the test chooses.
#[derive(Clone)]
struct Deezer {
    searches: Arc<AtomicUsize>,
    images: Arc<AtomicUsize>,
    picture: Arc<String>,
    bytes: Arc<Vec<u8>>,
}

/// A JPEG by its first bytes, which is all the fetcher checks; the rest is
/// made up for the run so two runs never share a hash.
fn portrait_bytes() -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0];
    bytes.extend_from_slice(run_key().as_bytes());
    bytes
}

/// Björk in a library of each of two accounts: two artist rows, one name.
async fn bjork_twice() -> (
    tempfile::TempDir,
    waveflow_server::AppState,
    uuid::Uuid,
    uuid::Uuid,
) {
    let (temp, config, state) = support::test_app().await;
    let owner = account(&state, "portrait-owner").await;
    let other = account(&state, "portrait-other").await;
    library_of(&state, &config, owner, "mine", &["Björk"]).await;
    library_of(&state, &config, other, "theirs", &["Björk"]).await;
    (temp, state, owner, other)
}

async fn artist_events(state: &waveflow_server::AppState) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM library_event WHERE entity_type = 'artist'")
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

// ── Portrait candidates: a Deezer search the server previews (RFC-013) ──
use waveflow_server::services::ServiceError;
