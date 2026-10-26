//! Local neighbours, and the similar-songs methods that answer from them.

use super::*;

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
