//! Listing and searching the catalogue: paging, sort names, songs without an
//! album, the library scope.

use super::*;

#[tokio::test]
async fn track_pages_are_stable_when_titles_and_fts_ranks_match() {
    let (_temp, config, state) = test_app().await;
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("stable-pages", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("stable-pages");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Stable pages",
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
    state.db.start_scan_job(scan_id, 2, false).await.unwrap();
    for index in 0..2 {
        state
            .db
            .apply_catalog_track(
                library_id,
                scan_id,
                &browse_input(
                    10_000 + index,
                    "Mirror Signal",
                    "Stable Paging",
                    "Deterministic Artist",
                    Some(index as i64 + 1),
                    Some(1),
                ),
                None,
                false,
            )
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state);
    let token = login_token(&router, "stable-pages", &password).await;
    let page_id = |query: &'static str| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(format!("/api/v2/libraries/{library_id}/tracks?{query}"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            let page = body.as_array().unwrap();
            assert_eq!(page.len(), 1);
            page[0]["id"].as_str().unwrap().to_owned()
        }
    };

    let normal = vec![
        page_id("limit=1&offset=0").await,
        page_id("limit=1&offset=1").await,
    ];
    let fts = vec![
        page_id("q=Mirror%20Signal&limit=1&offset=0").await,
        page_id("q=Mirror%20Signal&limit=1&offset=1").await,
    ];
    let mut expected = normal.clone();
    expected.sort();
    assert_eq!(normal, expected, "title ties use the UUID as final order");
    assert_eq!(fts, expected, "FTS rank ties use the UUID as final order");
    assert_ne!(expected[0], expected[1]);
}

/// The three OpenSubsonic gaps the fifth audit named, pinned together because
/// they are one statement: what the catalogue can answer, it now says.
///
/// `sortName` moves from absent to present-and-possibly-empty — the presence
/// rule's difference between "not supported" and "not tagged". `song.parent`
/// stops naming a directory that would not list the song. And the native
/// search documents the 400 its required parameter already produced.
#[tokio::test]
async fn the_catalogue_answers_for_sort_names_and_for_songs_without_an_album() {
    let (_temp, config, state) = test_app().await;
    let subsonic_password = "subsonic-secret-123";
    let api_key = "wfsk_sortname-key";
    let admin = state
        .db
        .create_account(
            "sort-admin",
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
    let music = config.data_dir.join("sort-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(admin, "Sorted", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 4, false).await.unwrap();

    // A tagged album, whose sort forms differ from the display forms — the
    // only case where the field carries information.
    let mut tagged = catalog_input(0, "The Nocturnes");
    tagged.title = "Opening".into();
    tagged.album = Some("The Night Sessions".into());
    tagged.album_artist = Some("The Nocturnes".into());
    tagged.is_compilation = false;
    tagged.sort_album = Some("Night Sessions, The".into());
    tagged.sort_album_artist = Some("Nocturnes, The".into());
    tagged.sort_artist = Some("Nocturnes, The".into());
    state
        .db
        .apply_catalog_track(library, scan, &tagged, None, false)
        .await
        .unwrap();

    // An untagged album by another artist: supported and unknown, which is not
    // the same statement as unsupported.
    let mut untagged = catalog_input(1, "Plain Ensemble");
    untagged.title = "Untitled".into();
    untagged.album = Some("Plain Record".into());
    untagged.album_artist = Some("Plain Ensemble".into());
    untagged.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &untagged, None, false)
        .await
        .unwrap();

    // And a track belonging to no album at all: the one that names its library
    // as its parent for want of an album id.
    let mut orphan = catalog_input(2, "Lone Voice");
    orphan.title = "Single Only".into();
    orphan.album = None;
    orphan.album_artist = None;
    orphan.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &orphan, None, false)
        .await
        .unwrap();
    // A second one, so the folder's ceiling has something to cut.
    let mut second_orphan = catalog_input(3, "Lone Voice");
    second_orphan.title = "Also Alone".into();
    second_orphan.album = None;
    second_orphan.album_artist = None;
    second_orphan.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &second_orphan, None, false)
        .await
        .unwrap();
    // Sort names are derived at the end of a scan, like the identifiers: the
    // scanner runs both passes here, so a test driving the catalogue directly
    // runs them too.
    state.db.consolidate_sort_names(library).await.unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    // --- sortName on AlbumID3 -------------------------------------------
    let albums = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let albums = albums["subsonic-response"]["albumList2"]["album"]
        .as_array()
        .expect("the album list")
        .clone();
    let sort_of = |name: &str| -> String {
        albums
            .iter()
            .find(|album| album["name"] == name)
            .unwrap_or_else(|| panic!("{name} is listed"))["sortName"]
            .as_str()
            .expect("sortName is emitted for every album")
            .to_owned()
    };
    assert_eq!(sort_of("The Night Sessions"), "Night Sessions, The");
    // Emitted empty rather than omitted: the difference between a server that
    // cannot answer and an album no file supplied a sort tag for.
    assert_eq!(sort_of("Plain Record"), "");

    // --- sortName on ArtistID3 ------------------------------------------
    let artists = subsonic_json(&router, "getArtists", api_key, "").await;
    let mut seen = std::collections::BTreeMap::new();
    for index in artists["subsonic-response"]["artists"]["index"]
        .as_array()
        .expect("the artist index")
    {
        for artist in index["artist"].as_array().expect("an index holds artists") {
            seen.insert(
                artist["name"].as_str().unwrap().to_owned(),
                artist["sortName"]
                    .as_str()
                    .expect("sortName is emitted for every artist")
                    .to_owned(),
            );
        }
    }
    assert_eq!(
        seen.get("The Nocturnes").map(String::as_str),
        Some("Nocturnes, The")
    );
    assert_eq!(seen.get("Plain Ensemble").map(String::as_str), Some(""));

    // --- the parent of a song without an album --------------------------
    let directory = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={library}"),
    )
    .await;
    let children = directory["subsonic-response"]["directory"]["child"]
        .as_array()
        .expect("the folder lists children")
        .clone();
    let orphan_child = children
        .iter()
        .find(|child| child["title"] == "Single Only")
        .expect("a song with no album is reachable by browsing its library");
    // The claim is coherence, not a new identifier: the song already said this
    // was its parent, and browsing there now finds it.
    assert_eq!(
        orphan_child["parent"].as_str(),
        Some(library.to_string()).as_deref()
    );
    assert_eq!(orphan_child["isDir"], serde_json::json!(false));
    // The artists of the library are still listed alongside it.
    assert!(
        children
            .iter()
            .any(|child| child["title"] == "The Nocturnes" || child["name"] == "The Nocturnes"),
        "the folder still lists its artists: {children:?}"
    );
    // An album's own track is not duplicated into the folder level.
    assert!(
        !children.iter().any(|child| child["title"] == "Opening"),
        "only album-less tracks belong at the folder level"
    );
    // Both album-less tracks are there: the ceiling the facade passes is a
    // bound on the answer, not a page the client has to ask again for.
    assert!(
        children.iter().any(|child| child["title"] == "Also Alone"),
        "every album-less track under the ceiling is listed: {children:?}"
    );

    // --- the folder listing is bounded ----------------------------------
    // `getMusicDirectory` takes no offset, so the query has to stop on its
    // own or a library of loose files answers with all of them at once.
    let capped = state
        .services
        .songs_without_album(admin, library, 1)
        .await
        .unwrap();
    assert_eq!(
        capped
            .iter()
            .map(|song| song.title.as_str())
            .collect::<Vec<_>>(),
        vec!["Also Alone"],
        "the ceiling cuts the listing in its own order, not at random"
    );
    let whole = state
        .services
        .songs_without_album(admin, library, 2_000)
        .await
        .unwrap();
    assert_eq!(whole.len(), 2, "a ceiling above the library cuts nothing");
    assert!(
        matches!(
            state.services.songs_without_album(admin, library, 0).await,
            Err(ServiceError::Invalid)
        ),
        "a ceiling of nothing is a caller error, not an empty folder"
    );

    // --- the same artist projection, wherever it is read ----------------
    // `getArtists`, the folder listing and the search each used to spell the
    // artist columns out for themselves, and one copy fell behind the day the
    // list gained `sortName`. They read one projection now, so the value that
    // reaches the index has to reach the search too.
    let searched = subsonic_json(&router, "search3", api_key, "&query=Nocturnes").await;
    let searched_artist = searched["subsonic-response"]["searchResult3"]["artist"]
        .as_array()
        .expect("the search answers with artists")
        .iter()
        .find(|artist| artist["name"] == "The Nocturnes")
        .expect("the searched artist is listed")
        .clone();
    assert_eq!(searched_artist["sortName"], "Nocturnes, The");

    // --- the native search's required parameter -------------------------
    let token = login_token(&router, "sort-admin", "correct horse battery staple").await;
    let missing_q = router
        .clone()
        .oneshot(
            Request::get("/api/v2/search")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        missing_q.status(),
        StatusCode::BAD_REQUEST,
        "the OpenAPI document now says 400, so the route must mean it"
    );

    // --- a sort tag removed from the files leaves the catalogue ---------
    // Writing the value during the per-track upsert could not do this: the
    // artist row is rewritten once per track, so a file with no tag had to be
    // stopped from erasing what a sibling supplied — and that preservation
    // outlived the tag. Deriving at the end of the scan is what makes removal
    // mean removal, exactly as it already does for the MusicBrainz ids.
    let rescan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(rescan, 1, false).await.unwrap();
    let mut untagged_now = tagged.clone();
    untagged_now.sort_album = None;
    untagged_now.sort_album_artist = None;
    untagged_now.sort_artist = None;
    // The same file, retagged: re-applied onto its own row rather than added
    // beside it, which is what a rescan of an edited file does.
    let existing = state
        .services
        .catalog_snapshot(admin, &[])
        .await
        .unwrap()
        .songs
        .into_iter()
        .find(|song| song.title == "Opening")
        .expect("the tagged track is in the catalogue")
        .id;
    state
        .db
        .apply_catalog_track(library, rescan, &untagged_now, Some(existing), false)
        .await
        .unwrap();
    state.db.consolidate_sort_names(library).await.unwrap();
    state.db.finish_scan_job(rescan, 0).await.unwrap();

    let after = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let after_album = after["subsonic-response"]["albumList2"]["album"]
        .as_array()
        .expect("the album list")
        .iter()
        .find(|album| album["name"] == "The Night Sessions")
        .expect("the album is still listed")
        .clone();
    assert_eq!(
        after_album["sortName"].as_str(),
        Some(""),
        "a sort tag removed from the files must leave the catalogue with it"
    );

    let after_artists = subsonic_json(&router, "getArtists", api_key, "").await;
    let mut after_seen = std::collections::BTreeMap::new();
    for index in after_artists["subsonic-response"]["artists"]["index"]
        .as_array()
        .expect("the artist index")
    {
        for artist in index["artist"].as_array().expect("an index holds artists") {
            after_seen.insert(
                artist["name"].as_str().unwrap().to_owned(),
                artist["sortName"].as_str().unwrap().to_owned(),
            );
        }
    }
    assert_eq!(
        after_seen.get("The Nocturnes").map(String::as_str),
        Some("")
    );
}

/// The active library narrows a search, and narrows all three kinds of it.
///
/// `/api/v2/search` was the one catalogue route the scope did not reach, and
/// the web client said so on the screen rather than pretending otherwise. The
/// FTS index is server-wide and stays so — it is the rows a match resolves to
/// that are narrowed, which is where tenancy already lived.
///
/// Two libraries, one word, and a deliberate hit of every kind in each: an
/// assertion that only counted songs would pass against a server that scoped
/// the song query alone, and the album and artist queries are separate
/// statements that each had to be changed.
///
/// The account is inserted rather than created, and reaches HTTP through an
/// API token rather than a password. `tests/scrobbling.rs` explains why the
/// row is written by hand; what this adds is that the trick is not confined to
/// targets without a surface — `wfapi_` authenticates on the same header a
/// session does, so a test needing routes needs no fixture password either.
#[tokio::test]
async fn a_search_narrows_to_one_library_across_all_three_kinds() {
    let (_temp, config, state) = test_app().await;
    let owner = uuid::Uuid::new_v4();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(owner.to_string())
    .bind("search-scope")
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();

    // The same word in the title, the album and the artist of both, so all
    // three kinds have something to answer on either side of the boundary.
    let shelf = |name: &'static str, index: usize, title: &'static str, album, artist| {
        let state = &state;
        let config = &config;
        async move {
            let music = config.data_dir.join(name);
            std::fs::create_dir_all(&music).unwrap();
            let library = state
                .db
                .create_library(
                    owner,
                    name,
                    &std::fs::canonicalize(&music).unwrap(),
                    LibraryVisibility::Private,
                    now,
                )
                .await
                .unwrap();
            let scan = state
                .db
                .create_scan_job(library, Some(owner), "manual")
                .await
                .unwrap();
            state.db.start_scan_job(scan, 1, false).await.unwrap();
            let input = browse_input(index, title, album, artist, Some(1), Some(1));
            state
                .db
                .apply_catalog_track(library, scan, &input, None, false)
                .await
                .unwrap();
            state
                .db
                .consolidate_catalog_derivations(library)
                .await
                .unwrap();
            state.db.finish_scan_job(scan, 0).await.unwrap();
            library
        }
    };
    let near = shelf(
        "near-shelf",
        700,
        "Beacon Hymn",
        "Beacon Harbour",
        "Beacon Wardens",
    )
    .await;
    let far = shelf(
        "far-shelf",
        701,
        "Beacon Echo",
        "Beacon Foundry",
        "Beacon Keepers",
    )
    .await;

    let (_record, secret) = state
        .services
        .create_api_token(owner, &[], "search-scope", "scoped search", &[])
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state);
    let names = |uri: String| {
        let router = router.clone();
        let secret = secret.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(uri)
                        .header("authorization", format!("Bearer {secret}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let found = json_body(response).await;
            let pluck = |kind: &str, field: &str| {
                let mut values = found[kind]
                    .as_array()
                    .unwrap_or_else(|| panic!("{kind} is a list"))
                    .iter()
                    .map(|item| item[field].as_str().unwrap().to_owned())
                    .collect::<Vec<_>>();
                values.sort();
                values
            };
            (
                pluck("artists", "name"),
                pluck("albums", "title"),
                pluck("songs", "title"),
            )
        }
    };

    // Unscoped, the search still answers for every library the account sees:
    // this parameter narrows, it does not become mandatory.
    let (artists, albums, songs) = names("/api/v2/search?q=Beacon".into()).await;
    assert_eq!(artists, ["Beacon Keepers", "Beacon Wardens"]);
    assert_eq!(albums, ["Beacon Foundry", "Beacon Harbour"]);
    assert_eq!(songs, ["Beacon Echo", "Beacon Hymn"]);

    let (artists, albums, songs) =
        names(format!("/api/v2/search?q=Beacon&library_id={near}")).await;
    assert_eq!(artists, ["Beacon Wardens"], "the artist query narrows");
    assert_eq!(albums, ["Beacon Harbour"], "the album query narrows");
    assert_eq!(songs, ["Beacon Hymn"], "the song query narrows");

    // And the other way round, so the assertion above cannot be satisfied by a
    // server that always answers the first library.
    let (artists, albums, songs) = names(format!("/api/v2/search?q=Beacon&library_id={far}")).await;
    assert_eq!(artists, ["Beacon Keepers"]);
    assert_eq!(albums, ["Beacon Foundry"]);
    assert_eq!(songs, ["Beacon Echo"]);
}

/// The songs listing answers the whole catalogue, orders it, and pages it.
///
/// #179 reported `GET /api/v2/songs` requiring a `genre` and so listing
/// nothing. It was closed by renaming the route, which made the contract
/// honest without making the listing exist. The first assertion here is that
/// one does: no parameters, every available track.
///
/// Everything is read back from the server. The client's own types are
/// hand-written assertions about this wire and its mocks are built from those
/// types, so a test on that side would confirm the type rather than the route.
#[tokio::test]
async fn the_songs_listing_answers_the_whole_catalogue_orders_it_and_pages_it() {
    let (_temp, config, state) = test_app().await;
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("songs-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let stranger = state
        .db
        .create_account("songs-stranger", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    assert_ne!(owner, stranger);
    let music = config.data_dir.join("songs-listing");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Songs listing",
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
    state.db.start_scan_job(scan_id, 4, false).await.unwrap();

    // Titles deliberately disagree with sleeve order, so ordering by title and
    // ordering by album cannot both be satisfied by one arrangement — an
    // assertion that holds under either would prove nothing.
    //
    // The last row carries no artist and no album. That is the row the
    // `NULLS LAST` claim is about: under `COALESCE(x, '')`, which is what the
    // album listing does, it would lead both alphabetical answers.
    //
    // One row carries a different genre. Without it the genre assertion below
    // could not fail: `browse_input` tags every row "Ambient", so asking for
    // Ambient would answer four whether the filter applied or was dropped.
    for (index, (title, album, artist, track, genre)) in [
        (
            "Zephyr",
            Some("Anvil Hours"),
            Some("Beta Person"),
            Some(2),
            "Ambient",
        ),
        (
            "Anvil",
            Some("Anvil Hours"),
            Some("Beta Person"),
            Some(1),
            "Ambient",
        ),
        (
            "Marrow",
            Some("Zenith Field"),
            Some("Aria Lux"),
            Some(1),
            "Dub",
        ),
        ("Orphan", None, None, None, "Ambient"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(
            20_000 + index,
            title,
            album.unwrap_or("placeholder"),
            artist.unwrap_or("placeholder"),
            track,
            Some(1),
        );
        input.album = album.map(str::to_owned);
        input.album_artist = album.and(artist).map(str::to_owned);
        input.artist = artist.map(str::to_owned);
        input.genre = Some(genre.to_owned());
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "songs-owner", &password).await;
    let stranger_token = login_token(&router, "songs-stranger", &password).await;

    async fn titles(router: &axum::Router, token: &str, query: &str) -> Vec<String> {
        let response = router
            .clone()
            .oneshot(
                Request::get(format!("/api/v2/songs{query}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "GET /api/v2/songs{query}"
        );
        json_body(response)
            .await
            .as_array()
            .expect("an array of songs")
            .iter()
            .map(|song| song["title"].as_str().expect("a title").to_owned())
            .collect()
    }

    // 1. No parameters, the whole catalogue. This is the gap #179 described.
    let everything = titles(&router, &token, "").await;
    assert_eq!(everything.len(), 4, "{everything:?}");

    // 2. By title.
    assert_eq!(
        titles(&router, &token, "?sort=alphabeticalByName").await,
        ["Anvil", "Marrow", "Orphan", "Zephyr"]
    );

    // 3. By artist, and the untagged row goes **last**. Aria Lux before Beta
    //    Person; the two Beta Person rows then fall back to title.
    assert_eq!(
        titles(&router, &token, "?sort=alphabeticalByArtist").await,
        ["Marrow", "Anvil", "Zephyr", "Orphan"]
    );

    // 4. By album, in sleeve order inside each one, untagged last. "Anvil
    //    Hours" holds Anvil at track 1 and Zephyr at track 2, so a listing
    //    ordered by title would have given the opposite pair.
    assert_eq!(
        titles(&router, &token, "?sort=alphabeticalByAlbum").await,
        ["Anvil", "Zephyr", "Marrow", "Orphan"]
    );

    // 5. Two pages partition the catalogue: no row twice, none missing. Every
    //    ordering ends on `id`, which is what makes an offset stable when two
    //    rows tie.
    let first = titles(&router, &token, "?sort=alphabeticalByName&limit=2&offset=0").await;
    let second = titles(&router, &token, "?sort=alphabeticalByName&limit=2&offset=2").await;
    assert_eq!(first, ["Anvil", "Marrow"]);
    assert_eq!(second, ["Orphan", "Zephyr"]);
    let mut paged: Vec<String> = first.into_iter().chain(second).collect();
    paged.sort();
    let mut whole = everything;
    whole.sort();
    assert_eq!(paged, whole, "the pages do not cover the listing exactly");

    // 6. `byGenre` with no genre is malformed, not empty. Answering the whole
    //    catalogue would drop the filter in silence, which is the fault #179
    //    was filed about in the first place.
    let no_genre = router
        .clone()
        .oneshot(
            Request::get("/api/v2/songs?sort=byGenre")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_genre.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // Three of the four, and the fourth reachable by its own genre. Counted
    // **and named**: a count alone would be satisfied by a filter that
    // dropped the wrong row, and asking only for the genre three rows share
    // would be satisfied by no filter at all.
    let ambient = titles(&router, &token, "?sort=byGenre&genre=Ambient").await;
    assert_eq!(ambient, ["Anvil", "Orphan", "Zephyr"]);
    assert_eq!(
        titles(&router, &token, "?sort=byGenre&genre=Dub").await,
        ["Marrow"]
    );
    // And a genre nobody uses is empty rather than everything.
    assert!(titles(&router, &token, "?sort=byGenre&genre=Polka")
        .await
        .is_empty());

    // 7. An ordering nobody implements is refused rather than silently
    //    replaced by the default.
    let nonsense = router
        .clone()
        .oneshot(
            Request::get("/api/v2/songs?sort=byWhateverTheClientFeltLike")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(nonsense.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // 8. Tenancy is in the query, so an account with no membership sees an
    //    empty catalogue rather than another tenant's.
    assert!(titles(&router, &stranger_token, "").await.is_empty());
}
