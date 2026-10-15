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

/// Pacing refuses no request, so the ceiling on fills in flight is what bounds
/// the queue: one artist past it is not asked, and its next visit will be.
#[tokio::test]
async fn no_more_than_the_ceiling_of_fills_is_ever_queued() {
    let (_temp, _config, state) = support::test_app().await;
    // Slow enough that every fill is still out when the last visit lands.
    let listener = fake_lastfm(Mode::Hit, Duration::from_secs(1)).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );

    let names: Vec<String> = (0..=waveflow_server::enrichment::MAX_IN_FLIGHT)
        .map(|index| format!("Artist {index}"))
        .collect();
    let visits = names.iter().map(|name| enrichment.artist(name));
    for answer in futures_util::future::join_all(visits).await {
        answer.unwrap();
    }
    enrichment.idle().await;

    assert_eq!(
        listener.fake.info.load(Ordering::SeqCst),
        waveflow_server::enrichment::MAX_IN_FLIGHT
    );
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
    // One artist per mode: asked about the same one, the second engine would
    // find the first one's fresh row and send nothing — and the 503 checked
    // below would then only be found if another test happened to log one.
    for (mode, artist) in [(Mode::Hit, "Björk"), (Mode::Unavailable, "Múm")] {
        let listener = fake_lastfm(mode, Duration::ZERO).await;
        let enrichment = engine(&state, &settings, &listener);
        enrichment.artist(artist).await.unwrap();
        enrichment.idle().await;
        assert_eq!(listener.fake.info.load(Ordering::SeqCst), 1, "{artist}");
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

// ── The readers: getArtistInfo, getArtistInfo2 and the native artist page ──
//
// Accounts are inserted as rows rather than through `create_account`: these
// tests need user ids and libraries, not a login, and a password hashed from a
// literal is what makes CodeQL flag a test. They reach HTTP through an API
// token and a Subsonic key minted at run time.

async fn account(state: &waveflow_server::AppState, username: &str) -> uuid::Uuid {
    let id = uuid::Uuid::new_v4();
    let now = waveflow_server::authentication::now_ms();
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
    let now = waveflow_server::authentication::now_ms();
    let root = config.data_dir.join(name);
    std::fs::create_dir_all(&root).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            name,
            &std::fs::canonicalize(&root).unwrap(),
            waveflow_server::database::LibraryVisibility::Private,
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

/// Decision 7: a neighbour is an artist the caller can see, or nothing. Múm
/// is in Last.fm's list and in this instance, in a library the owner is not a
/// member of; returning it would hand out an id `getArtist` then refuses,
/// having already confirmed the row exists.
#[tokio::test]
async fn a_similar_artist_living_only_in_another_members_library_is_not_returned() {
    let (_temp, _config, state, owner, _outsider) = two_members().await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;

    state
        .services
        .artist_enrichment(owner, bjork, 20)
        .await
        .unwrap();
    enrichment.idle().await;
    let answer = state
        .services
        .artist_enrichment(owner, bjork, 20)
        .await
        .unwrap();

    let similar = answer.similar.expect("a neighbour the owner can see");
    let names: Vec<_> = similar
        .artists
        .iter()
        .map(|artist| artist.artist.name.as_str())
        .collect();
    assert_eq!(names, ["Sigur Rós"]);
    assert_eq!(similar.source.provider, "lastfm");
}

/// The lookup comes first and for its refusal: an artist in another member's
/// library answers as a missing one, and no provider is asked about it.
#[tokio::test]
async fn a_foreign_artist_id_answers_as_a_missing_one() {
    let (_temp, _config, state, owner, outsider) = two_members().await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let mum = artist_id(&state, outsider, "Múm").await;

    let refused = state.services.artist_enrichment(owner, mum, 20).await;
    let missing = state
        .services
        .artist_enrichment(owner, uuid::Uuid::new_v4(), 20)
        .await;
    enrichment.idle().await;

    assert!(matches!(
        refused,
        Err(waveflow_server::services::ServiceError::NotFound)
    ));
    assert!(matches!(
        missing,
        Err(waveflow_server::services::ServiceError::NotFound)
    ));
    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 0);
}

/// Decision 11's four states, each where it holds, through the native page —
/// the one surface that publishes them.
#[tokio::test]
async fn each_of_the_four_states_appears_when_it_should() {
    use waveflow_server::services::EnrichmentState;

    let (_temp, _config, state, owner, _outsider) = two_members().await;
    let bjork = artist_id(&state, owner, "Björk").await;
    let state_now = |state: &waveflow_server::AppState| {
        let services = state.services.clone();
        async move {
            services
                .artist_page(owner, bjork)
                .await
                .unwrap()
                .enrichment
                .expect("the native page always carries it")
                .state
        }
    };

    // Nothing configured: the server's default.
    assert_eq!(state_now(&state).await, EnrichmentState::Disabled);

    // Configured, first visit: scheduled, not waited for.
    let found = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &found,
    );
    state.services.use_enrichment(enrichment.clone());
    assert_eq!(state_now(&state).await, EnrichmentState::Pending);
    enrichment.idle().await;
    assert_eq!(state_now(&state).await, EnrichmentState::Ready);

    // A provider that knows nothing about an artist: `none`, not absence.
    let unknown = fake_lastfm(Mode::Miss, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &unknown,
    );
    state.services.use_enrichment(enrichment.clone());
    let sigur = artist_id(&state, owner, "Sigur Rós").await;
    let page = |state: &waveflow_server::AppState| {
        let services = state.services.clone();
        async move {
            services
                .artist_page(owner, sigur)
                .await
                .unwrap()
                .enrichment
                .unwrap()
        }
    };
    assert_eq!(page(&state).await.state, EnrichmentState::Pending);
    enrichment.idle().await;
    let none = page(&state).await;
    assert_eq!(none.state, EnrichmentState::None);
    assert!(none.bio.is_none() && none.similar.is_none());
}

/// An expired miss is being asked again, so it is `pending` rather than
/// `none`: the miss window here is zero, so the row is stale the moment it is
/// written, and the next visit schedules its refresh.
#[tokio::test]
async fn an_expired_miss_being_refreshed_is_pending() {
    use waveflow_server::services::EnrichmentState;

    let (_temp, _config, state, owner, _outsider) = two_members().await;
    let listener = fake_lastfm(Mode::Miss, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &MetadataSettings {
            miss_ttl: Duration::ZERO,
            ..settings(vec![Provider::LastFm], &run_key())
        },
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;

    state.services.artist_page(owner, bjork).await.unwrap();
    enrichment.idle().await;
    let refreshing = state
        .services
        .artist_page(owner, bjork)
        .await
        .unwrap()
        .enrichment
        .unwrap();
    enrichment.idle().await;

    assert_eq!(refreshing.state, EnrichmentState::Pending);
    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 2);
}

/// The façade end to end, in JSON: the first visit is the empty container it
/// always was; the next carries the biography, and the neighbours as an array
/// even when there is one of them.
#[tokio::test]
async fn get_artist_info_2_carries_the_biography_and_the_visible_neighbours() {
    let (_temp, config, state, owner, _outsider) = two_members().await;
    let api_key = subsonic_key(&state, owner).await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;
    let router = waveflow_server::app(&config, state.clone());

    let first =
        support::subsonic_json(&router, "getArtistInfo2", &api_key, &format!("&id={bjork}")).await;
    assert_eq!(
        first["subsonic-response"]["artistInfo2"],
        serde_json::json!({}),
        "{first}"
    );
    enrichment.idle().await;
    let next =
        support::subsonic_json(&router, "getArtistInfo2", &api_key, &format!("&id={bjork}")).await;

    let info = &next["subsonic-response"]["artistInfo2"];
    assert_eq!(info["biography"], "Icelandic singer.", "{next}");
    let similar = info["similarArtist"]
        .as_array()
        .unwrap_or_else(|| panic!("an array even with one entry: {next}"));
    assert_eq!(similar.len(), 1);
    assert_eq!(similar[0]["name"], "Sigur Rós");
    assert!(similar[0]["roles"].is_array(), "{next}");
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
            &waveflow_server::security::token_hash(&api_key),
            waveflow_server::authentication::now_ms(),
        )
        .await
        .unwrap();
    api_key
}

// ── The draw: getSimilarSongs and getSimilarSongs2 ──

/// The track artists of a draw, sorted, so a random order compares.
async fn drawn(state: &waveflow_server::AppState, user: uuid::Uuid, id: uuid::Uuid) -> Vec<String> {
    let mut names: Vec<_> = state
        .services
        .similar_songs(user, id, 50)
        .await
        .unwrap()
        .into_iter()
        .map(|song| song.artist.unwrap_or_default())
        .collect();
    names.sort();
    names
}

/// `getSimilarSongs` takes "the artist, album or song ID": an album stands for
/// its album artist and a track for its track artist, and both then draw what
/// the artist id draws.
#[tokio::test]
async fn an_album_id_and_a_song_id_reach_the_same_artist_as_the_artist_id() {
    let (_temp, _config, state, owner, _outsider) = two_members().await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;
    let album = state.services.artist(owner, bjork).await.unwrap().albums[0].id;
    let song = state.services.album(owner, album).await.unwrap().songs[0].id;
    drawn(&state, owner, bjork).await;
    enrichment.idle().await;

    for id in [bjork, album, song] {
        assert_eq!(drawn(&state, owner, id).await, ["Björk", "Sigur Rós"]);
    }
}

/// "Songs from the given artist *and* similar artists": the artist is in the
/// draw, before the neighbours are known and after.
#[tokio::test]
async fn the_artist_s_own_tracks_are_in_the_draw() {
    let (_temp, _config, state, owner, _outsider) = two_members().await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;

    assert_eq!(drawn(&state, owner, bjork).await, ["Björk"]);
    enrichment.idle().await;
    assert_eq!(drawn(&state, owner, bjork).await, ["Björk", "Sigur Rós"]);
}

/// A neighbour drawn by name draws only from the caller's libraries: Sigur
/// Rós is also in a library of the outsider's, and that track stays out. And
/// an id of the outsider's answers as a missing one, before any provider is
/// asked about it.
#[tokio::test]
async fn no_track_from_a_library_the_caller_is_not_in() {
    let (_temp, config, state, owner, outsider) = two_members().await;
    library_of(&state, &config, outsider, "elsewhere", &["Sigur Rós"]).await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;
    let mum = artist_id(&state, outsider, "Múm").await;
    let foreign_album = state.services.artist(outsider, mum).await.unwrap().albums[0].id;

    for id in [mum, foreign_album, uuid::Uuid::new_v4()] {
        assert!(matches!(
            state.services.similar_songs(owner, id, 50).await,
            Err(waveflow_server::services::ServiceError::NotFound)
        ));
    }
    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 0);
    drawn(&state, owner, bjork).await;
    enrichment.idle().await;

    let songs = state
        .services
        .similar_songs(owner, bjork, 50)
        .await
        .unwrap();
    let mine = state
        .services
        .list_artists(
            owner,
            None,
            waveflow_server::services::BrowsePage::new(None, None).unwrap(),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|artist| artist.artist.library_id)
        .collect::<Vec<_>>();
    assert_eq!(songs.len(), 2);
    assert!(songs.iter().all(|song| mine.contains(&song.library_id)));
}

/// The façade end to end: disabled, both methods are the empty container they
/// always were; enabled, `count` absent draws the specification's fifty, the
/// track list is an array, and a foreign id is error 70.
#[tokio::test]
async fn count_absent_yields_at_most_fifty() {
    let (_temp, config, state) = support::test_app().await;
    let owner = account(&state, "similar-owner").await;
    let outsider = account(&state, "similar-outsider").await;
    library_of(&state, &config, owner, "many", &["Björk"; 60]).await;
    library_of(&state, &config, outsider, "foreign", &["Múm"]).await;
    let api_key = subsonic_key(&state, owner).await;
    let bjork = artist_id(&state, owner, "Björk").await;
    let mum = artist_id(&state, outsider, "Múm").await;
    let router = waveflow_server::app(&config, state.clone());
    let id = format!("&id={bjork}");

    for (method, container) in [
        ("getSimilarSongs", "similarSongs"),
        ("getSimilarSongs2", "similarSongs2"),
    ] {
        let off = support::subsonic_json(&router, method, &api_key, &id).await;
        assert_eq!(
            off["subsonic-response"][container],
            serde_json::json!({}),
            "{off}"
        );
    }

    let listener = fake_lastfm(Mode::Miss, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    for (method, container) in [
        ("getSimilarSongs", "similarSongs"),
        ("getSimilarSongs2", "similarSongs2"),
    ] {
        let on = support::subsonic_json(&router, method, &api_key, &id).await;
        let songs = on["subsonic-response"][container]["song"]
            .as_array()
            .unwrap_or_else(|| panic!("an array: {on}"));
        assert_eq!(songs.len(), 50, "{method}");
        let one = support::subsonic_json(&router, method, &api_key, &format!("{id}&count=1")).await;
        assert_eq!(
            one["subsonic-response"][container]["song"]
                .as_array()
                .map(Vec::len),
            Some(1),
            "{one}"
        );
        let refused =
            support::subsonic_json(&router, method, &api_key, &format!("&id={mum}")).await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{refused}"
        );
    }
    enrichment.idle().await;
}

/// The draw is balanced across names, not over one pool of tracks: sixty of
/// Björk's beside five of Sigur Rós's, twenty drawn, and every one of Sigur
/// Rós's five is in — the turns they cannot take go to Björk. One pool would
/// draw about one and a half of them.
#[tokio::test]
async fn a_prolific_artist_does_not_crowd_out_the_neighbours() {
    let (_temp, config, state) = support::test_app().await;
    let owner = account(&state, "balanced-owner").await;
    library_of(&state, &config, owner, "prolific", &["Björk"; 60]).await;
    library_of(&state, &config, owner, "sparse", &["Sigur Rós"; 5]).await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;
    state.services.similar_songs(owner, bjork, 1).await.unwrap();
    enrichment.idle().await;

    let songs = state
        .services
        .similar_songs(owner, bjork, 20)
        .await
        .unwrap();
    let neighbours = songs
        .iter()
        .filter(|song| song.artist.as_deref() == Some("Sigur Rós"))
        .count();
    assert_eq!(songs.len(), 20);
    assert_eq!(neighbours, 5);
}

/// A track is filed under the name that put it in the pool, not under its
/// first track artist: ten tracks on Sigur Rós's album, each by a different
/// guest, are one neighbour's share. Filed by track artist they would be ten
/// names, each taking a turn before Björk's second, and a draw of four would
/// hold one of Björk's at most.
#[tokio::test]
async fn a_neighbour_s_album_of_guests_takes_one_share() {
    let (_temp, config, state) = support::test_app().await;
    let owner = account(&state, "guests-owner").await;
    library_of(&state, &config, owner, "prolific", &["Björk"; 60]).await;
    let guests: Vec<String> = (0..10).map(|n| format!("Guest {n}")).collect();
    let credits: Vec<_> = guests
        .iter()
        .map(|guest| (guest.as_str(), "Sigur Rós"))
        .collect();
    library_with(&state, &config, owner, "guests", &credits).await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;
    state.services.similar_songs(owner, bjork, 1).await.unwrap();
    enrichment.idle().await;

    let songs = state.services.similar_songs(owner, bjork, 4).await.unwrap();
    let own = songs
        .iter()
        .filter(|song| song.artist.as_deref() == Some("Björk"))
        .count();
    assert_eq!(songs.len(), 4);
    assert_eq!(own, 2);
}
