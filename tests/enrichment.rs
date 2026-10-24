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
    portrait::AddressPolicy,
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
        state.artwork_dir.clone(),
        Endpoints {
            lastfm: listener.base_url.clone(),
            ..Endpoints::default()
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
        state.artwork_dir.clone(),
        Endpoints {
            lastfm: "http://127.0.0.1:1/2.0/".to_owned(),
            ..Endpoints::default()
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

/// An artist's albums ask no provider; the artist's page does.
///
/// `GET /api/v2/artists/{id}/albums` exists so an album page can list the
/// artist's other records without the enrichment the artist page carries. The
/// page is read here too, against the same engine, so a provider that is never
/// asked is the engine working and not a fake that cannot be reached.
#[tokio::test]
async fn an_artists_albums_ask_no_provider() {
    let (_temp, _config, state, owner, _outsider) = two_members().await;
    let listener = fake_lastfm(Mode::Hit, Duration::ZERO).await;
    let enrichment = engine(
        &state,
        &settings(vec![Provider::LastFm], &run_key()),
        &listener,
    );
    state.services.use_enrichment(enrichment.clone());
    let bjork = artist_id(&state, owner, "Björk").await;

    let albums = state.services.artist(owner, bjork).await.unwrap();
    enrichment.idle().await;
    assert!(!albums.albums.is_empty());
    assert!(albums.enrichment.is_none());
    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 0);

    state.services.artist_page(owner, bjork).await.unwrap();
    enrichment.idle().await;
    assert_eq!(listener.fake.info.load(Ordering::SeqCst), 1);
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

struct DeezerListener {
    fake: Deezer,
    base_url: String,
}

/// A JPEG by its first bytes, which is all the fetcher checks; the rest is
/// made up for the run so two runs never share a hash.
fn portrait_bytes() -> Vec<u8> {
    let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0];
    bytes.extend_from_slice(run_key().as_bytes());
    bytes
}

async fn deezer_search(State(fake): State<Deezer>) -> Response {
    fake.searches.fetch_add(1, Ordering::SeqCst);
    json(
        serde_json::json!({"data": [
            {"id": 1, "name": "Björk", "picture_xl": *fake.picture}
        ]})
        .to_string(),
    )
}

async fn deezer_picture(State(fake): State<Deezer>) -> Response {
    fake.images.fetch_add(1, Ordering::SeqCst);
    (
        [(header::CONTENT_TYPE, "image/jpeg")],
        (*fake.bytes).clone(),
    )
        .into_response()
}

async fn fake_deezer(picture_host: &str) -> DeezerListener {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = Deezer {
        searches: Arc::default(),
        images: Arc::default(),
        picture: Arc::new(format!("http://{picture_host}:{port}/img/portrait.jpg")),
        bytes: Arc::new(portrait_bytes()),
    };
    let router = Router::new()
        .route("/search/artist", get(deezer_search))
        .route("/img/portrait.jpg", get(deezer_picture))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    DeezerListener {
        fake,
        base_url: format!("http://127.0.0.1:{port}"),
    }
}

/// An engine asking the fake Deezer, fetching pictures from `hosts` under
/// `policy`.
fn deezer_engine(
    state: &waveflow_server::AppState,
    settings: &MetadataSettings,
    listener: &DeezerListener,
    hosts: &[&str],
    policy: AddressPolicy,
) -> Enrichment {
    Enrichment::with_endpoints(
        state.db.clone(),
        settings,
        state.artwork_dir.clone(),
        Endpoints {
            deezer: listener.base_url.clone(),
            deezer_image_hosts: hosts.iter().map(|host| (*host).to_owned()).collect(),
            image_addresses: policy,
            ..Endpoints::default()
        },
    )
    .unwrap()
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

async fn portrait_of(state: &waveflow_server::AppState, user: uuid::Uuid) -> Option<String> {
    let bjork = artist_id(state, user, "Björk").await;
    state
        .services
        .artist(user, bjork)
        .await
        .unwrap()
        .artist
        .artwork_hash
}

/// Decision 8 end to end: the picture is fetched once, stored in the artwork
/// store under its hash, and every artist row with the name shows it — one
/// event for each, in its own library.
#[tokio::test]
async fn a_portrait_is_fetched_once_and_shown_on_every_artist_with_the_name() {
    let (_temp, state, owner, other) = bjork_twice().await;
    let listener = fake_deezer("127.0.0.1").await;
    let enrichment = deezer_engine(
        &state,
        &settings(vec![Provider::Deezer], &run_key()),
        &listener,
        &["127.0.0.1"],
        AddressPolicy::AllowLoopback,
    );
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    let mine = portrait_of(&state, owner).await;
    let hash = mine.clone().expect("the portrait is linked");
    assert_eq!(portrait_of(&state, other).await, mine);
    assert!(state.artwork_dir.join(format!("{hash}.jpg")).is_file());
    let source: String = sqlx::query_scalar("SELECT source FROM artwork WHERE hash = ?")
        .bind(&hash)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(source, "deezer");
    assert_eq!(artist_events(&state).await, 2);
    assert_eq!(listener.fake.images.load(Ordering::SeqCst), 1);

    // Decision 11: the native page names where the portrait came from.
    state.services.use_enrichment(enrichment);
    let bjork = artist_id(&state, owner, "Björk").await;
    let enriched = state
        .services
        .artist_page(owner, bjork)
        .await
        .unwrap()
        .enrichment
        .expect("the native page always carries it");
    let image = enriched.image.expect("the portrait is published");
    assert!(image.artwork_hash == hash, "another portrait was published");
    assert_eq!(image.source.provider, "deezer");
    assert_eq!(
        enriched.state,
        waveflow_server::services::EnrichmentState::Ready
    );
}

/// A picture URL naming a host off the allowlist is dropped without being
/// contacted, and the answer is cached as a miss rather than retried.
#[tokio::test]
async fn a_host_outside_the_allowlist_is_never_contacted() {
    let (_temp, state, owner, _other) = bjork_twice().await;
    let listener = fake_deezer("127.0.0.1").await;
    let enrichment = deezer_engine(
        &state,
        &settings(vec![Provider::Deezer], &run_key()),
        &listener,
        &["localhost"],
        AddressPolicy::AllowLoopback,
    );
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    assert_eq!(listener.fake.searches.load(Ordering::SeqCst), 1);
    assert_eq!(listener.fake.images.load(Ordering::SeqCst), 0);
    let rows = enrichment.artist("Björk").await.unwrap();
    assert_eq!(rows[0].outcome, Outcome::Miss);
    assert_eq!(portrait_of(&state, owner).await, None);
}

/// Production's policy, with the fake's own host allowlisted: still never
/// contacted, because it is on loopback.
#[tokio::test]
async fn the_default_policy_does_not_reach_a_loopback_host() {
    let (_temp, state, _owner, _other) = bjork_twice().await;
    let listener = fake_deezer("127.0.0.1").await;
    let enrichment = deezer_engine(
        &state,
        &settings(vec![Provider::Deezer], &run_key()),
        &listener,
        &["127.0.0.1"],
        AddressPolicy::default(),
    );
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    assert_eq!(listener.fake.searches.load(Ordering::SeqCst), 1);
    assert_eq!(listener.fake.images.load(Ordering::SeqCst), 0);
}

/// The artwork row, the cache row and the artist link are one transaction:
/// when the link cannot be written, neither is the artwork row — nothing is
/// left for the sweeper to find unreferenced, and nothing claims an image.
#[tokio::test]
async fn the_artwork_row_and_the_hash_land_together_or_not_at_all() {
    let (_temp, state, _owner, _other) = bjork_twice().await;
    sqlx::query(
        "CREATE TRIGGER refuse_portrait BEFORE UPDATE OF artwork_hash ON artist \
         BEGIN SELECT RAISE(ABORT, 'refused by the test'); END",
    )
    .execute(state.db.pool())
    .await
    .unwrap();
    let listener = fake_deezer("127.0.0.1").await;
    let enrichment = deezer_engine(
        &state,
        &settings(vec![Provider::Deezer], &run_key()),
        &listener,
        &["127.0.0.1"],
        AddressPolicy::AllowLoopback,
    );
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    assert_eq!(listener.fake.images.load(Ordering::SeqCst), 1);
    let artwork: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork WHERE source = 'deezer'")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let cached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM metadata_artist")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!((artwork, cached), (0, 0));
}

/// RFC-007 decision 6's rule: an event only when `artwork_hash` moved. The
/// row expires at once, so the second visit fetches the same picture again,
/// and says nothing about it.
#[tokio::test]
async fn an_unchanged_portrait_emits_no_event() {
    let (_temp, state, _owner, _other) = bjork_twice().await;
    let listener = fake_deezer("127.0.0.1").await;
    let enrichment = deezer_engine(
        &state,
        &MetadataSettings {
            ttl: Duration::ZERO,
            ..settings(vec![Provider::Deezer], &run_key())
        },
        &listener,
        &["127.0.0.1"],
        AddressPolicy::AllowLoopback,
    );
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    assert_eq!(artist_events(&state).await, 2);
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    assert_eq!(listener.fake.images.load(Ordering::SeqCst), 2);
    assert_eq!(artist_events(&state).await, 2);
}

/// An artist row that lost its portrait — re-identified by a PID spec change,
/// or scanned since — is linked again from the cache on the next visit,
/// without a second download.
#[tokio::test]
async fn a_cached_portrait_is_linked_again_without_a_second_download() {
    let (_temp, state, owner, _other) = bjork_twice().await;
    let listener = fake_deezer("127.0.0.1").await;
    let enrichment = deezer_engine(
        &state,
        &settings(vec![Provider::Deezer], &run_key()),
        &listener,
        &["127.0.0.1"],
        AddressPolicy::AllowLoopback,
    );
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    let linked = portrait_of(&state, owner).await;
    sqlx::query("UPDATE artist SET artwork_hash = NULL")
        .execute(state.db.pool())
        .await
        .unwrap();

    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    assert!(linked.is_some());
    assert_eq!(portrait_of(&state, owner).await, linked);
    assert_eq!(listener.fake.images.load(Ordering::SeqCst), 1);
}

// ── Portrait candidates: a Deezer search the server previews (RFC-013) ──

use waveflow_server::services::ServiceError;

/// A fake Deezer that offers, for any query, twenty-four artists with a
/// picture of their own each — and two it cannot show among them: one with
/// Deezer's placeholder, one whose picture is not an image.
#[derive(Clone)]
struct Gallery {
    searches: Arc<AtomicUsize>,
    images: Arc<AtomicUsize>,
    /// Pictures asked at any size but Deezer's big one.
    other_sizes: Arc<AtomicUsize>,
    /// A search waits here for a permit. A test that holds a search in
    /// flight starts it with none and adds them when it is done holding.
    gate: Arc<tokio::sync::Semaphore>,
    /// Every picture answers 503: a host that cannot be reached.
    failing: bool,
    origin: Arc<String>,
}

/// The fake's ids, in the order its search lists them.
const PLACEHOLDER_ID: i64 = 900;
const NOT_AN_IMAGE_ID: i64 = 901;

async fn gallery_search(State(gallery): State<Gallery>) -> Response {
    gallery.searches.fetch_add(1, Ordering::SeqCst);
    let _permit = gallery.gate.acquire().await.unwrap();
    let origin = &*gallery.origin;
    let mut data = Vec::new();
    for id in 1..=24_i64 {
        if id == 3 {
            data.push(serde_json::json!({
                "id": PLACEHOLDER_ID,
                "name": "Nobody",
                "picture_big": format!("{origin}/images/artist//500x500-000000-80-0-0.jpg"),
            }));
        }
        if id == 5 {
            data.push(serde_json::json!({
                "id": NOT_AN_IMAGE_ID,
                "name": "Broken",
                "picture_big": format!("{origin}/img/{NOT_AN_IMAGE_ID}/big.jpg"),
            }));
        }
        data.push(serde_json::json!({
            "id": id,
            "name": format!("Björk {id}"),
            "nb_fan": id * 1000,
            "picture_small": format!("{origin}/img/{id}/small.jpg"),
            "picture_big": format!("{origin}/img/{id}/big.jpg"),
            "picture_xl": format!("{origin}/img/{id}/xl.jpg"),
        }));
    }
    json(serde_json::json!({ "data": data }).to_string())
}

async fn gallery_picture(
    State(gallery): State<Gallery>,
    axum::extract::Path((id, size)): axum::extract::Path<(i64, String)>,
) -> Response {
    gallery.images.fetch_add(1, Ordering::SeqCst);
    if size != "big.jpg" {
        gallery.other_sizes.fetch_add(1, Ordering::SeqCst);
    }
    if gallery.failing {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let bytes = if id == NOT_AN_IMAGE_ID {
        b"no picture here".to_vec()
    } else {
        let mut bytes = portrait_bytes();
        bytes.extend_from_slice(format!("-{id}").as_bytes());
        bytes
    };
    ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response()
}

async fn fake_gallery(failing: bool, open: bool) -> (Gallery, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let gallery = Gallery {
        searches: Arc::default(),
        images: Arc::default(),
        other_sizes: Arc::default(),
        gate: Arc::new(tokio::sync::Semaphore::new(if open { 1_000 } else { 0 })),
        failing,
        origin: Arc::new(base_url.clone()),
    };
    let router = Router::new()
        .route("/search/artist", get(gallery_search))
        .route("/img/{id}/{size}", get(gallery_picture))
        .with_state(gallery.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (gallery, base_url)
}

fn gallery_engine(
    state: &waveflow_server::AppState,
    base_url: &str,
    settings: &MetadataSettings,
) -> Enrichment {
    Enrichment::with_endpoints(
        state.db.clone(),
        settings,
        state.artwork_dir.clone(),
        Endpoints {
            deezer: base_url.to_owned(),
            deezer_image_hosts: vec!["127.0.0.1".to_owned()],
            image_addresses: AddressPolicy::AllowLoopback,
            ..Endpoints::default()
        },
    )
    .unwrap()
}

/// Björk and Sigur Rós in a library of the owner's, and the services asking
/// `gallery` for portraits.
async fn picker(
    failing: bool,
    open: bool,
) -> (
    tempfile::TempDir,
    waveflow_server::Config,
    waveflow_server::AppState,
    Gallery,
    uuid::Uuid,
    uuid::Uuid,
) {
    let (temp, config, state) = support::test_app().await;
    let owner = account(&state, "picker-owner").await;
    library_of(&state, &config, owner, "picked", &["Björk", "Sigur Rós"]).await;
    let (gallery, base_url) = fake_gallery(failing, open).await;
    state.services.use_enrichment(gallery_engine(
        &state,
        &base_url,
        &settings(vec![Provider::Deezer], &run_key()),
    ));
    let bjork = artist_id(&state, owner, "Björk").await;
    (temp, config, state, gallery, owner, bjork)
}

async fn searches_held(state: &waveflow_server::AppState) -> Vec<(i64, Option<i64>)> {
    sqlx::query_as("SELECT complete, expires_at FROM artist_image_search ORDER BY searched_at")
        .fetch_all(state.db.pool())
        .await
        .unwrap()
}

/// Waits, a few seconds at most, for the fake to have been asked `count`
/// searches.
async fn until_searched(gallery: &Gallery, count: usize) {
    for _ in 0..500 {
        if gallery.searches.load(Ordering::SeqCst) >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the fake was never asked");
}

async fn send(
    router: &Router,
    method: axum::http::Method,
    uri: String,
    token: &str,
    body: Option<serde_json::Value>,
) -> Response {
    use tower::ServiceExt;
    let request = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"));
    let request = match body {
        Some(body) => request
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string())),
        None => request.body(axum::body::Body::empty()),
    };
    router.clone().oneshot(request.unwrap()).await.unwrap()
}

/// RFC-013 decision 4 through the router: twenty candidates in Deezer's order,
/// each previewed from a picture the server fetched at the big size and holds,
/// and no picture URL; the same query answered again from the stored search;
/// the preview served to owners and managers only; a pick accepted only for an
/// id a live search of that artist offered; and both routes closed the moment
/// Deezer is. Passwords are drawn at run time, for CodeQL.
#[tokio::test]
async fn candidates_are_previewed_by_the_server_and_picked_from_a_live_search() {
    use axum::http::Method;
    use waveflow_server::database::{AccountRole, LibraryRole};

    let (_temp, config, state, gallery, owner, bjork) = picker(false, true).await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = waveflow_server::security::hash_password(&password).unwrap();
    let now = waveflow_server::authentication::now_ms();
    sqlx::query("UPDATE account SET username='gallery-owner', password_hash=? WHERE id=?")
        .bind(&hash)
        .bind(owner.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let listener = state
        .db
        .create_account("gallery-listener", &hash, AccountRole::User, now)
        .await
        .unwrap();
    state
        .db
        .create_account("gallery-stranger", &hash, AccountRole::User, now)
        .await
        .unwrap();
    let library: String = sqlx::query_scalar("SELECT id FROM library WHERE name='picked'")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library.parse().unwrap(),
            listener,
            LibraryRole::Listener,
            now,
        )
        .await
        .unwrap();
    let sigur = artist_id(&state, owner, "Sigur Rós").await;
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = support::login_token(&router, "gallery-owner", &password).await;
    let listener_token = support::login_token(&router, "gallery-listener", &password).await;
    let stranger_token = support::login_token(&router, "gallery-stranger", &password).await;
    let candidates_uri = |artist: uuid::Uuid, query: &str| {
        format!(
            "/api/v2/artists/{artist}/image/candidates?q={}",
            query.replace(' ', "%20")
        )
    };
    let pick_uri = |artist: uuid::Uuid| format!("/api/v2/artists/{artist}/image/portrait/deezer");

    let offered = send(
        &router,
        Method::GET,
        candidates_uri(bjork, "Björk"),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(offered.status(), StatusCode::OK);
    let offered = support::json_body(offered).await;
    let offered = offered.as_array().unwrap().clone();
    let ids: Vec<i64> = offered
        .iter()
        .map(|candidate| candidate["deezer_id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, (1..=20).collect::<Vec<_>>());
    assert_eq!(offered[0]["name"], "Björk 1");
    assert_eq!(offered[0]["fans"], 1000);
    for candidate in &offered {
        let mut keys: Vec<_> = candidate.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        assert_eq!(keys, ["artwork_hash", "deezer_id", "fans", "name"]);
    }
    // Twenty previews and the picture that was not an image; the placeholder
    // never asked; every picture at the big size.
    assert_eq!(gallery.searches.load(Ordering::SeqCst), 1);
    assert_eq!(gallery.images.load(Ordering::SeqCst), 21);
    assert_eq!(gallery.other_sizes.load(Ordering::SeqCst), 0);

    let preview = offered[0]["artwork_hash"].as_str().unwrap().to_owned();
    let artwork_uri = format!("/api/v2/artwork/{preview}");
    let served = send(
        &router,
        Method::GET,
        artwork_uri.clone(),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(served.status(), StatusCode::OK);
    let refused = send(&router, Method::GET, artwork_uri, &listener_token, None).await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);

    let again = send(
        &router,
        Method::GET,
        candidates_uri(bjork, " BJÖRK "),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(again.status(), StatusCode::OK);
    assert_eq!(
        support::json_body(again).await.as_array().unwrap(),
        &offered
    );
    assert_eq!(gallery.searches.load(Ordering::SeqCst), 1);
    assert_eq!(gallery.images.load(Ordering::SeqCst), 21);

    for token in [&listener_token, &stranger_token] {
        let refused = send(
            &router,
            Method::GET,
            candidates_uri(bjork, "Björk"),
            token,
            None,
        )
        .await;
        assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    }
    let empty = send(
        &router,
        Method::GET,
        candidates_uri(bjork, "  "),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(empty.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let pick = |id: i64| Some(serde_json::json!({ "deezer_id": id }));
    // Offered, but for another artist; never offered; not the listener's.
    let elsewhere = send(&router, Method::PUT, pick_uri(sigur), &owner_token, pick(3)).await;
    assert_eq!(elsewhere.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unoffered = send(
        &router,
        Method::PUT,
        pick_uri(bjork),
        &owner_token,
        pick(PLACEHOLDER_ID),
    )
    .await;
    assert_eq!(unoffered.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let listened = send(
        &router,
        Method::PUT,
        pick_uri(bjork),
        &listener_token,
        pick(3),
    )
    .await;
    assert_eq!(listened.status(), StatusCode::NOT_FOUND);

    let picked = send(&router, Method::PUT, pick_uri(bjork), &owner_token, pick(3)).await;
    assert_eq!(picked.status(), StatusCode::OK);
    let picked = support::json_body(picked).await;
    assert_eq!(picked["artwork_hash"], offered[2]["artwork_hash"]);
    assert_eq!(picked["image_choice"]["portrait"]["source"], "deezer");
    assert_eq!(gallery.images.load(Ordering::SeqCst), 21);

    // The operator's switch is read at every call: a live search notwithstanding.
    state.services.use_enrichment(Enrichment::disabled());
    let closed = send(
        &router,
        Method::GET,
        candidates_uri(bjork, "Björk"),
        &owner_token,
        None,
    )
    .await;
    assert_eq!(closed.status(), StatusCode::NOT_FOUND);
    let closed = send(&router, Method::PUT, pick_uri(bjork), &owner_token, pick(4)).await;
    assert_eq!(closed.status(), StatusCode::NOT_FOUND);
}

/// One search at a time per account: a second while the first is fetching
/// answers 429, and the next one after it starts at once.
#[tokio::test]
async fn a_second_search_while_the_first_fetches_is_refused() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(false, false).await;
    let services = state.services.clone();
    let first = tokio::spawn(async move {
        services
            .artist_image_candidates(owner, bjork, "Björk")
            .await
    });
    until_searched(&gallery, 1).await;

    let second = state
        .services
        .artist_image_candidates(owner, bjork, "Sigur Rós")
        .await;
    assert!(matches!(second, Err(ServiceError::Busy)), "{second:?}");

    gallery.gate.add_permits(1_000);
    assert_eq!(first.await.unwrap().unwrap().len(), 20);
    let third = state
        .services
        .artist_image_candidates(owner, bjork, "Sigur Rós")
        .await;
    assert!(third.is_ok(), "{third:?}");
}

/// The fetching is not the request's: a request dropped mid-search — a closed
/// tab — leaves the task to complete the search, and the next picker that
/// asks the same query is answered from it.
#[tokio::test]
async fn a_search_whose_request_is_dropped_still_completes() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(false, false).await;
    let services = state.services.clone();
    let request = tokio::spawn(async move {
        services
            .artist_image_candidates(owner, bjork, "Björk")
            .await
    });
    until_searched(&gallery, 1).await;
    request.abort();
    assert!(request.await.unwrap_err().is_cancelled());

    gallery.gate.add_permits(1_000);
    let mut completed = false;
    for _ in 0..500 {
        if searches_held(&state).await.first().map(|search| search.0) == Some(1) {
            completed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(completed, "the search was left incomplete");

    let again = state
        .services
        .artist_image_candidates(owner, bjork, "björk")
        .await
        .unwrap();
    assert_eq!(again.len(), 20);
    assert_eq!(gallery.searches.load(Ordering::SeqCst), 1);
}

/// A search whose previews cannot be fetched is deleted by its own task, and
/// so does not hold the account's next search at 429.
#[tokio::test]
async fn a_failed_search_is_deleted_and_does_not_hold_the_account() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(true, true).await;
    for _ in 0..2 {
        let failed = state
            .services
            .artist_image_candidates(owner, bjork, "Björk")
            .await;
        assert!(
            matches!(failed, Err(ServiceError::Unavailable)),
            "{failed:?}"
        );
        assert!(searches_held(&state).await.is_empty());
    }
    assert_eq!(gallery.searches.load(Ordering::SeqCst), 2);
}

/// The task runs under its deadline as a whole: a Deezer that never answers
/// fails the search at the deadline, not at the request timeout, and the
/// search is deleted. The deadline is cut to a second here and the request
/// timeout set to a minute, so which of the two stopped it shows in the time
/// the search took.
#[tokio::test]
async fn a_search_stops_at_its_deadline() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(false, false).await;
    let patient = MetadataSettings {
        request_timeout: Duration::from_secs(60),
        ..settings(vec![Provider::Deezer], &run_key())
    };
    state
        .services
        .use_enrichment(gallery_engine(&state, gallery.origin.as_str(), &patient));
    state
        .services
        .shorten_image_search_deadline(Duration::from_secs(1));
    let started = std::time::Instant::now();
    let stopped = state
        .services
        .artist_image_candidates(owner, bjork, "Björk")
        .await;
    let waited = started.elapsed();
    assert!(
        matches!(stopped, Err(ServiceError::Unavailable)),
        "{stopped:?}"
    );
    assert!(waited < Duration::from_secs(30), "stopped after {waited:?}");
    assert!(searches_held(&state).await.is_empty());
}

/// An incomplete search past its deadline has no task behind it: it holds
/// nobody at 429, and the sweep deletes it — while a search still fetching,
/// and the previews of a live one, are left alone. Once the live search
/// expires, its previews go with the sweep, all but the one chosen.
#[tokio::test]
async fn the_sweep_takes_spent_searches_and_their_previews_only() {
    let (_temp, _config, state, _gallery, owner, bjork) = picker(false, true).await;
    let now = waveflow_server::authentication::now_ms();
    let stale = uuid::Uuid::new_v4().to_string();
    let fetching = uuid::Uuid::new_v4().to_string();
    for (id, by, at) in [
        (&stale, owner.to_string(), now - 6 * 60 * 1000),
        (&fetching, uuid::Uuid::new_v4().to_string(), now),
    ] {
        sqlx::query(
            "INSERT INTO artist_image_search (id, artist_id, query_key, searched_by, searched_at) \
             VALUES (?, ?, 'stale', ?, ?)",
        )
        .bind(id)
        .bind(bjork.to_string())
        .bind(by)
        .bind(at)
        .execute(state.db.pool())
        .await
        .unwrap();
    }

    let offered = state
        .services
        .artist_image_candidates(owner, bjork, "Björk")
        .await
        .expect("a search past its deadline holds nobody");
    let previews: Vec<String> = offered
        .iter()
        .map(|candidate| candidate.artwork_hash.clone())
        .collect();
    let held = |state: &waveflow_server::AppState, previews: Vec<String>| {
        let pool = state.db.pool().clone();
        async move {
            let mut count = 0;
            for hash in previews {
                let found: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork WHERE hash=?")
                    .bind(hash)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
                count += found;
            }
            count
        }
    };

    state.services.sweep_artwork_store().await.unwrap();
    let ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM artist_image_search WHERE complete=0")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert_eq!(ids, std::slice::from_ref(&fetching));
    assert_eq!(held(&state, previews.clone()).await, 20);

    state
        .services
        .choose_deezer_portrait(owner, bjork, 2)
        .await
        .unwrap();
    sqlx::query("UPDATE artist_image_search SET expires_at=? WHERE complete=1")
        .bind(now - 1)
        .execute(state.db.pool())
        .await
        .unwrap();
    let late = state.services.choose_deezer_portrait(owner, bjork, 3).await;
    assert!(matches!(late, Err(ServiceError::Invalid)), "{late:?}");

    state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(searches_held(&state).await, [(0, None)]);
    assert_eq!(held(&state, previews.clone()).await, 1);
    assert_eq!(
        held(&state, vec![offered[1].artwork_hash.clone()]).await,
        1,
        "the chosen preview went with the search"
    );
}

/// Nothing can be fetching for a search across a restart: boot deletes every
/// incomplete one and keeps the complete ones.
#[tokio::test]
async fn boot_deletes_the_searches_left_fetching() {
    let (_temp, config, state, _gallery, owner, bjork) = picker(false, true).await;
    let now = waveflow_server::authentication::now_ms();
    for (complete, expires_at) in [(0, None), (1, Some(now + 60 * 60 * 1000))] {
        sqlx::query(
            "INSERT INTO artist_image_search \
               (id, artist_id, query_key, searched_by, searched_at, expires_at, complete) \
             VALUES (?, ?, 'björk', ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(bjork.to_string())
        .bind(owner.to_string())
        .bind(now)
        .bind(expires_at)
        .bind(complete)
        .execute(state.db.pool())
        .await
        .unwrap();
    }

    waveflow_server::initialize(&config).await.unwrap();

    let held = searches_held(&state).await;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].0, 1);
}

/// Every request to Deezer keeps enrichment's pace, the previews as much as
/// the search: at twenty a second, a search and its twenty-one picture
/// requests cannot take less than a second.
#[tokio::test]
async fn previews_keep_deezer_s_pace() {
    let (_temp, _config, state, gallery, owner, bjork) = picker(false, true).await;
    let paced = MetadataSettings {
        rate_per_sec: 20,
        ..settings(vec![Provider::Deezer], &run_key())
    };
    state
        .services
        .use_enrichment(gallery_engine(&state, gallery.origin.as_str(), &paced));
    let started = std::time::Instant::now();
    state
        .services
        .artist_image_candidates(owner, bjork, "Björk")
        .await
        .unwrap();
    assert_eq!(gallery.images.load(Ordering::SeqCst), 21);
    assert!(
        started.elapsed() >= Duration::from_secs(1),
        "twenty-two requests in {:?}",
        started.elapsed()
    );
}

/// A search outlives the ordinary request timeout: the route keeps a timeout
/// of its own past the search's deadline, so a slow Deezer answers the
/// request with what the task found, not a 408 that sends the retry into a
/// 429. The ordinary timeout is cut to a second here, and the search held for
/// two. Passwords are drawn at run time, for CodeQL.
#[tokio::test]
async fn a_search_outlives_the_ordinary_request_timeout() {
    let (_temp, config, state, gallery, owner, bjork) = picker(false, false).await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = waveflow_server::security::hash_password(&password).unwrap();
    sqlx::query("UPDATE account SET username='slow-owner', password_hash=? WHERE id=?")
        .bind(&hash)
        .bind(owner.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    // Logged in under the ordinary timeout: hashing the password can take
    // longer than a second on a loaded machine, and that is not what is tested.
    let token = support::login_token(
        &waveflow_server::app(&config, state.clone()),
        "slow-owner",
        &password,
    )
    .await;
    let impatient = waveflow_server::Config {
        request_timeout: Duration::from_secs(1),
        ..config.clone()
    };
    let router = waveflow_server::app(&impatient, state.clone());

    let search = {
        let router = router.clone();
        tokio::spawn(async move {
            send(
                &router,
                axum::http::Method::GET,
                format!("/api/v2/artists/{bjork}/image/candidates?q=Bj%C3%B6rk"),
                &token,
                None,
            )
            .await
        })
    };
    until_searched(&gallery, 1).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    gallery.gate.add_permits(1_000);

    let answered = search.await.unwrap();
    assert_eq!(answered.status(), StatusCode::OK);
    assert_eq!(
        support::json_body(answered).await.as_array().unwrap().len(),
        20
    );
}

// ── Banners: TheAudioDB's fanart (RFC-014) ──

/// What the fake TheAudioDB answers to a search.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fanarts {
    /// Björk, with four fanarts: one on a host off the allowlist, one that is
    /// not an image, then two real ones.
    Four,
    /// Björk, with one real fanart.
    One,
    /// `{"artists": null}`: TheAudioDB knows nobody by that name.
    Nobody,
    /// A `429`, as TheAudioDB answers a key over its ceiling.
    Limited,
}

#[derive(Clone)]
struct AudioDb {
    mode: Arc<Mutex<Fanarts>>,
    searches: Arc<AtomicUsize>,
    images: Arc<AtomicUsize>,
    /// The key each search carried in its path.
    keys: Arc<Mutex<Vec<String>>>,
    port: u16,
    /// Made up for the run, so two runs never share a hash.
    token: Arc<String>,
}

impl AudioDb {
    /// The bytes the fake serves for the picture called `name`: a JPEG by its
    /// first bytes, which is all the fetcher checks.
    fn picture(&self, name: &str) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0];
        bytes.extend_from_slice(self.token.as_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes
    }
}

async fn audiodb_search(
    State(fake): State<AudioDb>,
    axum::extract::Path(key): axum::extract::Path<String>,
) -> Response {
    fake.searches.fetch_add(1, Ordering::SeqCst);
    fake.keys.lock().unwrap().push(key);
    let port = fake.port;
    let picture = |name: &str| format!("http://127.0.0.1:{port}/img/{name}.jpg");
    let artist = match *fake.mode.lock().unwrap() {
        Fanarts::Limited => return StatusCode::TOO_MANY_REQUESTS.into_response(),
        Fanarts::Nobody => return json(r#"{"artists":null}"#.to_owned()),
        Fanarts::Four => serde_json::json!({
            "strArtist": "Björk",
            "strArtistFanart": format!("http://localhost:{port}/img/elsewhere.jpg"),
            "strArtistFanart2": picture("broken"),
            "strArtistFanart3": picture("second"),
            "strArtistFanart4": picture("third"),
            "strArtistBanner": picture("strip"),
        }),
        Fanarts::One => serde_json::json!({
            "strArtist": "Björk",
            "strArtistFanart": picture("only"),
        }),
    };
    json(serde_json::json!({ "artists": [artist] }).to_string())
}

async fn audiodb_picture(
    State(fake): State<AudioDb>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Response {
    fake.images.fetch_add(1, Ordering::SeqCst);
    let bytes = if name == "broken.jpg" {
        b"no picture here".to_vec()
    } else {
        fake.picture(&name)
    };
    ([(header::CONTENT_TYPE, "image/jpeg")], bytes).into_response()
}

async fn fake_audiodb(mode: Fanarts) -> (AudioDb, String) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let fake = AudioDb {
        mode: Arc::new(Mutex::new(mode)),
        searches: Arc::default(),
        images: Arc::default(),
        keys: Arc::default(),
        port,
        token: Arc::new(run_key()),
    };
    let router = Router::new()
        .route("/{key}/search.php", get(audiodb_search))
        .route("/img/{name}", get(audiodb_picture))
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (fake, format!("http://127.0.0.1:{port}"))
}

fn audiodb_settings(key: Option<&str>) -> MetadataSettings {
    MetadataSettings {
        providers: vec![Provider::TheAudioDb],
        theaudiodb_key: key.map(str::to_owned),
        ..settings(Vec::new(), &run_key())
    }
}

fn audiodb_engine(
    state: &waveflow_server::AppState,
    base_url: &str,
    settings: &MetadataSettings,
) -> anyhow::Result<Enrichment> {
    Enrichment::with_endpoints(
        state.db.clone(),
        settings,
        state.artwork_dir.clone(),
        Endpoints {
            theaudiodb: base_url.to_owned(),
            theaudiodb_image_hosts: vec!["127.0.0.1".to_owned()],
            image_addresses: AddressPolicy::AllowLoopback,
            ..Endpoints::default()
        },
    )
}

async fn fanarts_of(state: &waveflow_server::AppState) -> Vec<Option<String>> {
    sqlx::query_scalar("SELECT fanart_hash FROM artist WHERE name = 'Björk' ORDER BY id")
        .fetch_all(state.db.pool())
        .await
        .unwrap()
}

async fn artwork_rows(state: &waveflow_server::AppState, hash: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM artwork WHERE hash = ?")
        .bind(hash)
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

/// Asks again: the cache row is made stale, the artist visited, the fill
/// awaited.
async fn refill(state: &waveflow_server::AppState, enrichment: &Enrichment) {
    sqlx::query("UPDATE metadata_artist SET expires_at = 0")
        .execute(state.db.pool())
        .await
        .unwrap();
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
}

/// RFC-014 decisions 1 to 5 end to end. The operator's key is the one sent;
/// the fanarts are tried in order, the one off the allowlist never contacted
/// and the one that is not an image passed over, and the first real one is
/// linked on every artist with the name, one event each; the strip TheAudioDB
/// calls a banner is never asked. The artist page shows it as the banner and
/// credits TheAudioDB, the artwork route serves it to a member, and the
/// requests keep TheAudioDB's own pace however fast the shared rate is.
#[tokio::test]
async fn a_banner_is_the_first_real_fanart_and_is_shown_on_every_artist() {
    let (_temp, state, owner, other) = bjork_twice().await;
    let (fake, base_url) = fake_audiodb(Fanarts::Four).await;
    let key = run_key();
    let enrichment = audiodb_engine(&state, &base_url, &audiodb_settings(Some(&key))).unwrap();
    let started = std::time::Instant::now();
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    let took = started.elapsed();

    assert_eq!(*fake.keys.lock().unwrap(), [key]);
    // The one that is not an image and the first real one; neither the host
    // off the allowlist, nor the third, nor the strip.
    assert_eq!(fake.images.load(Ordering::SeqCst), 2);
    // Three requests at one every two seconds, the shared rate notwithstanding.
    assert!(took >= Duration::from_secs(4), "three requests in {took:?}");
    let linked = fanarts_of(&state).await;
    assert_eq!(linked.len(), 2);
    let hash = linked[0].clone().expect("the banner is linked");
    assert_eq!(linked[1].as_deref(), Some(hash.as_str()));
    assert_eq!(
        hash,
        waveflow_server::portrait::Portrait::from_bytes(fake.picture("second.jpg"))
            .unwrap()
            .hash
    );
    assert!(state.artwork_dir.join(format!("{hash}.jpg")).is_file());
    assert_eq!(artist_events(&state).await, 2);

    state.services.use_enrichment(enrichment);
    let bjork = artist_id(&state, owner, "Björk").await;
    let page = state.services.artist_page(owner, bjork).await.unwrap();
    assert_eq!(page.banner_hash.as_deref(), Some(hash.as_str()));
    let banner = page
        .enrichment
        .expect("the native page always carries it")
        .banner
        .expect("the automatic banner is credited");
    assert_eq!(banner.artwork_hash, hash);
    assert_eq!(banner.source.provider, "theaudiodb");
    assert!(state
        .services
        .artwork_for_user(other, &hash)
        .await
        .unwrap()
        .is_some());
}

/// A banner chosen by hand wins, and is not credited to a provider; removing
/// it brings TheAudioDB's back (RFC-014 decision 4, amending RFC-013's
/// decision 5).
#[tokio::test]
async fn a_chosen_banner_wins_and_the_fanart_comes_back_when_it_goes() {
    let (_temp, state, owner, _other) = bjork_twice().await;
    let (_fake, base_url) = fake_audiodb(Fanarts::One).await;
    let enrichment =
        audiodb_engine(&state, &base_url, &audiodb_settings(Some(&run_key()))).unwrap();
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    state.services.use_enrichment(enrichment);
    let bjork = artist_id(&state, owner, "Björk").await;
    let fanart = fanarts_of(&state).await[0].clone().unwrap();

    let mut chosen = portrait_bytes();
    chosen.extend_from_slice(b"chosen");
    state
        .services
        .choose_artist_image(
            owner,
            bjork,
            waveflow_server::services::ArtistImageKind::Banner,
            chosen,
        )
        .await
        .unwrap();
    let page = state.services.artist_page(owner, bjork).await.unwrap();
    assert_ne!(page.banner_hash.as_deref(), Some(fanart.as_str()));
    assert!(page.banner_hash.is_some());
    assert!(page.enrichment.unwrap().banner.is_none());

    state
        .services
        .remove_artist_image(
            owner,
            bjork,
            waveflow_server::services::ArtistImageKind::Banner,
        )
        .await
        .unwrap();
    let page = state.services.artist_page(owner, bjork).await.unwrap();
    assert_eq!(page.banner_hash.as_deref(), Some(fanart.as_str()));
    assert!(page.enrichment.unwrap().banner.is_some());
}

/// RFC-014 decisions 3 and 6: the sweep keeps a linked banner. A refresh that
/// cannot reach TheAudioDB — a 429 — unlinks nothing; one that comes back a
/// miss unlinks it from every artist, one event each, and the sweep then
/// takes its row.
#[tokio::test]
async fn a_miss_unlinks_the_banner_and_an_unreached_refresh_does_not() {
    let (_temp, state, _owner, _other) = bjork_twice().await;
    let (fake, base_url) = fake_audiodb(Fanarts::One).await;
    let enrichment =
        audiodb_engine(&state, &base_url, &audiodb_settings(Some(&run_key()))).unwrap();
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    let fanart = fanarts_of(&state).await[0].clone().unwrap();
    state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(artwork_rows(&state, &fanart).await, 1);
    assert_eq!(artist_events(&state).await, 2);

    *fake.mode.lock().unwrap() = Fanarts::Limited;
    refill(&state, &enrichment).await;
    assert_eq!(
        fanarts_of(&state).await,
        [Some(fanart.clone()), Some(fanart.clone())]
    );
    assert_eq!(artist_events(&state).await, 2);

    *fake.mode.lock().unwrap() = Fanarts::Nobody;
    refill(&state, &enrichment).await;
    assert_eq!(fanarts_of(&state).await, [None, None]);
    assert_eq!(artist_events(&state).await, 4);
    state.services.sweep_artwork_store().await.unwrap();
    assert_eq!(artwork_rows(&state, &fanart).await, 0);
    assert_eq!(fake.searches.load(Ordering::SeqCst), 3);
}

/// RFC-014 decision 1: TheAudioDB is not built without the operator's key,
/// nor with images off, since banners are all it is asked for.
#[tokio::test]
async fn theaudiodb_needs_its_key_and_images_on() {
    let (_temp, _config, state) = support::test_app().await;
    let (fake, base_url) = fake_audiodb(Fanarts::One).await;
    assert!(audiodb_engine(&state, &base_url, &audiodb_settings(None)).is_err());

    let off = MetadataSettings {
        images: false,
        ..audiodb_settings(Some(&run_key()))
    };
    let enrichment = audiodb_engine(&state, &base_url, &off).unwrap();
    assert!(!enrichment.is_enabled());
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    assert_eq!(fake.searches.load(Ordering::SeqCst), 0);
}

/// An artist row that lost its banner — re-identified, or scanned since — is
/// linked again from the cache on the next visit, without a second download,
/// as a portrait is.
#[tokio::test]
async fn a_cached_banner_is_linked_again_without_a_second_download() {
    let (_temp, state, _owner, _other) = bjork_twice().await;
    let (fake, base_url) = fake_audiodb(Fanarts::One).await;
    let enrichment =
        audiodb_engine(&state, &base_url, &audiodb_settings(Some(&run_key()))).unwrap();
    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;
    let linked = fanarts_of(&state).await;
    sqlx::query("UPDATE artist SET fanart_hash = NULL")
        .execute(state.db.pool())
        .await
        .unwrap();

    enrichment.artist("Björk").await.unwrap();
    enrichment.idle().await;

    assert!(linked[0].is_some());
    assert_eq!(fanarts_of(&state).await, linked);
    assert_eq!(fake.images.load(Ordering::SeqCst), 1);
}
