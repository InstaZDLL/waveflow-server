//! Album discovery, ordered and filtered in SQL for both surfaces.

use super::*;

/// The ten Subsonic album-list modes and their native equivalents resolve
/// through one SQL implementation, so both surfaces agree by construction and
/// neither loads the catalogue to sort it.
#[tokio::test]
async fn album_discovery_orders_and_filters_in_sql_for_both_surfaces() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let api_key = "wfsk_discovery-key";
    let owner = state
        .db
        .create_account("discovery-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"discovery-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("discovery-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Discovery",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 5, false).await.unwrap();
    // "delta moon" is lowercase on purpose: a byte-wise sort would file it after
    // "Gamma Sun", and album order is documented as case-insensitive.
    for (index, (title, album, artist, genre, year)) in [
        ("Tidewater", "Alpha Sea", "Zed Waves", "Rock", 1999),
        ("Undertow", "Alpha Sea", "Zed Waves", "Rock", 1999),
        ("Cirrus", "Beta Sky", "Aria Lux", "Jazz; Rock", 2010),
        ("Corona", "Gamma Sun", "Mono Field", "Jazz", 2024),
        ("Waning", "delta moon", "Beta Person", "Hip-Hop", 2005),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(index + 40, title, album, artist, Some(1), Some(1));
        input.genre = Some(genre.into());
        input.year = Some(year);
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    // Albums created inside one scan share a millisecond, and the tie-break is a
    // random UUID. Pinning created_at is what makes `newest` assertable at all.
    for (title, created_at) in [
        ("Alpha Sea", 1_000_i64),
        ("Beta Sky", 2_000),
        ("Gamma Sun", 3_000),
        ("delta moon", 4_000),
    ] {
        sqlx::query("UPDATE album SET created_at = ? WHERE title = ?")
            .bind(created_at)
            .bind(title)
            .execute(state.db.pool())
            .await
            .unwrap();
    }

    let albums = state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap();
    let album_id = |title: &str| {
        albums
            .iter()
            .find(|album| album.title == title)
            .unwrap_or_else(|| panic!("{title} was indexed"))
            .id
    };

    state
        .services
        .set_rating(owner, "album", album_id("Gamma Sun"), 5)
        .await
        .unwrap();
    state
        .services
        .set_rating(owner, "album", album_id("Alpha Sea"), 3)
        .await
        .unwrap();
    state
        .services
        .set_star(owner, "album", album_id("Beta Sky"), true)
        .await
        .unwrap();
    let gamma_track = state
        .services
        .album(owner, album_id("Gamma Sun"))
        .await
        .unwrap()
        .songs[0]
        .id;
    let alpha_track = state
        .services
        .album(owner, album_id("Alpha Sea"))
        .await
        .unwrap()
        .songs[0]
        .id;
    for time in [1_000_i64, 2_000, 3_000] {
        state
            .services
            .scrobble(owner, gamma_track, true, Some(time))
            .await
            .unwrap();
    }
    // Played once but most recently: `frequent` and `recent` must not agree.
    state
        .services
        .scrobble(owner, alpha_track, true, Some(9_000))
        .await
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "discovery-owner", password).await;

    let subsonic_titles = |kind: String| {
        let router = router.clone();
        async move {
            let response = subsonic_json(&router, "getAlbumList2", api_key, &kind).await;
            response["subsonic-response"]["albumList2"]["album"]
                .as_array()
                .unwrap()
                .iter()
                .map(|album| album["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };
    let native_titles = |query: String| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(format!("/api/v2/albums?{query}"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{query}");
            json_body(response)
                .await
                .as_array()
                .unwrap()
                .iter()
                .map(|album| album["title"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };

    for (kind, native, expected) in [
        (
            "&type=alphabeticalByName&size=500",
            "sort=alphabeticalByName&limit=500",
            vec!["Alpha Sea", "Beta Sky", "delta moon", "Gamma Sun"],
        ),
        (
            "&type=alphabeticalByArtist&size=500",
            "sort=alphabeticalByArtist&limit=500",
            vec!["Beta Sky", "delta moon", "Gamma Sun", "Alpha Sea"],
        ),
        (
            "&type=newest&size=500",
            "sort=newest&limit=500",
            vec!["delta moon", "Gamma Sun", "Beta Sky", "Alpha Sea"],
        ),
        (
            "&type=highest&size=500",
            "sort=highest&limit=500",
            vec!["Gamma Sun", "Alpha Sea"],
        ),
        (
            "&type=frequent&size=500",
            "sort=frequent&limit=500",
            vec!["Gamma Sun", "Alpha Sea"],
        ),
        (
            "&type=recent&size=500",
            "sort=recent&limit=500",
            vec!["Alpha Sea", "Gamma Sun"],
        ),
        (
            "&type=starred&size=500",
            "sort=starred&limit=500",
            vec!["Beta Sky"],
        ),
        (
            "&type=byYear&fromYear=2000&toYear=2020&size=500",
            "sort=byYear&from_year=2000&to_year=2020&limit=500",
            vec!["delta moon", "Beta Sky"],
        ),
        (
            // A reversed range is how Subsonic asks for descending years.
            "&type=byYear&fromYear=2020&toYear=2000&size=500",
            "sort=byYear&from_year=2020&to_year=2000&limit=500",
            vec!["Beta Sky", "delta moon"],
        ),
        (
            "&type=byGenre&genre=Rock&size=500",
            "sort=byGenre&genre=Rock&limit=500",
            vec!["Alpha Sea", "Beta Sky"],
        ),
        (
            // Genre matching is on the canonical form, so punctuation and case
            // no longer split one genre in two.
            "&type=byGenre&genre=hip%20hop&size=500",
            "sort=byGenre&genre=hip+hop&limit=500",
            vec!["delta moon"],
        ),
    ] {
        assert_eq!(subsonic_titles(kind.into()).await, expected, "{kind}");
        assert_eq!(native_titles(native.into()).await, expected, "{native}");
    }

    // `random` draws from the same set as every other ordering. Its page
    // contents cannot be asserted: SQLite reshuffles per statement, so two
    // requests are two independent draws and a title may repeat or be missed
    // across them. What must hold is membership — no ordering may surface an
    // album the account cannot see.
    let catalogue = vec!["Alpha Sea", "Beta Sky", "Gamma Sun", "delta moon"];
    let mut shuffled = subsonic_titles("&type=random&size=500".into()).await;
    shuffled.sort();
    assert_eq!(shuffled, catalogue);
    for offset in [0, 2] {
        let page = subsonic_titles(format!("&type=random&size=2&offset={offset}")).await;
        assert!(page.len() <= 2, "offset {offset} returned {page:?}");
        for title in &page {
            assert!(
                catalogue.contains(&title.as_str()),
                "offset {offset} returned {title}"
            );
        }
    }

    // Paging happens in SQL now; the second page of an ordered list is exact.
    // Both surfaces are asserted because they reach `page` by different routes:
    // Subsonic clamps `size` before building it, the native handler maps
    // `offset`/`limit` straight onto `BrowsePage::new`.
    assert_eq!(
        subsonic_titles("&type=alphabeticalByName&size=2&offset=2".into()).await,
        vec!["delta moon", "Gamma Sun"]
    );
    assert_eq!(
        native_titles("sort=alphabeticalByName&limit=2&offset=2".into()).await,
        vec!["delta moon", "Gamma Sun"]
    );
    assert_eq!(
        native_titles("sort=newest&limit=1&offset=1".into()).await,
        vec!["Gamma Sun"]
    );

    // An empty page is where the two surfaces deliberately diverge. Subsonic
    // answered `size=0` with an empty container long before this change and
    // still does, while the native contract is `1 <= limit <= 500` and rejects
    // the bound like it rejects 501.
    let empty_page = subsonic_json(&router, "getAlbumList2", api_key, "&type=newest&size=0").await;
    let empty_page = &empty_page["subsonic-response"]["albumList2"];
    assert!(empty_page.is_object());
    assert!(empty_page.get("album").is_none());
    for query in ["sort=newest&limit=0", "sort=newest&limit=501"] {
        let out_of_bounds = router
            .clone()
            .oneshot(
                Request::get(format!("/api/v2/albums?{query}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            out_of_bounds.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query}"
        );
    }

    // An unknown ordering is refused on both surfaces rather than silently
    // falling back to the default.
    let rejected = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbumList2.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&type=nope"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(rejected).await["subsonic-response"]["error"]["code"],
        10
    );
    let rejected_native = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums?sort=nope")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected_native.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // byGenre without a genre would silently drop the filter if it were not
    // refused, so both surfaces reject it.
    let missing_genre = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums?sort=byGenre")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_genre.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let missing_genre_subsonic = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbumList2.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&type=byGenre"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_genre_subsonic.status(), StatusCode::OK);
    assert_eq!(
        json_body(missing_genre_subsonic).await["subsonic-response"]["error"]["code"],
        10
    );

    // songCount and duration describe the album, not the tracks the caller
    // happened to load. "Tidewater" matches one of Alpha Sea's two tracks.
    let hit = subsonic_json(&router, "search3", api_key, "&query=Tidewater").await;
    let matched = &hit["subsonic-response"]["searchResult3"]["album"][0];
    assert_eq!(matched["name"], "Alpha Sea");
    assert_eq!(matched["songCount"], 2);
    assert_eq!(matched["duration"], 240);

    // Genres are counted once per canonical name across every visible library,
    // on both surfaces.
    let genres = subsonic_json(&router, "getGenres", api_key, "").await;
    let genres = genres["subsonic-response"]["genres"]["genre"]
        .as_array()
        .unwrap()
        .iter()
        .map(|genre| {
            (
                genre["value"].as_str().unwrap().to_owned(),
                genre["songCount"].as_i64().unwrap(),
                genre["albumCount"].as_i64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        genres,
        vec![
            ("Hip-Hop".to_owned(), 1, 1),
            ("Jazz".to_owned(), 2, 2),
            ("Rock".to_owned(), 3, 2),
        ]
    );
    let native_genres = router
        .clone()
        .oneshot(
            Request::get("/api/v2/genres")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native_genres.status(), StatusCode::OK);
    let native_genres = json_body(native_genres).await;
    assert_eq!(
        native_genres
            .as_array()
            .unwrap()
            .iter()
            .map(|genre| (
                genre["name"].as_str().unwrap().to_owned(),
                genre["song_count"].as_i64().unwrap(),
                genre["album_count"].as_i64().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("Hip-Hop".to_owned(), 1, 1),
            ("Jazz".to_owned(), 2, 2),
            ("Rock".to_owned(), 3, 2),
        ]
    );
}
