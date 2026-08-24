//! Browsing, listing and searching through the facade.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::authentication::now_ms;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::services::ServiceError;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

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

/// The browse methods used to resolve through a snapshot of every visible
/// track. They now ask for what they render, which is only observable as
/// behaviour at the edges: a foreign id is still not found, an album still
/// comes back in sleeve order, and the match-all search still pages.
#[tokio::test]
async fn browse_methods_read_only_what_they_render() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_browse-key";
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("browse-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"browse-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("browse-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();

    let seed = |account: Uuid, name: &'static str, artist: &'static str| {
        let state = state.clone();
        let root = config.data_dir.join(name);
        async move {
            std::fs::create_dir_all(&root).unwrap();
            let library = state
                .db
                .create_library(
                    account,
                    name,
                    &std::fs::canonicalize(&root).unwrap(),
                    LibraryVisibility::Private,
                    now_ms(),
                )
                .await
                .unwrap();
            let scan = state
                .db
                .create_scan_job(library, Some(account), "manual")
                .await
                .unwrap();
            state.db.start_scan_job(scan, 3, false).await.unwrap();
            // Deliberately out of sleeve order, and titled so that ordering by
            // title would give a different answer from ordering by track.
            for (index, (title, track)) in [("Zephyr", 1), ("Anvil", 2), ("Marrow", 3)]
                .into_iter()
                .enumerate()
            {
                let mut input = browse_input(
                    500 + index + name.len() * 10,
                    title,
                    "Ordered",
                    artist,
                    Some(track),
                    Some(1),
                );
                input.relative_path = format!("{name}-{index}.flac");
                input.quick_hash = format!("{:064x}", index + 51_000 + name.len() * 100);
                input.full_hash = format!("{:064x}", index + 52_000 + name.len() * 100);
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
            library
        }
    };
    let library = seed(owner, "browse-own", "Own Artist").await;
    seed(outsider, "browse-foreign", "Foreign Artist").await;

    let router = waveflow_server::app(&config, state.clone());
    let mine = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    let theirs = state
        .services
        .catalog_snapshot(outsider, &[])
        .await
        .unwrap();

    // getAlbum asks for one album, and returns it in sleeve order rather than
    // alphabetically.
    let album = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", mine.albums[0].id),
    )
    .await;
    let titles = album["subsonic-response"]["album"]["song"]
        .as_array()
        .expect("the album lists its songs")
        .iter()
        .map(|song| song["title"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(titles, vec!["Zephyr", "Anvil", "Marrow"]);

    // A foreign album is not found, indistinguishably from one that does not
    // exist. This is what the snapshot used to enforce by simply not holding
    // the row.
    for (method, id) in [
        ("getAlbum", theirs.albums[0].id),
        ("getArtist", theirs.artists[0].artist.id),
        ("getMusicDirectory", theirs.albums[0].id),
    ] {
        let response = subsonic_json(&router, method, api_key, &format!("&id={id}")).await;
        assert_eq!(
            response["subsonic-response"]["error"]["code"], 70,
            "{method} reached another account: {response}"
        );
    }

    // getMusicDirectory answers at all three levels, and the album level is the
    // only one that loads tracks.
    let folder = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={library}"),
    )
    .await;
    assert_eq!(
        folder["subsonic-response"]["directory"]["child"][0]["name"],
        "Own Artist"
    );
    let artist_dir = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={}", mine.artists[0].artist.id),
    )
    .await;
    assert_eq!(
        artist_dir["subsonic-response"]["directory"]["child"][0]["title"],
        "Ordered"
    );
    let album_dir = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={}", mine.albums[0].id),
    )
    .await;
    assert_eq!(
        album_dir["subsonic-response"]["directory"]["child"]
            .as_array()
            .expect("the album directory lists its tracks")
            .len(),
        3
    );

    // getStarred reads the star join rather than the catalogue, and reports one
    // of each kind.
    for (entity, id) in [
        ("artist", mine.artists[0].artist.id),
        ("album", mine.albums[0].id),
        ("track", mine.songs[0].id),
    ] {
        state
            .services
            .set_star(owner, entity, id, true)
            .await
            .unwrap();
    }
    let starred = subsonic_json(&router, "getStarred2", api_key, "").await;
    let starred = &starred["subsonic-response"]["starred2"];
    for field in ["artist", "album", "song"] {
        assert_eq!(
            starred[field].as_array().expect(field).len(),
            1,
            "{field}: {starred}"
        );
    }
    assert!(starred["song"][0]["starred"].is_string());

    // search3's match-all pages in SQL. Two pages of two cover three songs and
    // stop, and the third page is empty rather than an error.
    let page = |offset: usize| {
        let router = router.clone();
        async move {
            let response = subsonic_json(
                &router,
                "search3",
                api_key,
                &format!(
                    "&query=%22%22&songCount=2&songOffset={offset}&artistCount=0&albumCount=0"
                ),
            )
            .await;
            response["subsonic-response"]["searchResult3"]["song"]
                .as_array()
                .map(|songs| {
                    songs
                        .iter()
                        .map(|song| song["title"].as_str().unwrap_or_default().to_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }
    };
    let first = page(0).await;
    let second = page(2).await;
    let third = page(4).await;
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 1);
    assert!(third.is_empty());
    // The pages do not overlap, and between them they are the whole library —
    // the outsider's tracks are in neither.
    let mut seen = first;
    seen.extend(second);
    seen.sort();
    assert_eq!(seen, vec!["Anvil", "Marrow", "Zephyr"]);
}

/// `search3` runs on the FTS5 index rather than filtering a fully materialised
/// catalogue in memory. That is not a pure refactor — it changes which queries
/// match — so the trade is pinned here rather than left to a client to discover.
#[tokio::test]
async fn subsonic_search_matches_through_the_fts_index() {
    let (_temp, config, state) = test_app().await;
    let subsonic_password = "subsonic-secret-123";
    let api_key = "wfsk_search-key";
    let admin = state
        .db
        .create_account(
            "search-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state
        .secret_box
        .encrypt(subsonic_password.as_bytes())
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("search-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(admin, "Search", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 3, false).await.unwrap();
    // One album of three tracks, one of them accented.
    for (index, title) in ["Echo Chamber", "Écho lointain", "Silent Partner"]
        .into_iter()
        .enumerate()
    {
        let mut input = catalog_input(index, "Nocturne");
        input.title = title.to_owned();
        input.album = Some("Night Sessions".into());
        input.is_compilation = false;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let titles = |result: &serde_json::Value| -> Vec<String> {
        result["subsonic-response"]["searchResult3"]["song"]
            .as_array()
            .map(|songs| {
                songs
                    .iter()
                    .filter_map(|song| song["title"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };

    // Whole-word match, as before.
    let whole = subsonic_json(&router, "search3", api_key, "&query=Echo&songCount=10").await;
    assert!(titles(&whole).contains(&"Echo Chamber".to_owned()));

    // Search-as-you-type: the trailing term matches as a prefix, so a client
    // querying on every keystroke keeps getting results.
    let prefix = subsonic_json(&router, "search3", api_key, "&query=Ech&songCount=10").await;
    assert!(titles(&prefix).contains(&"Echo Chamber".to_owned()));

    // Gained: the tokenizer folds diacritics, so "echo" now reaches "Écho".
    // The previous lowercase substring test did not.
    assert!(titles(&prefix).contains(&"Écho lointain".to_owned()));

    // Given up: matching inside a word. "cho" used to find "Echo Chamber"
    // through a substring test and no longer does. Documented, not accidental.
    let infix = subsonic_json(&router, "search3", api_key, "&query=cho&songCount=10").await;
    assert!(!titles(&infix).contains(&"Echo Chamber".to_owned()));

    // Terms narrow rather than widen.
    let narrowed = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Echo%20Silent&songCount=10",
    )
    .await;
    assert!(titles(&narrowed).is_empty());

    // An album reports its own size, not how much of it the query hit: two of
    // the three tracks match "echo", and songCount must still read 3.
    let album = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Echo&albumCount=10&songCount=0",
    )
    .await;
    let albums = album["subsonic-response"]["searchResult3"]["album"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let night = albums
        .iter()
        .find(|album| album["name"] == "Night Sessions")
        .expect("matching album should be returned");
    assert_eq!(
        night["songCount"], 3,
        "songCount must describe the album, not the query"
    );

    // The documented match-all query still returns the whole catalogue.
    let all = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=%22%22&songCount=500&albumCount=500&artistCount=500",
    )
    .await;
    assert_eq!(titles(&all).len(), 3);
}

#[tokio::test]
async fn search_pages_each_kind_in_sql_and_bounds_what_one_request_may_name() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("pager", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let encrypted = state
        .secret_box
        .encrypt(b"dedicated-subsonic-secret")
        .unwrap();
    let api_key = "wfsk_pager-key";
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("pager-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Pager library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 5, false).await.unwrap();
    // One token every kind matches on, so the three pages are exercised by one
    // query rather than by three that happen not to overlap.
    for (index, name) in ["Aria", "Bela", "Cyd", "Dara", "Eno"]
        .into_iter()
        .enumerate()
    {
        let mut input = catalog_input(index, &format!("Nocturne {name}"));
        input.title = format!("Nocturne {index}");
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();
    // What a real scan does after applying its rows, and what builds the
    // artist search index this query has to reach.
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();

    let paged = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Nocturne&songCount=2&songOffset=1&artistCount=1&artistOffset=2&albumCount=5&albumOffset=1",
    )
    .await;
    let result = &paged["subsonic-response"]["searchResult3"];
    assert_eq!(
        result["song"]
            .as_array()
            .unwrap()
            .iter()
            .map(|song| song["title"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["Nocturne 1", "Nocturne 2"],
        "the song offset has to skip in SQL and still land on the same rows"
    );
    assert_eq!(
        result["artist"]
            .as_array()
            .unwrap()
            .iter()
            .map(|artist| artist["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["Nocturne Cyd"],
        "each kind pages independently, which is what search3 has always allowed"
    );
    // Five tracks, one album between them: an offset of one leaves nothing, and
    // `searchResult3` omits a kind it has no rows for rather than sending `[]`.
    assert!(result["album"].is_null());

    // A request may not name more identifiers than the queue may hold: each one
    // costs a mutation under the process-wide writer gate.
    let track_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs[0]
        .id;
    let oversized = (0..=waveflow_server::services::MAX_QUEUE_TRACKS)
        .map(|_| format!("&id={track_id}"))
        .collect::<String>();
    let refused = subsonic_json(&router, "star", api_key, &oversized).await;
    assert_eq!(refused["subsonic-response"]["status"], "failed");
    assert_eq!(refused["subsonic-response"]["error"]["code"], 10);
    // And one below the ceiling still works.
    let accepted = subsonic_json(&router, "star", api_key, &format!("&id={track_id}")).await;
    assert_eq!(accepted["subsonic-response"]["status"], "ok");

    // A playlist is bounded on what it holds rather than on what one request
    // carries, because it grows across many of them.
    let too_many = vec![track_id; waveflow_server::services::MAX_PLAYLIST_TRACKS + 1];
    assert!(matches!(
        state
            .services
            .create_playlist(owner, "Oversized", &too_many)
            .await,
        Err(ServiceError::Invalid)
    ));
}
