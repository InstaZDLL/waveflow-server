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
    enrichment::{Endpoints, Enrichment, Outcome, Provider},
};

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
        Endpoints {
            lastfm: listener.base_url.clone(),
        },
    )
    .unwrap()
}

fn canonical(name: &str) -> String {
    waveflow_core::scanner::canonical_name(name)
}

/// Decision 3: the provider list is the switch. A key placed without a
/// provider naming it — the shape an operator who set up scrobbling first
/// leaves behind — is not consent, and asks nothing.
#[tokio::test]
async fn an_empty_provider_list_asks_nothing_even_with_a_key() {
    let (_temp, _config, state) = support::test_app().await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(&state, &settings(Vec::new(), &run_key()), &listener);

    assert!(!enrichment.is_enabled());
    assert!(enrichment.artist("Björk").await.unwrap().is_empty());
    enrichment.idle().await;

    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 0);
    assert_eq!(listener.fake.similar.load(Ordering::SeqCst), 0);
    assert!(state
        .db
        .metadata_artist_rows(&canonical("Björk"))
        .await
        .unwrap()
        .is_empty());
}

/// The ordinary path, and decision 6's cost stated as a test: the first visit
/// answers from an empty cache, the second has what came back.
#[tokio::test]
async fn a_hit_is_cached_with_its_biography_and_ranked_neighbours() {
    let (_temp, _config, state) = support::test_app().await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let settings = settings(vec![Provider::LastFm], &run_key());
    let enrichment = engine(&state, &settings, &listener);

    assert!(enrichment.artist("Björk").await.unwrap().is_empty());
    enrichment.idle().await;
    let rows = enrichment.artist("Björk").await.unwrap();

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.provider, Provider::LastFm);
    assert_eq!(row.outcome, Outcome::Hit);
    assert_eq!(row.bio_summary.as_deref(), Some("Icelandic singer."));
    assert_eq!(
        row.bio_full.as_deref(),
        Some("Icelandic singer, at length.")
    );
    assert_eq!(
        row.expires_at - row.fetched_at,
        settings.ttl.as_millis() as i64
    );
    let similar = state
        .db
        .metadata_artist_similar(Provider::LastFm, &canonical("Björk"))
        .await
        .unwrap();
    let names: Vec<_> = similar.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Sigur Rós", "Múm"]);
    assert_eq!(similar[0].name_canonical, canonical("Sigur Rós"));
    // A fresh row is served, not asked again.
    enrichment.idle().await;
    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 1);
}

/// Decision 5: "the provider knows nothing" is an answer, cached — for the
/// short window, because it is the answer most likely to change.
#[tokio::test]
async fn a_provider_that_knows_nothing_is_cached_as_a_miss_for_the_miss_ttl() {
    let (_temp, _config, state) = support::test_app().await;
    let listener = fake_lastfm(Mode::Miss, Duration::ZERO).await;
    let settings = settings(vec![Provider::LastFm], &run_key());
    let enrichment = engine(&state, &settings, &listener);

    enrichment.artist("Nobody Knows").await.unwrap();
    enrichment.idle().await;

    let rows = state
        .db
        .metadata_artist_rows(&canonical("Nobody Knows"))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, Outcome::Miss);
    assert_eq!(rows[0].bio_summary, None);
    assert_eq!(
        rows[0].expires_at - rows[0].fetched_at,
        settings.miss_ttl.as_millis() as i64,
        "a miss stands for the miss TTL, not the hit TTL"
    );
}

/// Decision 5's third outcome: a provider that could not be asked writes no
/// row, so the next visit asks again instead of reading the outage as an
/// artist without a biography.
#[tokio::test]
async fn a_provider_that_cannot_be_reached_writes_no_row() {
    let (_temp, _config, state) = support::test_app().await;
    let listener = fake_lastfm(Mode::Unavailable, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );

    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    assert!(state
        .db
        .metadata_artist_rows(&canonical("Björk"))
        .await
        .unwrap()
        .is_empty());

    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 2);
}

/// Decision 6: ten clients opening the same page are one request.
#[tokio::test]
async fn ten_concurrent_first_visits_make_one_request() {
    let (_temp, _config, state) = support::test_app().await;
    // Slow enough that every visit lands while the first fill is still out.
    let listener = fake_lastfm(Mode::Hit, Duration::from_millis(300)).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );

    let visits = (0..10).map(|_| enrichment.artist("Björk"));
    for answer in futures_util::future::join_all(visits).await {
        assert!(answer.unwrap().is_empty());
    }
    enrichment.idle().await;

    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 1);
    assert_eq!(listener.fake.similar.load(Ordering::SeqCst), 1);
}

/// Decision 9: no redirect is followed. Followed, this one would have been
/// cached as a hit from wherever the provider pointed.
#[tokio::test]
async fn a_redirect_from_the_provider_is_not_followed() {
    let (_temp, _config, state) = support::test_app().await;
    let listener = fake_lastfm(Mode::Redirect, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );

    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 1);
    assert_eq!(listener.fake.followed.load(Ordering::SeqCst), 0);
    assert!(state
        .db
        .metadata_artist_rows(&canonical("Björk"))
        .await
        .unwrap()
        .is_empty());
}

/// Decision 9: a response is bounded. Read whole, this one would have been
/// cached as a three-mebibyte biography.
#[tokio::test]
async fn an_answer_over_the_ceiling_is_refused() {
    let (_temp, _config, state) = support::test_app().await;
    let listener = fake_lastfm(Mode::Oversized, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );

    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 1);
    assert!(state
        .db
        .metadata_artist_rows(&canonical("Björk"))
        .await
        .unwrap()
        .is_empty());
}

/// A writer for the log, kept so the test can read it back.
#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The log of this whole test binary, at every level.
///
/// Global rather than scoped to one test: a subscriber installed with
/// `set_default` covers one thread, and tracing caches each callsite's interest
/// for the whole process — with the other tests running beside it, a scoped
/// capture kept two lines out of hundreds. Every test here writes into this
/// one; the assertion below looks for its own run's key, which no other test
/// knows.
fn captured_log() -> &'static Captured {
    static LOG: std::sync::OnceLock<Captured> = std::sync::OnceLock::new();
    LOG.get_or_init(|| {
        let captured = Captured::default();
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(captured.clone())
            .with_ansi(false)
            .init();
        captured
    })
}

/// Decision 9 and `CLAUDE.md`: the key travels in Last.fm's query string, and
/// must reach no log line — not on success, not on a refusal, not on a
/// connection that never opened — at any level, the HTTP stack's own included.
#[tokio::test]
async fn the_key_never_reaches_the_log() {
    let captured = captured_log();

    let (_temp, _config, state) = support::test_app().await;
    let key = run_key();
    let settings = settings(vec![Provider::LastFm], &key);
    for mode in [Mode::Hit, Mode::Unavailable] {
        let listener = fake_lastfm(mode, Duration::ZERO).await;
        let enrichment = engine(&state, &settings, &listener);
        enrichment.artist("Björk").await.unwrap();
        enrichment.idle().await;
    }
    // Nothing listens on port 1 of loopback: the connection itself fails.
    let closed = Enrichment::with_endpoints(
        state.db.clone(),
        &settings,
        Endpoints {
            lastfm: "http://127.0.0.1:1/2.0/".to_owned(),
        },
    )
    .unwrap();
    closed.artist("Sigur Rós").await.unwrap();
    closed.idle().await;

    let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    // The capture works: both failures were logged, by provider and verdict.
    // A closed port is refused on Linux and left to the connect timeout on
    // Windows, so the second verdict is one of two.
    assert!(log.contains("answered HTTP 503"), "{log}");
    assert!(
        log.contains("verdict=transport failure") || log.contains("verdict=timed out"),
        "{log}"
    );
    assert!(!log.contains(&key), "the key reached the log:\n{log}");
}
