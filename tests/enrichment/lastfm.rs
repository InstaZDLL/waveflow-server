//! The Last.fm provider behind the engine: what is cached, what is asked, and
//! what is refused.

use super::*;

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
