//! An artist's most played, from local history, on both surfaces.

use super::*;

/// An artist's most played tracks, on both surfaces, from local history alone.
///
/// `getTopSongs` answered an empty container because the reference server fills
/// it from a remote service WaveFlow does not query. `play_event` can answer a
/// narrower question with the same shape — what *this account* played — and one
/// service answers it for the native route and the facade both.
///
/// Four things are asserted that a discography sorted by anything would also
/// satisfy, and which pin this to the history instead:
/// - a track nobody played is absent, rather than last;
/// - the counting is per account, so a second member's listening moves its own
///   list and not the owner's;
/// - a guest track counts for the album's artist, which is what the artist page
///   lists albums by;
/// - another artist's heavily played track stays out.
#[tokio::test]
async fn an_artists_most_played_comes_from_local_history_on_both_surfaces() {
    let (_temp, config, state) = test_app().await;
    let now = now_ms();
    // Inserted rather than created: `create_account` wants a password hash, and
    // this account authenticates by API key and bearer token only.
    let account = |id: Uuid, username: &'static str| {
        let state = &state;
        async move {
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
        }
    };
    let owner = Uuid::new_v4();
    let listener = Uuid::new_v4();
    let stranger = Uuid::new_v4();
    account(owner, "lamp-owner").await;
    account(listener, "lamp-listener").await;
    account(stranger, "lamp-stranger").await;

    let api_key = security::generate_token("wfsk_");
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"lamp-secret").unwrap(),
            &security::token_hash(&api_key),
            now,
        )
        .await
        .unwrap();

    let music = config.data_dir.join("lamp-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Lamp",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now,
        )
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library,
            listener,
            waveflow_server::storage::database::LibraryRole::Listener,
            now,
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 5, false).await.unwrap();
    for (index, title, album, credited) in [
        (80, "Lamp Hymn", "Lamp Songs", "Harbour Bells"),
        (81, "Lamp Echo", "Lamp Songs", "Harbour Bells"),
        (82, "Lamp Silence", "Lamp Songs", "Harbour Bells"),
        // Credited to someone else on the file, on an album that is Harbour
        // Bells': the album-artist half of the question.
        (83, "Guest Turn", "Lamp Guests", "Visiting Horn"),
        (84, "Other Tide", "Other Shore", "Rival Keepers"),
    ] {
        let mut input = browse_input(index, title, album, credited, Some(1), Some(1));
        if title == "Guest Turn" {
            input.album_artist = Some("Harbour Bells".into());
        }
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

    let songs = state
        .services
        .list_songs(owner, &Default::default())
        .await
        .unwrap();
    let track = |title: &str| {
        songs
            .iter()
            .find(|song| song.title == title)
            .unwrap_or_else(|| panic!("{title} was indexed"))
            .id
    };
    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let artist = |name: &str| {
        artists
            .iter()
            .find(|summary| summary.artist.name == name)
            .unwrap_or_else(|| panic!("{name} was indexed"))
            .artist
            .id
    };

    // The history. "Lamp Silence" is played by nobody on purpose, and "Other
    // Tide" is played more than anything else by another artist.
    for (user, title, count) in [
        (owner, "Lamp Hymn", 3),
        (owner, "Guest Turn", 2),
        (owner, "Lamp Echo", 1),
        (owner, "Other Tide", 9),
        (listener, "Lamp Echo", 4),
    ] {
        for _ in 0..count {
            state
                .services
                .scrobble(user, track(title), true, None)
                .await
                .unwrap();
        }
    }
    // A play that was never submitted is a now-playing ping, and the
    // projections count submissions only.
    state
        .services
        .scrobble(owner, track("Lamp Silence"), false, None)
        .await
        .unwrap();

    let bells = artist("Harbour Bells");
    let (_record, owner_token) = state
        .services
        .create_api_token(owner, &[], "lamp-owner", "owner reader", &[])
        .await
        .unwrap();
    let (_record, listener_token) = state
        .services
        .create_api_token(listener, &[], "lamp-listener", "listener reader", &[])
        .await
        .unwrap();
    let (_record, stranger_token) = state
        .services
        .create_api_token(stranger, &[], "lamp-stranger", "stranger reader", &[])
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state);

    let native = |uri: String, token: String| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            (status, json_body(response).await)
        }
    };
    let titles_and_counts = |value: &serde_json::Value| {
        value
            .as_array()
            .expect("the route answers a list")
            .iter()
            .map(|song| {
                (
                    song["title"].as_str().unwrap().to_owned(),
                    song["play_count"].as_i64().unwrap(),
                )
            })
            .collect::<Vec<_>>()
    };

    let (status, body) = native(
        format!("/api/v2/artists/{bells}/top-songs"),
        owner_token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        titles_and_counts(&body),
        [
            ("Lamp Hymn".to_owned(), 3),
            ("Guest Turn".to_owned(), 2),
            ("Lamp Echo".to_owned(), 1),
        ],
        "most played first, the unplayed track absent, and the guest counted"
    );

    // The same question from the other member of the same library. Same
    // catalogue, different history, different answer — which is what makes the
    // list above the owner's rather than the instance's.
    let (status, body) = native(
        format!("/api/v2/artists/{bells}/top-songs"),
        listener_token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        titles_and_counts(&body),
        [("Lamp Echo".to_owned(), 4)],
        "a second member sees its own listening"
    );

    let (_status, body) = native(
        format!("/api/v2/artists/{bells}/top-songs?limit=1"),
        owner_token.clone(),
    )
    .await;
    assert_eq!(titles_and_counts(&body), [("Lamp Hymn".to_owned(), 3)]);

    // The catalogue's bounds, as every other listing applies them.
    for limit in ["0", "501"] {
        let (status, _body) = native(
            format!("/api/v2/artists/{bells}/top-songs?limit={limit}"),
            owner_token.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "limit={limit}");
    }

    // An artist nobody can reach, and one that does not exist, answer the same
    // way: 404 either way, so neither confirms the identifier.
    let (status, _body) = native(
        format!("/api/v2/artists/{bells}/top-songs"),
        stranger_token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "a non-member is refused");
    let (status, _body) = native(
        format!("/api/v2/artists/{}/top-songs", Uuid::new_v4()),
        owner_token.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And the facade, which asks by name.
    let facade = |extra: String| {
        let router = router.clone();
        let api_key = api_key.clone();
        async move {
            let body = subsonic_json(&router, "getTopSongs", &api_key, &extra).await;
            body["subsonic-response"]["topSongs"].clone()
        }
    };
    let facade_titles = |value: &serde_json::Value| {
        value["song"]
            .as_array()
            .map(|songs| {
                songs
                    .iter()
                    .map(|song| song["title"].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };

    let answered = facade("&artist=Harbour%20Bells".into()).await;
    assert_eq!(
        facade_titles(&answered),
        ["Lamp Hymn", "Guest Turn", "Lamp Echo"],
        "the facade answers what the native route does"
    );
    assert_eq!(
        answered["song"][0]["playCount"].as_i64(),
        Some(3),
        "and carries the count it ordered by"
    );

    // Matched on the canonical name, the one folding rule the rest of the
    // catalogue uses: a client echoing a name back finds the artist whatever
    // the punctuation and case did on the way.
    let folded = facade("&artist=harbour---BELLS".into()).await;
    assert_eq!(
        facade_titles(&folded),
        ["Lamp Hymn", "Guest Turn", "Lamp Echo"]
    );

    let one = facade("&artist=Harbour%20Bells&count=1".into()).await;
    // Named rather than implied. A single child is the one case that catches
    // the container collapsing back into an object, and `facade_titles` reads
    // the shape through `as_array().unwrap_or_default()` — so a regression would
    // fail on a list of titles and say nothing about the frozen rule it broke.
    assert!(one["song"].is_array(), "topSongs.song is a list: {one}");
    assert_eq!(facade_titles(&one), ["Lamp Hymn"], "{one}");

    // A page of nothing is a valid request here, as it is for getAlbumList.
    let none = facade("&artist=Harbour%20Bells&count=0".into()).await;
    assert!(none.is_object() && none.get("song").is_none());

    // A name is not an identifier: an unknown one is an empty list, not a
    // refusal, because nothing about the catalogue is revealed by it.
    let unknown = facade("&artist=Nobody%20In%20This%20Library".into()).await;
    assert!(unknown.is_object() && unknown.get("song").is_none());

    // The parameter itself is still required.
    let bare = subsonic_json(&router, "getTopSongs", &api_key, "").await;
    assert_eq!(
        bare["subsonic-response"]["error"]["code"].as_i64(),
        Some(10),
        "a missing artist is a malformed request"
    );

    // The two neighbours this change left empty are filled since RFC-024 by
    // the catalogue's own neighbours, and are not a history: what they draw
    // is the artist and its neighbours, not this account's most played.
    for method in ["getSimilarSongs", "getSimilarSongs2"] {
        let body = subsonic_json(
            &router,
            method,
            &api_key,
            &format!("&id={}", track("Lamp Hymn")),
        )
        .await;
        let container = &body["subsonic-response"][match method {
            "getSimilarSongs" => "similarSongs",
            _ => "similarSongs2",
        }];
        let drawn = container["song"]
            .as_array()
            .unwrap_or_else(|| panic!("{method} draws songs: {body}"));
        assert!(
            drawn.iter().any(|song| song["title"] == "Lamp Silence"),
            "{method} draws the artist's unplayed track too: {body}"
        );
    }
}
