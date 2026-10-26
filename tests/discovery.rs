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

struct Listener {
    fake: Fake,
    base_url: String,
}

async fn fake_lrclib(mode: Mode, delay: Duration) -> Listener {
    let fake = Fake {
        mode: Arc::new(Mutex::new(mode)),
        delay,
        asked: Arc::default(),
        followed: Arc::default(),
        last: Arc::default(),
    };
    let router = Router::new()
        .route("/api/get", get(answer))
        .route("/followed", get(followed))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Listener {
        fake,
        base_url: format!("http://{address}"),
    }
}

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

fn settings(providers: Vec<LyricsProvider>) -> LyricsSettings {
    LyricsSettings {
        providers,
        // Fast enough that pacing never decides a test.
        rate_per_sec: 1_000,
        request_timeout: Duration::from_secs(5),
        ..LyricsSettings::off()
    }
}

fn lookup(state: &AppState, listener: &Listener) -> LyricsLookup {
    LyricsLookup::with_base_url(
        state.db.clone(),
        &settings(vec![LyricsProvider::Lrclib]),
        listener.base_url.clone(),
    )
    .unwrap()
}

async fn lookup_rows(state: &AppState) -> Vec<(String, i64, i64)> {
    sqlx::query_as("SELECT outcome, fetched_at, expires_at FROM lyrics_lookup")
        .fetch_all(state.db.pool())
        .await
        .unwrap()
}

/// One owner, one library: a track with no lyrics, one with embedded lyrics
/// and one that will be given an upload.
struct Shelf {
    _temp: tempfile::TempDir,
    config: Config,
    state: AppState,
    owner: Uuid,
    bare: Uuid,
    embedded: Uuid,
    uploaded: Uuid,
}

async fn shelf() -> Shelf {
    let (temp, config, state) = support::test_app().await;
    let owner = account(&state, "lyrics-owner").await;
    let mut army = file("Army of Me", "Björk", Some("Pop"), Some(1995));
    army.album = "Post".into();
    let mut hyper = file("Hyperballad", "Björk", Some("Pop"), Some(1995));
    hyper.lyrics = Some("We live on a mountain");
    let isobel = file("Isobel", "Björk", Some("Pop"), Some(1995));
    let (_, ids) = library(&state, &config, owner, "lyrics", &[army, hyper, isobel]).await;
    state
        .services
        .upload_lyrics(
            owner,
            ids[2],
            waveflow_server::library::lyrics::UploadFormat::Txt,
            b"My name Isobel",
        )
        .await
        .unwrap();
    Shelf {
        _temp: temp,
        config,
        state,
        owner,
        bare: ids[0],
        embedded: ids[1],
        uploaded: ids[2],
    }
}

// ── LRCLIB: decisions 1 to 5 ──

/// Decision 1: the list is the switch. Built with no provider, the lookup is
/// disabled and the fake is never asked.
#[tokio::test]
async fn nothing_is_asked_while_lrclib_is_not_named() {
    let shelf = shelf().await;
    let listener = fake_lrclib(Mode::Hit, Duration::ZERO).await;
    let off = LyricsLookup::with_base_url(
        shelf.state.db.clone(),
        &settings(Vec::new()),
        listener.base_url.clone(),
    )
    .unwrap();
    assert!(!off.is_enabled());
    shelf.state.services.use_lyrics_lookup(off);

    let native = shelf
        .state
        .services
        .track_lyrics(shelf.owner, shelf.bare)
        .await
        .unwrap();
    assert!(native.structured_lyrics.is_empty());
    let facade = shelf
        .state
        .services
        .lyrics(shelf.owner, shelf.bare)
        .await
        .unwrap();
    assert!(facade.structured_lyrics.is_empty());
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 0);
    assert!(lookup_rows(&shelf.state).await.is_empty());
}

/// Decisions 2, 3 and 5: a track with no lyrics is looked up on the first
/// ask, answered with the synced sheet credited to LRCLIB, and the second
/// ask reads the cache.
#[tokio::test]
async fn a_track_without_lyrics_is_looked_up_once_and_credited() {
    let shelf = shelf().await;
    let listener = fake_lrclib(Mode::Hit, Duration::ZERO).await;
    shelf
        .state
        .services
        .use_lyrics_lookup(lookup(&shelf.state, &listener));
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let bearer = token(&shelf.state, shelf.owner, "lyrics-owner").await;

    let (status, body) = get_json(
        &router,
        &format!("/api/v2/tracks/{}/lyrics", shelf.bare),
        &bearer,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let sheets = body["structuredLyrics"].as_array().unwrap();
    assert_eq!(sheets.len(), 1, "{body}");
    assert_eq!(sheets[0]["source"], "lrclib");
    assert_eq!(sheets[0]["synced"], true);
    assert_eq!(sheets[0]["lang"], "xxx");
    assert_eq!(sheets[0]["line"][1]["start"], 4_500);
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 1);

    // What was sent: the four fields LRCLIB matches on, and an agent that
    // names the project.
    let (query, agent) = listener.fake.last.lock().unwrap().clone().unwrap();
    assert_eq!(query["artist_name"], "Björk");
    assert_eq!(query["track_name"], "Army of Me");
    assert_eq!(query["album_name"], "Post");
    assert_eq!(query["duration"], "234");
    assert!(agent.starts_with("WaveFlowServer/"), "{agent}");
    assert!(
        agent.contains("https://github.com/InstaZDLL/waveflow-server"),
        "{agent}"
    );

    // The cache answers the second ask, and the download serves the sheet.
    let again = shelf
        .state
        .services
        .track_lyrics(shelf.owner, shelf.bare)
        .await
        .unwrap();
    assert_eq!(again.structured_lyrics.len(), 1);
    let download = shelf
        .state
        .services
        .lyrics_file(shelf.owner, shelf.bare, None)
        .await
        .unwrap();
    assert!(download.name.ends_with(".lrc"), "{}", download.name);
    assert!(download.content.contains("[00:04.50]"));
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 1);
    let rows = lookup_rows(&shelf.state).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "hit");
    assert_eq!(rows[0].2 - rows[0].1, 30 * DAY);
}

/// Decision 2: the file and the uploads always win, on every surface.
#[tokio::test]
async fn a_track_with_lyrics_of_its_own_is_never_looked_up() {
    let shelf = shelf().await;
    let listener = fake_lrclib(Mode::Hit, Duration::ZERO).await;
    shelf
        .state
        .services
        .use_lyrics_lookup(lookup(&shelf.state, &listener));

    let embedded = shelf
        .state
        .services
        .track_lyrics(shelf.owner, shelf.embedded)
        .await
        .unwrap();
    assert_eq!(embedded.structured_lyrics.len(), 1);
    assert_eq!(
        embedded.structured_lyrics[0].source.as_deref(),
        Some("embedded")
    );
    let uploaded = shelf
        .state
        .services
        .track_lyrics(shelf.owner, shelf.uploaded)
        .await
        .unwrap();
    assert_eq!(
        uploaded.structured_lyrics[0].source.as_deref(),
        Some("upload_txt")
    );
    // The façade does not serve uploads (RFC-012), and still does not look
    // the track up: its owner chose its words.
    let facade = shelf
        .state
        .services
        .lyrics(shelf.owner, shelf.uploaded)
        .await
        .unwrap();
    assert!(facade.structured_lyrics.is_empty());
    let facade = shelf
        .state
        .services
        .lyrics(shelf.owner, shelf.embedded)
        .await
        .unwrap();
    assert_eq!(facade.structured_lyrics.len(), 1);
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 0);
}

/// Decision 5: `getLyricsBySongId` carries the sheet, synced, with no source
/// attribute — the frozen element has none — and `getLyrics` finds it by
/// exact artist and title.
#[tokio::test]
async fn the_facade_serves_the_sheet_without_a_source() {
    let shelf = shelf().await;
    let listener = fake_lrclib(Mode::Hit, Duration::ZERO).await;
    shelf
        .state
        .services
        .use_lyrics_lookup(lookup(&shelf.state, &listener));
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let api_key = subsonic_key(&shelf.state, shelf.owner).await;

    let by_id = support::subsonic_json(
        &router,
        "getLyricsBySongId",
        &api_key,
        &format!("&id={}", shelf.bare),
    )
    .await;
    let sheets = by_id["subsonic-response"]["lyricsList"]["structuredLyrics"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {by_id}"));
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0]["synced"], true);
    assert!(support::omits(&sheets[0], "source"), "{by_id}");
    // And at the service the façade reads, not only in the node it builds:
    // the sheet names no source there either.
    let facade = shelf
        .state
        .services
        .lyrics(shelf.owner, shelf.bare)
        .await
        .unwrap();
    assert_eq!(facade.structured_lyrics.len(), 1);
    assert!(facade.structured_lyrics[0].source.is_none());

    let by_name = support::subsonic_json(
        &router,
        "getLyrics",
        &api_key,
        "&artist=Bj%C3%B6rk&title=Army%20of%20Me",
    )
    .await;
    let value = by_name["subsonic-response"]["lyrics"]["value"]
        .as_str()
        .unwrap_or_else(|| panic!("lyrics text: {by_name}"));
    assert!(value.contains("army of me"), "{by_name}");
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 1);
}

/// Decision 4: a 404 and an instrumental are answers, cached; a miss for the
/// short TTL, an instrumental for the long one, and neither is a sheet.
#[tokio::test]
async fn a_miss_and_an_instrumental_are_cached_as_answers() {
    for (mode, outcome, ttl) in [
        (Mode::Miss, "miss", 7 * DAY),
        (Mode::Instrumental, "instrumental", 30 * DAY),
    ] {
        let shelf = shelf().await;
        let listener = fake_lrclib(mode, Duration::ZERO).await;
        shelf
            .state
            .services
            .use_lyrics_lookup(lookup(&shelf.state, &listener));
        for _ in 0..2 {
            let answer = shelf
                .state
                .services
                .track_lyrics(shelf.owner, shelf.bare)
                .await
                .unwrap();
            assert!(answer.structured_lyrics.is_empty());
        }
        assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 1, "{outcome}");
        let rows = lookup_rows(&shelf.state).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, outcome);
        assert_eq!(rows[0].2 - rows[0].1, ttl, "{outcome}");
    }
}

/// Decision 4: an outage, a redirect and an oversized answer are unreached.
/// They write nothing, so the next ask tries again — and the redirect is not
/// followed.
#[tokio::test]
async fn an_unreached_lookup_writes_nothing_and_is_asked_again() {
    for mode in [Mode::Unavailable, Mode::Redirect, Mode::Oversized] {
        let shelf = shelf().await;
        let listener = fake_lrclib(mode, Duration::ZERO).await;
        let lookup = lookup(&shelf.state, &listener);
        shelf.state.services.use_lyrics_lookup(lookup.clone());
        for _ in 0..2 {
            let answer = shelf
                .state
                .services
                .track_lyrics(shelf.owner, shelf.bare)
                .await
                .unwrap();
            assert!(answer.structured_lyrics.is_empty());
            lookup.idle().await;
        }
        assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 2);
        assert_eq!(listener.fake.followed.load(Ordering::SeqCst), 0);
        assert!(lookup_rows(&shelf.state).await.is_empty());
    }
}

/// Decision 3: concurrent asks for one track wait on one request.
#[tokio::test]
async fn concurrent_asks_make_one_request() {
    let shelf = shelf().await;
    let listener = fake_lrclib(Mode::Hit, Duration::from_millis(300)).await;
    shelf
        .state
        .services
        .use_lyrics_lookup(lookup(&shelf.state, &listener));
    let asks = (0..6).map(|_| {
        let services = shelf.state.services.clone();
        let (owner, track) = (shelf.owner, shelf.bare);
        tokio::spawn(async move { services.track_lyrics(owner, track).await.unwrap() })
    });
    for ask in futures_util::future::join_all(asks).await {
        assert_eq!(ask.unwrap().structured_lyrics.len(), 1);
    }
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 1);
}

/// Decision 3: the answering path waits a bounded time. A slower answer is
/// not waited for, and is in the cache for the next listen.
#[tokio::test]
async fn a_slow_answer_is_not_waited_for_and_lands_for_the_next_listen() {
    let shelf = shelf().await;
    let listener = fake_lrclib(Mode::Hit, Duration::from_millis(600)).await;
    let lookup = lookup(&shelf.state, &listener).with_answer_wait(Duration::from_millis(50));
    shelf.state.services.use_lyrics_lookup(lookup.clone());

    let started = std::time::Instant::now();
    let first = shelf
        .state
        .services
        .track_lyrics(shelf.owner, shelf.bare)
        .await
        .unwrap();
    assert!(first.structured_lyrics.is_empty());
    assert!(started.elapsed() < Duration::from_millis(500));
    lookup.idle().await;
    let next = shelf
        .state
        .services
        .track_lyrics(shelf.owner, shelf.bare)
        .await
        .unwrap();
    assert_eq!(next.structured_lyrics.len(), 1);
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 1);
}

/// Decision 4: a stale row is served at once, and its refresh is scheduled.
#[tokio::test]
async fn a_stale_answer_is_served_while_it_is_refreshed() {
    let shelf = shelf().await;
    let listener = fake_lrclib(Mode::Hit, Duration::ZERO).await;
    let lookup = lookup(&shelf.state, &listener);
    shelf.state.services.use_lyrics_lookup(lookup.clone());
    shelf
        .state
        .services
        .track_lyrics(shelf.owner, shelf.bare)
        .await
        .unwrap();
    sqlx::query("UPDATE lyrics_lookup SET expires_at = 1")
        .execute(shelf.state.db.pool())
        .await
        .unwrap();
    *listener.fake.mode.lock().unwrap() = Mode::Miss;

    let stale = shelf
        .state
        .services
        .track_lyrics(shelf.owner, shelf.bare)
        .await
        .unwrap();
    assert_eq!(
        stale.structured_lyrics.len(),
        1,
        "the stale sheet is served"
    );
    lookup.idle().await;
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 2);
    assert_eq!(lookup_rows(&shelf.state).await[0].0, "miss");
}

/// Decision 4: the tenancy lookup comes first. Another account's track is
/// not found, and nothing is read or sent about it.
#[tokio::test]
async fn another_account_s_track_is_not_found_and_not_looked_up() {
    let shelf = shelf().await;
    let outsider = account(&shelf.state, "lyrics-outsider").await;
    let listener = fake_lrclib(Mode::Hit, Duration::ZERO).await;
    shelf
        .state
        .services
        .use_lyrics_lookup(lookup(&shelf.state, &listener));

    assert!(matches!(
        shelf
            .state
            .services
            .track_lyrics(outsider, shelf.bare)
            .await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        shelf.state.services.lyrics(outsider, shelf.bare).await,
        Err(ServiceError::NotFound)
    ));
    let by_name = shelf
        .state
        .services
        .lyrics_by_metadata(outsider, Some("Björk"), Some("Army of Me"))
        .await
        .unwrap();
    assert!(by_name.is_none());
    assert_eq!(listener.fake.asked.load(Ordering::SeqCst), 0);
}

// ── Statistics: decision 6 ──

/// Two accounts, each with a library; the first also reaches the second's
/// for a while, then leaves it.
#[tokio::test]
async fn statistics_count_the_caller_s_listens_on_tracks_they_still_reach() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "stats-me").await;
    let other = account(&state, "stats-other").await;
    let (_, mine) = library(
        &state,
        &config,
        me,
        "stats-mine",
        &[
            file("One", "Björk", Some("Pop"), Some(1995)),
            file("Two", "Björk", Some("Pop; Electronic"), Some(1997)),
            file("Three", "Sigur Rós", Some("Post-Rock"), Some(2002)),
        ],
    )
    .await;
    let (theirs_id, theirs) = library(
        &state,
        &config,
        other,
        "stats-theirs",
        &[file("Elsewhere", "Björk", Some("Pop"), Some(2001))],
    )
    .await;
    // Joined for a while: my listen there counts while I am a member.
    sqlx::query(
        "INSERT INTO library_member (library_id, user_id, role, created_at) \
         VALUES (?, ?, 'listener', 0)",
    )
    .bind(theirs_id.to_string())
    .bind(me.to_string())
    .execute(state.db.pool())
    .await
    .unwrap();
    listen(&state, me, theirs[0], 3 * MINUTE).await;
    for ago in [MINUTE, 2 * DAY, 40 * DAY] {
        listen(&state, me, mine[1], ago).await;
    }
    listen(&state, me, mine[0], 5 * DAY).await;
    listen(&state, me, mine[2], 6 * DAY).await;
    // The other account's listens are never mine.
    for _ in 0..5 {
        listen(&state, other, theirs[0], MINUTE).await;
    }
    // A now playing is not a listen.
    state
        .services
        .scrobble(me, mine[2], false, None)
        .await
        .unwrap();

    let query = |period| waveflow_server::services::StatsQuery::new(Some(period), None, None);
    let week = state
        .services
        .listening_stats(me, query("7d").unwrap())
        .await
        .unwrap();
    assert_eq!(week.totals.plays, 5);
    assert_eq!(week.totals.listening_ms, 5 * 234_000);
    assert_eq!(week.totals.tracks, 4);
    // Björk in two libraries is one artist.
    assert_eq!(week.totals.artists, 2);
    assert_eq!(week.top_tracks[0].song.title, "Two");
    assert_eq!(week.top_tracks[0].plays, 2);
    assert_eq!(week.top_artists[0].artist.name, "Björk");
    assert_eq!(week.top_artists[0].plays, 4);
    assert_eq!(week.top_genres[0].name, "Pop");
    assert_eq!(week.top_genres[0].plays, 4);
    assert_eq!(week.timeline.len(), 7);
    assert_eq!(
        week.timeline.iter().map(|bucket| bucket.plays).sum::<i64>(),
        5
    );
    assert_eq!(week.first_play.as_ref().unwrap().song.title, "Three");
    assert_eq!(week.last_play.as_ref().unwrap().song.title, "Two");
    assert!(week.top_albums.iter().all(|entry| entry.plays > 0));

    let month = state
        .services
        .listening_stats(me, query("30d").unwrap())
        .await
        .unwrap();
    assert_eq!(month.totals.plays, 5);
    assert_eq!(month.timeline.len(), 30);
    let all = state
        .services
        .listening_stats(me, query("all").unwrap())
        .await
        .unwrap();
    assert_eq!(all.totals.plays, 6);
    assert!(all.from.is_none());
    let year = state
        .services
        .listening_stats(me, query("365d").unwrap())
        .await
        .unwrap();
    assert_eq!(year.timeline.len(), 12);

    // Leaving the library takes its listens out, as it takes the track out
    // of every other answer.
    sqlx::query("DELETE FROM library_member WHERE library_id=? AND user_id=?")
        .bind(theirs_id.to_string())
        .bind(me.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let after = state
        .services
        .listening_stats(me, query("7d").unwrap())
        .await
        .unwrap();
    assert_eq!(after.totals.plays, 4);
    assert!(after
        .top_tracks
        .iter()
        .all(|entry| entry.song.title != "Elsewhere"));

    // And the other account sees only its own five.
    let theirs_stats = state
        .services
        .listening_stats(other, query("7d").unwrap())
        .await
        .unwrap();
    assert_eq!(theirs_stats.totals.plays, 5);
}

/// The route: its parameters are validated and it needs a credential.
#[tokio::test]
async fn the_statistics_route_validates_its_parameters() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "stats-route").await;
    let router = waveflow_server::app(&config, state.clone());
    let bearer = token(&state, me, "stats-route").await;

    let (status, body) = get_json(
        &router,
        "/api/v2/stats?period=7d&utc_offset_minutes=120",
        &bearer,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["period"], "7d");
    assert_eq!(body["bucket"], "day");
    assert_eq!(body["totals"]["plays"], 0);
    assert_eq!(body["timeline"].as_array().unwrap().len(), 7);
    for bad in [
        "period=90d",
        "limit=0",
        "limit=51",
        "utc_offset_minutes=900",
    ] {
        let (status, _) = get_json(&router, &format!("/api/v2/stats?{bad}"), &bearer).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad}");
    }
    let anonymous = support::send_json(&router, "GET", "/api/v2/stats", "not-a-token", None).await;
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

// ── Mixes: decision 7 ──

#[tokio::test]
async fn mixes_are_listed_and_an_empty_one_is_left_out() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "mix-list").await;
    let files: Vec<File> = (0..12)
        .map(|n| {
            file(
                &format!("Song {n}"),
                &format!("Artist {n}"),
                Some("Jazz"),
                Some(1960),
            )
        })
        .collect();
    library(&state, &config, me, "mix-list", &files).await;
    let query = waveflow_server::services::MixQuery::new(Vec::new(), None, None).unwrap();

    let mixes = state.services.list_mixes(me, &query).await.unwrap();
    let ids: Vec<&str> = mixes.iter().map(|mix| mix.id.as_str()).collect();
    // Nothing played and nothing starred: those two are not offered.
    assert_eq!(
        ids,
        [
            "recently-added",
            "never-played",
            "genre:jazz",
            "decade:1960"
        ],
        "{ids:?}"
    );
    assert_eq!(mixes[2].genre.as_deref(), Some("Jazz"));
    assert_eq!(mixes[3].decade, Some(1960));
    assert!(mixes.iter().all(|mix| mix.track_count == 12));
    assert!(matches!(
        state.services.mix(me, "most-played", &query).await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        state.services.mix(me, "no-such-mix", &query).await,
        Err(ServiceError::NotFound)
    ));
}

/// Variety and determinism: no artist holds more than its share, two asks
/// on one day answer one mix, and a listened track leaves "never played".
#[tokio::test]
async fn a_mix_is_varied_and_the_same_all_day() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "mix-vary").await;
    let mut files: Vec<File> = (0..30)
        .map(|n| {
            file(
                &format!("Prolific {n}"),
                "Prolific",
                Some("Rock"),
                Some(1990),
            )
        })
        .collect();
    files.extend((0..10).map(|n| {
        file(
            &format!("Other {n}"),
            &format!("Other {n}"),
            Some("Rock"),
            Some(1990),
        )
    }));
    let (_, ids) = library(&state, &config, me, "mix-vary", &files).await;
    let query = waveflow_server::services::MixQuery::new(Vec::new(), Some(20), None).unwrap();

    let first = state
        .services
        .mix(me, "never-played", &query)
        .await
        .unwrap();
    let again = state
        .services
        .mix(me, "never-played", &query)
        .await
        .unwrap();
    let order = |mix: &waveflow_server::services::Mix| {
        mix.songs.iter().map(|song| song.id).collect::<Vec<_>>()
    };
    assert_eq!(order(&first), order(&again));
    let prolific = first
        .songs
        .iter()
        .filter(|song| song.artist.as_deref() == Some("Prolific"))
        .count();
    assert_eq!(prolific, 2, "a tenth of twenty, and never fewer than two");
    assert_eq!(first.songs.len(), 12);
    for pair in first.songs.windows(2) {
        assert_ne!(pair[0].artist, pair[1].artist);
    }

    // Most played is a ranking: twice before once, whatever came last.
    listen(&state, me, ids[31], 3 * MINUTE).await;
    listen(&state, me, ids[31], 2 * MINUTE).await;
    listen(&state, me, ids[30], MINUTE).await;
    let played = state.services.mix(me, "most-played", &query).await.unwrap();
    assert_eq!(order(&played), [ids[31], ids[30]]);
    let never = state
        .services
        .mix(me, "never-played", &query)
        .await
        .unwrap();
    assert!(!order(&never).contains(&ids[30]));
    assert!(!order(&never).contains(&ids[31]));
}

/// A favourite is forgotten when starred long ago and not listened to
/// lately; a fresh star and a recent listen both keep it out.
#[tokio::test]
async fn forgotten_favourites_are_old_stars_not_listened_to_lately() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "mix-forgotten").await;
    let files: Vec<File> = (0..3)
        .map(|n| file(&format!("Star {n}"), &format!("Starred {n}"), None, None))
        .collect();
    let (_, ids) = library(&state, &config, me, "mix-forgotten", &files).await;
    let now = waveflow_server::auth::authentication::now_ms();
    for (track, starred_ago) in [(ids[0], 60 * DAY), (ids[1], DAY), (ids[2], 60 * DAY)] {
        sqlx::query(
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'track', ?, ?)",
        )
        .bind(me.to_string())
        .bind(track.to_string())
        .bind(now - starred_ago)
        .execute(state.db.pool())
        .await
        .unwrap();
    }
    listen(&state, me, ids[2], 10 * DAY).await;
    let query = waveflow_server::services::MixQuery::new(Vec::new(), None, None).unwrap();
    let mix = state
        .services
        .mix(me, "forgotten-favourites", &query)
        .await
        .unwrap();
    assert_eq!(
        mix.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
        [ids[0]]
    );
}

/// Tenancy: another account's tracks are never in a mix, and a genre only
/// another account holds is not found.
#[tokio::test]
async fn a_mix_holds_only_the_caller_s_tracks() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "mix-me").await;
    let other = account(&state, "mix-other").await;
    let (_, mine) = library(
        &state,
        &config,
        me,
        "mix-me",
        &[file("Mine", "Me", Some("Folk"), Some(1970))],
    )
    .await;
    library(
        &state,
        &config,
        other,
        "mix-other",
        &[file("Theirs", "Them", Some("Grime"), Some(2005))],
    )
    .await;
    let router = waveflow_server::app(&config, state.clone());
    let bearer = token(&state, me, "mix-me").await;

    let (status, body) = get_json(&router, "/api/v2/mixes/never-played", &bearer).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(support::song_ids(&body["songs"]), [mine[0].to_string()]);
    let (status, _) = get_json(&router, "/api/v2/mixes/genre%3Agrime", &bearer).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = get_json(&router, "/api/v2/mixes/decade%3A2000", &bearer).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, body) = get_json(&router, "/api/v2/mixes", &bearer).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body
        .as_array()
        .unwrap()
        .iter()
        .all(|mix| mix["id"] != "genre:grime"));
    let (status, _) = get_json(&router, "/api/v2/mixes?limit=101", &bearer).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
}

// ── Similar songs without a provider: decision 8 ──

async fn drawn(state: &AppState, user: Uuid, id: Uuid) -> Vec<String> {
    let mut names: Vec<String> = state
        .services
        .similar_songs(user, id, 50)
        .await
        .unwrap()
        .into_iter()
        .map(|song| song.artist.unwrap_or_default())
        .collect();
    names.sort();
    names.dedup();
    names
}

async fn artist_named(state: &AppState, user: Uuid, name: &str) -> Uuid {
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

/// A shared genre makes a neighbour; an artist sharing nothing does not,
/// even of the same era; and an artist in another account's library is never
/// one, whatever it shares.
#[tokio::test]
async fn a_shared_genre_makes_a_local_neighbour_and_nothing_foreign_does() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "similar-me").await;
    let other = account(&state, "similar-other").await;
    library(
        &state,
        &config,
        me,
        "similar-me",
        &[
            file("Hyperballad", "Björk", Some("Electronic"), Some(1995)),
            // No year, so its score is the genre alone — the score each
            // foreign artist below would have.
            file("Svefn-g-englar", "Sigur Rós", Some("Electronic"), None),
            file("Enter Sandman", "Metallica", Some("Metal"), Some(1995)),
        ],
    )
    .await;
    // Twenty artists sharing the genre, in a library the caller is not in,
    // named to sort before Sigur Rós: counted as neighbours, they would take
    // every place and push the visible one out of the draw.
    let foreign: Vec<File> = (0..20)
        .map(|n| file("Elsewhere", &format!("Aa {n:02}"), Some("Electronic"), None))
        .collect();
    library(&state, &config, other, "similar-other", &foreign).await;
    let bjork = artist_named(&state, me, "Björk").await;

    assert_eq!(drawn(&state, me, bjork).await, ["Björk", "Sigur Rós"]);
}

/// A co-credit makes a neighbour without a shared genre, and so does
/// listening to an artist straight after the seed.
#[tokio::test]
async fn a_co_credit_and_a_co_listen_make_local_neighbours() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "similar-credit").await;
    let mut duet = file("Nattura", "Björk", None, None);
    duet.artists = vec!["Björk".into(), "Thom Yorke".into()];
    let (_, ids) = library(
        &state,
        &config,
        me,
        "similar-credit",
        &[
            file("Joga", "Björk", None, None),
            duet,
            file("Clair de lune", "Debussy", None, None),
            file("Unrelated", "Nobody Near", None, None),
            file("Analyse", "Thom Yorke", None, None),
        ],
    )
    .await;
    let bjork = artist_named(&state, me, "Björk").await;
    // Thom Yorke's own track is drawn because of the duet; Debussy and
    // Nobody Near share nothing with Björk yet.
    assert_eq!(
        drawn(&state, me, bjork).await,
        ["Björk", "Björk, Thom Yorke", "Thom Yorke"]
    );

    // Debussy straight after Björk, twice; Nobody Near a day later.
    listen(&state, me, ids[0], 60 * MINUTE).await;
    listen(&state, me, ids[2], 55 * MINUTE).await;
    listen(&state, me, ids[0], 20 * MINUTE).await;
    listen(&state, me, ids[2], 15 * MINUTE).await;
    listen(&state, me, ids[3], 2 * DAY).await;
    let names = drawn(&state, me, bjork).await;
    assert!(names.contains(&"Debussy".to_owned()), "{names:?}");
    assert!(!names.contains(&"Nobody Near".to_owned()), "{names:?}");
}

/// The façade end to end with no provider: both methods answer songs where
/// they used to answer an empty container, and the native route is the same
/// draw.
#[tokio::test]
async fn the_similar_songs_methods_answer_without_a_provider() {
    let (_temp, config, state) = support::test_app().await;
    let me = account(&state, "similar-route").await;
    let other = account(&state, "similar-route-other").await;
    library(
        &state,
        &config,
        me,
        "similar-route",
        &[
            file("Hyperballad", "Björk", Some("Electronic"), Some(1995)),
            file(
                "Svefn-g-englar",
                "Sigur Rós",
                Some("Electronic"),
                Some(1999),
            ),
        ],
    )
    .await;
    library(
        &state,
        &config,
        other,
        "similar-route-other",
        &[file("Far", "Far Away", None, None)],
    )
    .await;
    let bjork = artist_named(&state, me, "Björk").await;
    let far = artist_named(&state, other, "Far Away").await;
    let router = waveflow_server::app(&config, state.clone());
    let api_key = subsonic_key(&state, me).await;
    let bearer = token(&state, me, "similar-route").await;

    for (method, container) in [
        ("getSimilarSongs", "similarSongs"),
        ("getSimilarSongs2", "similarSongs2"),
    ] {
        let body = support::subsonic_json(&router, method, &api_key, &format!("&id={bjork}")).await;
        let songs = body["subsonic-response"][container]["song"]
            .as_array()
            .unwrap_or_else(|| panic!("an array: {body}"));
        assert_eq!(songs.len(), 2, "{method}");
    }
    let (status, body) = get_json(
        &router,
        &format!("/api/v2/similar-songs/{bjork}?limit=1"),
        &bearer,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1);
    let (status, _) = get_json(&router, &format!("/api/v2/similar-songs/{far}"), &bearer).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
