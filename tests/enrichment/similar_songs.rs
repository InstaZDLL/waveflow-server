//! Similar songs: the draw, what it may reach, and how it is shared out.

use super::*;

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
///
/// Before the provider answers, the neighbours are the catalogue's own
/// (RFC-024 decision 8). Here those share nothing with Björk — Sigur Rós's
/// track loses the genre the fixture gives every track — so the artist is
/// drawn alone until Last.fm names Sigur Rós.
#[tokio::test]
async fn the_artist_s_own_tracks_are_in_the_draw() {
    let (_temp, _config, state, owner, _outsider) = two_members().await;
    sqlx::query(
        "DELETE FROM track_genre WHERE track_id IN \
           (SELECT id FROM track WHERE artist_display='Sigur Rós')",
    )
    .execute(state.db.pool())
    .await
    .unwrap();
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

/// The façade end to end: `count` absent draws the specification's fifty,
/// the track list is an array, and a foreign id is error 70 — enabled, and
/// since RFC-024 disabled too, where the catalogue's own neighbours stand in
/// for a provider's and the empty container these methods used to answer is
/// gone.
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
        let songs = off["subsonic-response"][container]["song"]
            .as_array()
            .unwrap_or_else(|| panic!("an array: {off}"));
        assert_eq!(songs.len(), 50, "{method}");
        let refused =
            support::subsonic_json(&router, method, &api_key, &format!("&id={mum}")).await;
        assert_eq!(
            refused["subsonic-response"]["error"]["code"], 70,
            "{refused}"
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
