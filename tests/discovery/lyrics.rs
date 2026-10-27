//! Lyrics looked up on LRCLIB, against a fake of it: when a track is asked
//! for, what is cached, and what is never asked.

use super::*;

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

/// An unsynced line is an object with a `value`, as a synced one is.
///
/// OpenSubsonic's `line` is `{start?, value}`. The JSON renderer turns an
/// element that holds only text into a bare string, which is right for
/// `moods` or `isrc` and wrong here: a line with no `start` has nothing but its
/// text, so a plain sheet went out as `"line": ["…"]`, which a client decoding
/// the documented shape refuses whole. No campaign met it — the trap catalogue
/// had no unsynced lyrics until 2026-10-27.
#[tokio::test]
async fn an_unsynced_line_is_an_object_with_a_value() {
    let shelf = shelf().await;
    let router = waveflow_server::app(&shelf.config, shelf.state.clone());
    let api_key = subsonic_key(&shelf.state, shelf.owner).await;

    let by_id = support::subsonic_json(
        &router,
        "getLyricsBySongId",
        &api_key,
        &format!("&id={}", shelf.embedded),
    )
    .await;
    let sheets = by_id["subsonic-response"]["lyricsList"]["structuredLyrics"]
        .as_array()
        .unwrap_or_else(|| panic!("a list: {by_id}"));
    assert_eq!(sheets.len(), 1, "{by_id}");
    assert_eq!(sheets[0]["synced"], false, "{by_id}");
    assert_eq!(
        sheets[0]["line"],
        serde_json::json!([{ "value": "We live on a mountain" }]),
        "{by_id}"
    );
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
