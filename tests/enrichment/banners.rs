//! Artist banners fetched from TheAudioDB.

use super::*;

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
        waveflow_server::remote::portrait::Portrait::from_bytes(fake.picture("second.jpg"))
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
    // The same rule from the other entry point, the one getArtistInfo reads.
    let enrichment = state
        .services
        .artist_enrichment(owner, bjork, 20)
        .await
        .unwrap();
    assert!(enrichment.banner.is_none());

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
