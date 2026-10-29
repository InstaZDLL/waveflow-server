//! Search through the FTS index, paged per kind in SQL.

use super::*;

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

    // Only that literal pair of quotes browses everything. An empty or blank
    // query names no term, and answers an empty result rather than the
    // catalogue — as docs/compatibility/subsonic-compatibility.md says.
    for blank in ["", "%20%20"] {
        let none = subsonic_json(
            &router,
            "search3",
            api_key,
            &format!("&query={blank}&songCount=500&albumCount=500&artistCount=500"),
        )
        .await;
        assert_eq!(none["subsonic-response"]["status"], "ok", "query={blank:?}");
        assert!(titles(&none).is_empty(), "query={blank:?}");
        for kind in ["artist", "album"] {
            assert!(
                none["subsonic-response"]["searchResult3"][kind]
                    .as_array()
                    .is_none_or(Vec::is_empty),
                "query={blank:?}: {kind}"
            );
        }
    }
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

    // A request may not name more identifiers than one request may carry:
    // each one costs a mutation under the process-wide writer gate.
    let track_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs[0]
        .id;
    let oversized = (0..=waveflow_server::services::MAX_REQUEST_IDENTIFIERS)
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

/// A track whose file was removed takes its title out of the album search.
///
/// Found in the 2026-10-28 client campaign: after *Coda* was removed from
/// *Late Additions*, `search2 query=Coda` still answered the album — with no
/// song, since the song search already left the removed track out. The album
/// search matched through `track_fts`, which keeps the removed track's row,
/// without asking whether the track is still there. Every surface reached the
/// same two queries, so each is asked here: `search2`, `search3` and the
/// native `/api/v2/search`. And the file comes back, to show the title is
/// findable again from the ordinary rescan, with no index rebuilt.
#[tokio::test]
async fn an_album_is_not_found_by_the_title_of_a_track_whose_file_went() {
    let (_temp, config, state) = test_app().await;
    // Drawn at run time: a literal reaching the hash is what CodeQL reports.
    let password = Uuid::new_v4().to_string();
    let owner = state
        .db
        .create_account(
            "removed-search",
            &security::hash_password(&password).unwrap(),
            AccountRole::User,
            now_ms(),
        )
        .await
        .unwrap();
    let api_key = format!("wfsk_{}", Uuid::new_v4().simple());
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state
                .secret_box
                .encrypt(Uuid::new_v4().to_string().as_bytes())
                .unwrap(),
            &security::token_hash(&api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("removed-search-music");
    std::fs::create_dir_all(&music).unwrap();
    // Two tones, so the two files differ in content as well as in name.
    write_tagged_flac(
        &music.join("Overture.flac"),
        "Overture",
        "Late Additions",
        440,
    );
    write_tagged_flac(&music.join("Coda.flac"), "Coda", "Late Additions", 660);
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Removed search",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let record = waveflow_server::storage::catalog::LibraryRecord {
        id: library,
        name: "Removed search".into(),
        root_path: root,
    };
    run_scan(&state, owner, record.clone()).await;
    let album_of = |path: &'static str| {
        let state = state.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT album_id FROM track WHERE relative_path=?")
                .bind(path)
                .fetch_one(state.db.pool())
                .await
                .unwrap()
        }
    };
    let album = album_of("Coda.flac").await;
    assert_eq!(
        album_of("Overture.flac").await,
        album,
        "the precondition: one album of two tracks"
    );
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "removed-search", &password).await;

    // The album ids and song titles one query answers, surface by surface.
    let found = |query: &'static str| {
        let router = router.clone();
        let api_key = api_key.clone();
        let token = token.clone();
        async move {
            let values = |list: &serde_json::Value, field: &str| -> Vec<String> {
                list.as_array()
                    .map(|items| {
                        items
                            .iter()
                            .map(|item| item[field].as_str().unwrap().to_owned())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let mut answers = Vec::new();
            for (method, result) in [("search2", "searchResult2"), ("search3", "searchResult3")] {
                let body = subsonic_json(
                    &router,
                    method,
                    &api_key,
                    &format!("&query={query}&albumCount=10&songCount=10"),
                )
                .await;
                let result = &body["subsonic-response"][result];
                answers.push((
                    method,
                    values(&result["album"], "id"),
                    values(&result["song"], "title"),
                ));
            }
            let response = send_json(
                &router,
                "GET",
                &format!("/api/v2/search?q={query}"),
                &token,
                None,
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            answers.push((
                "/api/v2/search",
                values(&body["albums"], "id"),
                values(&body["songs"], "title"),
            ));
            answers
        }
    };
    let assert_found = |answers: Vec<(&str, Vec<String>, Vec<String>)>, song: &str| {
        for (surface, albums, songs) in answers {
            assert_eq!(
                albums,
                std::slice::from_ref(&album),
                "{surface}: the album is found"
            );
            assert_eq!(songs, [song.to_owned()], "{surface}: the song is found");
        }
    };

    // Both titles find the album while both files are there.
    assert_found(found("Coda").await, "Coda");
    assert_found(found("Overture").await, "Overture");

    std::fs::remove_file(music.join("Coda.flac")).unwrap();
    run_scan(&state, owner, record.clone()).await;
    let available: i64 =
        sqlx::query_scalar("SELECT is_available FROM track WHERE relative_path='Coda.flac'")
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(available, 0, "the precondition: the track is unavailable");

    // The removed title finds nothing: neither its song nor its album.
    for (surface, albums, songs) in found("Coda").await {
        assert!(songs.is_empty(), "{surface}: no song by a removed title");
        assert!(
            albums.is_empty(),
            "{surface}: the album is found by the title of a track it no longer holds"
        );
    }
    // The album is still there, and found by the title it still holds.
    assert_found(found("Overture").await, "Overture");

    // The file comes back, and so does the match.
    write_tagged_flac(&music.join("Coda.flac"), "Coda", "Late Additions", 660);
    run_scan(&state, owner, record).await;
    assert_found(found("Coda").await, "Coda");
    assert_found(found("Overture").await, "Overture");
}
