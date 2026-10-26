//! Artist information and similar artists, as members of a library see them.

use super::*;

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
