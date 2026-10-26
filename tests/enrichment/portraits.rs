//! Artist portraits fetched from Deezer.

use super::*;

struct DeezerListener {
    fake: Deezer,
    base_url: String,
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
