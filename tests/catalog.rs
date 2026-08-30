//! The catalogue model: albums, artists, credits, genres and tenancy.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
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

/// An album artist tag holding two credits is two artists, and the album now
/// hangs off both.
///
/// Feeding the joined string to the artist table minted an entity named after
/// it, gave that entity the album, and left both real artists with nothing:
/// DSub browsed to either and found no album. Splitting the credit fixed the
/// entity but not the browse — the album still pointed at one artist through a
/// single column, so the second credit found nothing. The album's participants
/// are what answer now, and both credits reach it.
#[tokio::test]
async fn an_album_hangs_off_every_artist_it_is_credited_to() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("credits", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("credit-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Credits",
            &root,
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
    state.db.start_scan_job(scan, 4, false).await.unwrap();

    // Four albums whose credits differ only in where the boundaries fall.
    for (index, album, credit) in [
        (0usize, "Live", "Nova Kern; Lior Sand"),
        (1, "Live", "Nova Kern; Ada Vale"),
        (2, "Split", "A; B C"),
        (3, "Split", "A; B; C"),
    ] {
        let mut input = catalog_input(index, credit);
        input.title = format!("Track {index}");
        input.album = Some(album.into());
        input.album_artist = Some(credit.into());
        input.is_compilation = false;
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

    // No entity is named after a joined string.
    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .iter()
        .map(|summary| summary.artist.name.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "A".to_owned(),
            "Ada Vale".to_owned(),
            "B".to_owned(),
            "B C".to_owned(),
            "C".to_owned(),
            "Lior Sand".to_owned(),
            "Nova Kern".to_owned(),
        ],
        "each credit is its own artist and the joined string is nobody"
    );

    // Sharing a title and a lead credit is not sharing an album.
    let albums = state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap();
    assert_eq!(
        albums.iter().filter(|album| album.title == "Live").count(),
        2,
        "`A; B` and `A; C` are two records, not one"
    );
    assert_eq!(
        albums.iter().filter(|album| album.title == "Split").count(),
        2,
        "where the boundary falls is part of the identity"
    );

    // And the second credit reaches the album, which is what the single
    // `album_artist_id` column could never answer.
    let by_name = |name: &str| {
        artists
            .iter()
            .find(|summary| summary.artist.name == name)
            .unwrap_or_else(|| panic!("{name} was indexed"))
            .artist
            .id
    };
    for (name, credited_on) in [("Nova Kern", 2), ("Lior Sand", 1), ("Ada Vale", 1)] {
        let detail = state.services.artist(owner, by_name(name)).await.unwrap();
        assert_eq!(
            detail.albums.len(),
            credited_on,
            "browsing to {name} finds every album it is credited to"
        );
        assert!(detail.albums.iter().all(|album| album.title == "Live"));
        assert_eq!(
            detail.album_count, credited_on as i64,
            "{name}'s album count counts the credits, not a single column"
        );
    }

    // The display string stays what the file wrote, joins and all.
    let live = albums
        .iter()
        .find(|album| album.title == "Live")
        .expect("an album titled Live");
    assert!(
        live.artist
            .as_deref()
            .is_some_and(|credit| credit.contains("; ")),
        "the album still renders the credit as written: {:?}",
        live.artist
    );
}

/// A release identifier belongs to the release, not to whichever file was
/// scanned last.
///
/// Under the default identity spec the release identifier *is* the album's
/// identity, so files naming different releases are different albums and the
/// majority vote has nothing left to settle there. It still runs, because a
/// spec that does not name `musicbrainz_albumid` puts the disagreement back —
/// and because the artist's identifier is voted on the same way, where no spec
/// can make the question go away.
#[tokio::test]
async fn entity_musicbrainz_ids_are_a_majority_vote_over_the_tracks() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_mbid-key";
    let owner = state
        .db
        .create_account(
            "mbid-owner",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"mbid-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("mbid-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Mbid",
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
    state.db.start_scan_job(scan, 6, false).await.unwrap();

    struct TaggedFile {
        album: &'static str,
        title: &'static str,
        track: i64,
        disc: i64,
        release: Option<&'static str>,
        artist: Option<&'static str>,
    }
    let file = |album, title, track, disc, release, artist| TaggedFile {
        album,
        title,
        track,
        disc,
        release,
        artist,
    };
    let fixture = [
        // Two files agree on the reissue, one still carries the original
        // pressing. The majority wins, and the odd file does not.
        file(
            "Split Sky",
            "Dawn",
            1,
            1,
            Some("release-reissue"),
            Some("artist-vale"),
        ),
        file(
            "Split Sky",
            "Noon",
            2,
            1,
            Some("release-original"),
            Some("artist-vale"),
        ),
        file("Split Sky", "Dusk", 3, 1, Some("release-reissue"), None),
        // A genuine tie, one file each. It is broken by the earliest disc and
        // track, so the answer is stable across scans rather than arbitrary.
        // The earlier one sorts last as a string, so a lexical fallback alone
        // would answer the other.
        file(
            "Even Halves",
            "Side A",
            1,
            1,
            Some("release-zulu"),
            Some("artist-vale"),
        ),
        file(
            "Even Halves",
            "Side B",
            1,
            2,
            Some("release-alpha"),
            Some("artist-other"),
        ),
        // Nothing tagged at all.
        file("No Tags", "Silence", 1, 1, None, None),
    ];
    for (index, entry) in fixture.into_iter().enumerate() {
        let mut input = browse_input(
            300 + index,
            entry.title,
            entry.album,
            "Vale",
            Some(entry.track),
            Some(entry.disc),
        );
        input.relative_path = format!("mbid-{index}.flac");
        input.quick_hash = format!("{:064x}", index + 31_000);
        input.full_hash = format!("{:064x}", index + 32_000);
        input.musicbrainz_release_id = entry.release.map(str::to_owned);
        input.musicbrainz_artist_id = entry.artist.map(str::to_owned);
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let albums_by_title = || {
        let state = state.clone();
        async move {
            state
                .services
                .catalog_snapshot(owner, &[])
                .await
                .unwrap()
                .albums
                .into_iter()
                .map(|album| (album.title.clone(), album))
                .collect::<std::collections::BTreeMap<_, _>>()
        }
    };

    // Nothing is derived until the pass that derives it: the tracks carry the
    // identifiers from the moment they are indexed, the albums do not.
    assert!(state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap()
        .iter()
        .all(|album| album.musicbrainz_id.is_none()));

    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();

    // The majority vote no longer has anything to resolve on an album, and
    // that is the point: under the default identity spec a release identifier
    // *is* the album's identity, so two files carrying different ones are two
    // albums rather than one album with a disagreement to settle. "Split Sky"
    // holds three files naming two releases, and answers as two records.
    let all_albums = state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap();
    let split: Vec<&str> = all_albums
        .iter()
        .filter(|album| album.title == "Split Sky")
        .filter_map(|album| album.musicbrainz_id.as_deref())
        .collect();
    let mut split = split;
    split.sort_unstable();
    assert_eq!(
        split,
        vec!["release-original", "release-reissue"],
        "two release identifiers are two albums, each reporting its own"
    );
    // The vote still runs, and still clears: an album whose files name no
    // release has nothing to report.
    assert!(all_albums
        .iter()
        .filter(|album| album.title == "No Tags")
        .all(|album| album.musicbrainz_id.is_none()));

    // The artist takes the identifier from the tracks it is the primary credit
    // of. Every track here credits Vale first, and `artist-vale` is what most
    // of them say.
    let snapshot = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    let artist = snapshot
        .artists
        .iter()
        .find(|artist| artist.artist.name == "Vale")
        .expect("the artist was indexed");
    assert_eq!(artist.artist.musicbrainz_id.as_deref(), Some("artist-vale"));

    let router = waveflow_server::app(&config, state.clone());
    let albums = albums_by_title().await;
    // Either of the two "Split Sky" records answers; both carry the release
    // they were identified by.
    let reissue = all_albums
        .iter()
        .find(|album| album.musicbrainz_id.as_deref() == Some("release-reissue"))
        .expect("the reissue is one of the two records");
    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={}", reissue.id)).await;
    assert_eq!(
        album["subsonic-response"]["album"]["musicBrainzId"],
        "release-reissue"
    );
    // Presence, not omission: an album with no release id still carries the
    // field, because that is the only way a client tells "untagged" from "this
    // server does not read the tag".
    let untagged = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", albums["No Tags"].id),
    )
    .await;
    assert_eq!(untagged["subsonic-response"]["album"]["musicBrainzId"], "");
    let artist_response = subsonic_json(
        &router,
        "getArtist",
        api_key,
        &format!("&id={}", artist.artist.id),
    )
    .await;
    assert_eq!(
        artist_response["subsonic-response"]["artist"]["musicBrainzId"],
        "artist-vale"
    );

    // getAlbumInfo predates the presence rule and its members are elements, so
    // the untagged album omits it rather than sending an empty one.
    let info = subsonic_json(
        &router,
        "getAlbumInfo2",
        api_key,
        &format!("&id={}", reissue.id),
    )
    .await;
    assert_eq!(
        info["subsonic-response"]["albumInfo2"]["musicBrainzId"],
        "release-reissue"
    );
    let info = subsonic_json(
        &router,
        "getAlbumInfo2",
        api_key,
        &format!("&id={}", albums["No Tags"].id),
    )
    .await;
    assert!(info["subsonic-response"]["albumInfo2"]
        .get("musicBrainzId")
        .is_none());

    // On a directory child the specification defines musicBrainzId as the
    // recording, and a folder standing for a release has no recording, so the
    // browse view drops it rather than putting a release id under that name.
    let directory = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={}", artist.artist.id),
    )
    .await;
    let children = directory["subsonic-response"]["directory"]["child"]
        .as_array()
        .expect("the artist directory lists its albums");
    assert!(!children.is_empty());
    for child in children {
        assert!(
            child.get("musicBrainzId").is_none(),
            "a browsing entry claimed a recording id: {child}"
        );
    }

    // A tag removed from the files has to disappear from the catalogue: the
    // derivation runs after every scan, so it clears as readily as it sets.
    sqlx::query("UPDATE track SET musicbrainz_release_id = NULL WHERE library_id = ?")
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    // Read from the full list, not from a map keyed on the title: two records
    // share the title "Split Sky", so a map would collapse them and check one.
    assert!(state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap()
        .iter()
        .all(|album| album.musicbrainz_id.is_none()));
}

/// A genre is one thing or it is nothing. `getGenres` folds spelling variants
/// into one row, so the method that lists a genre's songs has to fold them the
/// same way — otherwise a client displays a genre it was just handed and finds
/// it empty.
#[tokio::test]
async fn genre_matching_is_canonical_on_every_surface() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_genre-key";
    let owner = state
        .db
        .create_account(
            "genre-owner",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"genre-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("genre-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Genres",
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
    state.db.start_scan_job(scan, 3, false).await.unwrap();

    // The same genre, spelled three ways across three files. Canonicalisation
    // folds case, punctuation and spacing, so all three are one genre.
    for (index, (title, genre)) in [
        ("Boom", "Hip-Hop"),
        ("Bap", "hip hop"),
        ("Clap", "HIP  HOP"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(
            400 + index,
            title,
            "Cipher",
            "Nine Mics",
            Some(index as i64 + 1),
            Some(1),
        );
        input.relative_path = format!("genre-{index}.flac");
        input.quick_hash = format!("{:064x}", index + 41_000);
        input.full_hash = format!("{:064x}", index + 42_000);
        input.genre = Some(genre.into());
        input.year = Some(2001);
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    // The album's own credits are derived at the end of a scan, like the
    // identifiers and the sort names: a test driving the catalogue directly
    // runs the same pass the scanner runs.
    state
        .db
        .consolidate_catalog_derivations(library)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());

    // One row, holding all three tracks.
    let genres = subsonic_json(&router, "getGenres", api_key, "").await;
    let listed = genres["subsonic-response"]["genres"]["genre"]
        .as_array()
        .expect("genres is an array");
    assert_eq!(listed.len(), 1, "one genre, three spellings: {listed:?}");
    assert_eq!(listed[0]["songCount"], 3);
    let name = listed[0]["value"]
        .as_str()
        .expect("the genre carries its name")
        .to_owned();

    // Asking for the name the server just gave must return all three, and so
    // must each of the other spellings: they are the same genre.
    for spelling in [name.as_str(), "Hip-Hop", "hip hop", "HIP  HOP"] {
        let encoded = spelling.replace(' ', "%20");
        let songs = subsonic_json(
            &router,
            "getSongsByGenre",
            api_key,
            &format!("&genre={encoded}&count=50"),
        )
        .await;
        let entries = songs["subsonic-response"]["songsByGenre"]["song"]
            .as_array()
            .unwrap_or_else(|| panic!("no songs for {spelling}: {songs}"));
        assert_eq!(entries.len(), 3, "{spelling}");
    }

    // getRandomSongs applies the same rule, and its year filter still narrows.
    let random = subsonic_json(&router, "getRandomSongs", api_key, "&genre=Hip-Hop&size=50").await;
    assert_eq!(
        random["subsonic-response"]["randomSongs"]["song"]
            .as_array()
            .expect("randomSongs is an array")
            .len(),
        3
    );
    let out_of_range = subsonic_json(
        &router,
        "getRandomSongs",
        api_key,
        "&genre=Hip-Hop&size=50&fromYear=2010&toYear=2020",
    )
    .await;
    assert!(out_of_range["subsonic-response"]["randomSongs"]
        .get("song")
        .is_none());
    // A reversed range is how Subsonic asks for one, not an empty request.
    let reversed = subsonic_json(
        &router,
        "getRandomSongs",
        api_key,
        "&genre=Hip-Hop&size=50&fromYear=2005&toYear=1999",
    )
    .await;
    assert_eq!(
        reversed["subsonic-response"]["randomSongs"]["song"]
            .as_array()
            .expect("randomSongs is an array")
            .len(),
        3
    );

    // A genre nobody uses is an empty list, not an error.
    let unknown = subsonic_json(&router, "getSongsByGenre", api_key, "&genre=Polka").await;
    assert_eq!(unknown["subsonic-response"]["status"], "ok");
    assert!(unknown["subsonic-response"]["songsByGenre"]
        .get("song")
        .is_none());

    // The album filter already matched canonically, and still agrees.
    let by_genre = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=byGenre&genre=hip%20hop&size=10",
    )
    .await;
    assert_eq!(
        by_genre["subsonic-response"]["albumList2"]["album"]
            .as_array()
            .expect("albumList2 is an array")
            .len(),
        1
    );

    // Paging getSongsByGenre no longer slices a full catalogue read, and the
    // page boundaries still line up.
    let first = subsonic_json(
        &router,
        "getSongsByGenre",
        api_key,
        "&genre=Hip-Hop&count=2&offset=0",
    )
    .await;
    let second = subsonic_json(
        &router,
        "getSongsByGenre",
        api_key,
        "&genre=Hip-Hop&count=2&offset=2",
    )
    .await;
    assert_eq!(
        first["subsonic-response"]["songsByGenre"]["song"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        second["subsonic-response"]["songsByGenre"]["song"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // The native surface answers the same two questions the facade does, on
    // the same services: an asymmetry the audit left open, where the query
    // existed and only the HTTP adapter was missing.
    let login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "genre-owner",
                "password": "correct horse battery staple",
                "device_name": "Integration"
            }),
        ))
        .await
        .unwrap();
    let access = json_body(login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let native = |path: String| {
        let router = router.clone();
        let access = access.clone();
        async move {
            router
                .oneshot(
                    Request::get(path)
                        .header("authorization", format!("Bearer {access}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    // Any spelling reaches the same three tracks, natively too.
    let by_genre = json_body(native("/api/v2/songs?genre=hip%20hop&limit=50".into()).await).await;
    assert_eq!(by_genre.as_array().expect("a list").len(), 3);
    let random =
        json_body(native("/api/v2/songs/random?genre=HIP%20%20HOP&limit=50".into()).await).await;
    assert_eq!(random.as_array().expect("a list").len(), 3);
    // The year filter narrows, and a genre nobody uses is empty rather than
    // an error.
    let narrowed = json_body(
        native("/api/v2/songs/random?genre=Hip-Hop&limit=50&from_year=2010&to_year=2020".into())
            .await,
    )
    .await;
    assert!(narrowed.as_array().expect("a list").is_empty());
    let unused = json_body(native("/api/v2/songs?genre=Polka".into()).await).await;
    assert!(unused.as_array().expect("a list").is_empty());
    // The genre is what the request is about, so its absence is a malformed
    // request and not an unfiltered catalogue.
    assert_eq!(
        native("/api/v2/songs".into()).await.status(),
        StatusCode::BAD_REQUEST
    );
    // Search pages each kind on its own offset.
    let paged =
        json_body(native("/api/v2/search?q=Boom&limit=10&song_offset=5".into()).await).await;
    assert!(paged["songs"].as_array().expect("songs").is_empty());
    assert_eq!(paged["albums"].as_array().expect("albums").len(), 1);

    // The album carries the credits and genres of its tracks, folded the same
    // way, which is what AlbumID3 asks for.
    let albums = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    let album = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", albums.albums[0].id),
    )
    .await;
    let album = &album["subsonic-response"]["album"];
    assert_eq!(
        album["genres"]
            .as_array()
            .expect("album genres is an array")
            .len(),
        1,
        "three spellings should be one album genre: {album}"
    );
    assert_eq!(album["artists"][0]["name"], "Nine Mics");
    // And a track names the album's credit beside its own.
    assert_eq!(album["song"][0]["displayAlbumArtist"], "Nine Mics");
    assert_eq!(album["song"][0]["albumArtists"][0]["name"], "Nine Mics");
}

/// The separators the reference cuts on, reaching the catalogue.
///
/// The old rule cut on `;` and nothing else, so a file crediting
/// "Nova Kern / Lior Sand" held one artist named after the whole string. The
/// new one cuts on a padded slash — padded so that `AC/DC`, which is one band,
/// survives it.
#[tokio::test]
async fn the_catalogue_cuts_a_credit_where_the_reference_cuts_it() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("separators", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("separator-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Separators",
            &root,
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
    state.db.start_scan_job(scan, 3, false).await.unwrap();

    for (index, credit, album) in [
        (0usize, "Nova Kern / Lior Sand", "Split By Slash"),
        (1, "AC/DC", "Kept Whole"),
        (2, "Ada Vale feat. Nova Kern", "Split By Feat"),
    ] {
        let mut input = catalog_input(index, credit);
        input.title = format!("Track {index}");
        input.album = Some(album.into());
        input.album_artist = Some(credit.into());
        input.is_compilation = false;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .into_iter()
        .map(|summary| summary.artist.name)
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "AC/DC".to_owned(),
            "Ada Vale".to_owned(),
            "Lior Sand".to_owned(),
            "Nova Kern".to_owned(),
        ],
        "a padded slash cuts, a bare one inside a name does not, and `feat.` cuts"
    );
}

#[tokio::test]
async fn the_participant_schema_replaced_the_single_artist_relation() {
    let (_temp, _config, state) = test_app().await;
    let table_exists = |name: &'static str| {
        let pool = state.db.pool().clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            )
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap()
                == 1
        }
    };
    assert!(table_exists("track_participant").await);
    assert!(table_exists("album_participant").await);
    assert!(table_exists("artist_role_stats").await);
    assert!(
        !table_exists("track_artist").await,
        "the single-artist relation is gone, not shadowed"
    );
}

/// A producer is a credit, not an artist of the track.
///
/// This is the failure the participants model makes possible and no existing
/// test could have anticipated: widening `track_participant` to hold every
/// role, and leaving one projection without its role predicate, does not turn
/// the suite red — it leaves it green while every song reports its producer
/// among its artists, and every album reports them among its own.
#[tokio::test]
async fn a_contributor_is_not_one_of_the_track_artists() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("contributors", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("contributor-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Credits",
            &root,
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
    state.db.start_scan_job(scan, 1, false).await.unwrap();

    let mut input = catalog_input(0, "Nova Kern");
    input.title = "Only Track".into();
    input.album = Some("Only Album".into());
    input.album_artist = Some("Nova Kern".into());
    input.is_compilation = false;
    input.roles = vec![
        (
            waveflow_server::tags::Role::Producer,
            vec!["Rita Sound".into()],
        ),
        (
            waveflow_server::tags::Role::Composer,
            vec!["Otto Pen".into()],
        ),
    ];
    input.performer_pairs = vec![("guitar".into(), "Jimmy Page".into())];
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

    // Every credit reached the catalogue...
    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .iter()
        .map(|summary| summary.artist.name.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "Jimmy Page".to_owned(),
            "Nova Kern".to_owned(),
            "Otto Pen".to_owned(),
            "Rita Sound".to_owned(),
        ]
    );

    // ...and none of them is one of the track's artists.
    let songs = state
        .services
        .songs_without_album(owner, library, 100)
        .await
        .unwrap();
    let song = state
        .services
        .album(owner, {
            let albums = state
                .services
                .list_albums(owner, &Default::default())
                .await
                .unwrap();
            albums[0].id
        })
        .await
        .unwrap();
    assert!(songs.is_empty(), "the track belongs to an album");
    let track = &song.songs[0];
    assert_eq!(
        track
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Nova Kern"],
        "artists[] holds the track's own credit and nothing else"
    );
    assert_eq!(
        song.album
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Nova Kern"],
        "an album's artists[] holds its album artists, not every contributor"
    );
    let credited = state
        .services
        .artist(
            owner,
            artists
                .iter()
                .find(|summary| summary.artist.name == "Rita Sound")
                .expect("the producer was indexed")
                .artist
                .id,
        )
        .await
        .unwrap();
    assert_eq!(
        credited.album_count, 0,
        "a producer holds no album of their own"
    );
    assert!(credited.albums.is_empty());

    // ...and none of them answers to a query that names none of them.
    //
    // The artist half of a search used to be derived rather than matched:
    // it took the tracks the full-text index had found and returned
    // everybody credited on them. Searching a title therefore returned the
    // whole session crew, and the participants model made that worse — a
    // track carries thirteen roles now where it carried one list of names.
    let by_title = state
        .services
        .catalog_search(
            owner,
            &[],
            "Only Track",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        by_title.songs.len(),
        1,
        "the title still finds the track it names"
    );
    assert!(
        by_title.artists.is_empty(),
        "a track title names no artist, so it returns none: {:?}",
        by_title
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>()
    );

    // The producer is still reachable — by their own name, which is the
    // whole point of indexing artists rather than deriving them.
    let by_name = state
        .services
        .catalog_search(
            owner,
            &[],
            "Rita",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        by_name
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Rita Sound"]
    );
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

/// A track no scan ever stamped is not a track a scan failed to find.
///
/// `apply_catalog_track_in_transaction` writes a NULL scan for a row that did
/// not come from a scan — a received file. A scan that starts its walk before
/// that file exists cannot find it, and sweeping it would mark a file that is
/// on disk as gone and announce a deletion to every client. A row that predates
/// the scan is the other case: the walk should have found it, and it is swept
/// as before.
#[tokio::test]
async fn a_row_that_never_came_from_a_scan_is_swept_only_if_it_predates_it() {
    let (_temp, config, state) = test_app().await;
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("unstamped", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("unstamped");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Unstamped",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    let seed = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(seed, 2, false).await.unwrap();
    let mut older = browse_input(
        90_001,
        "Older",
        "Unstamped album",
        "Unstamped artist",
        Some(1),
        Some(1),
    );
    older.full_hash = format!("{:064x}", 0x9001);
    let mut newer = browse_input(
        90_002,
        "Newer",
        "Unstamped album",
        "Unstamped artist",
        Some(2),
        Some(1),
    );
    newer.full_hash = format!("{:064x}", 0x9002);
    for input in [&older, &newer] {
        state
            .db
            .apply_catalog_track(library, seed, input, None, false)
            .await
            .unwrap();
    }

    // The scan that will do the sweeping, and the moment it began.
    let sweeper = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(sweeper, 0, false).await.unwrap();
    let started_at: i64 = sqlx::query_scalar("SELECT started_at FROM scan_job WHERE id=?")
        .bind(sweeper.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();

    // Both rows lose their scan stamp — the state a received file is written
    // in — and are placed either side of the moment the walk began.
    sqlx::query(
        "UPDATE track SET last_seen_scan_id=NULL, created_at=? \
         WHERE library_id=? AND full_hash=?",
    )
    .bind(started_at - 1)
    .bind(library.to_string())
    .bind(format!("{:064x}", 0x9001))
    .execute(state.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "UPDATE track SET last_seen_scan_id=NULL, created_at=? \
         WHERE library_id=? AND full_hash=?",
    )
    .bind(started_at + 1)
    .bind(library.to_string())
    .bind(format!("{:064x}", 0x9002))
    .execute(state.db.pool())
    .await
    .unwrap();

    let swept = state
        .db
        .mark_unseen_unavailable(library, sweeper)
        .await
        .unwrap();

    assert_eq!(swept, 1, "only the row that predates the walk is gone");
    let still_here: i64 =
        sqlx::query_scalar("SELECT is_available FROM track WHERE library_id=? AND full_hash=?")
            .bind(library.to_string())
            .bind(format!("{:064x}", 0x9002))
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(
        still_here, 1,
        "a file that landed after the walk began is not one the walk failed to find"
    );
    let predating: i64 =
        sqlx::query_scalar("SELECT is_available FROM track WHERE library_id=? AND full_hash=?")
            .bind(library.to_string())
            .bind(format!("{:064x}", 0x9001))
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(
        predating, 0,
        "a row the walk should have found is still swept"
    );

    // And the sweep must have said so on the feed, once.
    let deletes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM library_event \
         WHERE library_id=? AND action='delete' AND entity_type='track'",
    )
    .bind(library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(deletes, 1);
}

/// RFC-007 left open whether an album needs an event of its own; the desktop
/// named the case that says it does, and this is the shape of the answer.
#[tokio::test]
async fn an_album_that_is_merely_retagged_announces_itself_once() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("album-feed", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("album-feed-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Album feed",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    // Three tracks of one album and one of another. What the feed must not do
    // is announce the first album three times.
    // `existing` is what a rescan carries: the apply matches a file to the row
    // it already has, and passing `None` a second time would try to insert a
    // second track at a path the schema makes unique.
    let apply =
        |index: usize, album: &'static str, year: Option<i64>, existing: Option<uuid::Uuid>| {
            let db = state.db.clone();
            async move {
                let scan = db
                    .create_scan_job(library, Some(owner), "manual")
                    .await
                    .unwrap();
                db.start_scan_job(scan, 1, false).await.unwrap();
                let mut input = catalog_input(index, "Nova Kern");
                input.title = format!("Track {index}");
                input.album = Some(album.into());
                input.album_artist = Some("Nova Kern".into());
                input.year = year;
                db.apply_catalog_track(library, scan, &input, existing, false)
                    .await
                    .unwrap();
                db.finish_scan_job(scan, 0).await.unwrap();
            }
        };
    for index in 0..3 {
        apply(index, "One", Some(2000), None).await;
    }
    apply(3, "Two", Some(2000), None).await;
    let first = state
        .db
        .list_tracks_for_user(owner, library)
        .await
        .unwrap()
        .into_iter()
        .find(|track| track.title == "Track 0")
        .expect("the first track")
        .id;

    let albums_on_feed = |after: i64| {
        let services = state.services.clone();
        async move {
            services
                .library_changes(owner, library, after, 500)
                .await
                .unwrap()
                .events
                .into_iter()
                .filter(|event| event.entity_type == "album")
                .collect::<Vec<_>>()
        }
    };

    let albums = albums_on_feed(0).await;
    assert_eq!(
        albums.len(),
        2,
        "one event per album, not one per track: {albums:?}"
    );
    assert!(albums.iter().all(|event| event.action == "upsert"));
    let cursor = state
        .services
        .library_changes(owner, library, 0, 500)
        .await
        .unwrap()
        .events
        .last()
        .unwrap()
        .cursor;

    // Applying the same tags again writes nothing, so it announces nothing. An
    // album event per rescan would make the feed a scan log and cost the client
    // the refetch this event exists to spare it.
    apply(0, "One", Some(2000), Some(first)).await;
    assert!(
        albums_on_feed(cursor).await.is_empty(),
        "an unchanged album is not a change"
    );

    // And the case the desktop named: the album is retagged without gaining or
    // losing a track, so `song_count` does not move and an incremental walk
    // keyed on it would skip the album entirely.
    apply(0, "One", Some(2001), Some(first)).await;
    let announced = albums_on_feed(cursor).await;
    assert_eq!(announced.len(), 1, "the retag is announced: {announced:?}");
    assert_eq!(announced[0].action, "upsert");
}

/// RFC-007 decision 7: an age and a floor, the floor winning, and the age bound
/// exclusive. Nothing purged before this, so the feed's expiry answer was
/// correct code that had never once run.
#[tokio::test]
async fn retention_cuts_by_age_never_below_the_floor_and_never_at_the_bound() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    // Tuned before `initialize`: `DomainServices` copies these out of `Config`
    // when it is built, so raising a bound on the returned one changes nothing.
    config.library_event_retention.min_events = 2;
    config.library_event_retention.days = 30;
    let state = waveflow_server::initialize(&config).await.unwrap();

    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("retention", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("retention-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Retention",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    // Five events through the ordinary path. No album on the inputs, so each
    // track writes one event and nothing else — an album upsert emits its own
    // since #159, and a count this test reasons about must not include them.
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 5, false).await.unwrap();
    for index in 0..5usize {
        let mut input = catalog_input(index, "Nova Kern");
        input.title = format!("Track {index}");
        input.album = None;
        input.album_artist = None;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let cursors: Vec<i64> =
        sqlx::query_scalar("SELECT cursor FROM library_event WHERE library_id=? ORDER BY cursor")
            .bind(library.to_string())
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert_eq!(cursors.len(), 5, "one event per track and no album events");

    // Backdated in SQL because the ages are what this test is about: two far
    // past the bound, one exactly on it, two inside.
    //
    // `now` is handed to the purge rather than read by it. The bound is
    // exclusive, so "exactly thirty days old" is a single instant: read the
    // clock twice and the boundary event lands a millisecond on the wrong side,
    // and this test passes or fails on how long the lines above it took.
    let day = 24 * 60 * 60 * 1000i64;
    let now = now_ms();
    for (cursor, age) in cursors
        .iter()
        .zip([40 * day, 35 * day, 30 * day, 10 * day, 0])
    {
        sqlx::query("UPDATE library_event SET changed_at=? WHERE cursor=?")
            .bind(now - age)
            .bind(cursor)
            .execute(state.db.pool())
            .await
            .unwrap();
    }

    let purged = state.services.purge_library_events(now).await.unwrap();
    assert_eq!(purged.libraries_trimmed, 1);
    // Two go. The thirty-day one sits exactly on the bound and the bound is
    // exclusive — keeping one event too many breaks nobody, cutting one too
    // many sends somebody back to the snapshot.
    assert_eq!(purged.events_removed, 2, "only what is strictly older");

    // The watermark moved with the delete, in the same transaction. Written
    // separately there is a window where it claims less than has gone, and a
    // client reading into it gets a catch-up that looks complete while skipping
    // the gap.
    let watermark: i64 = sqlx::query_scalar("SELECT events_purged_through FROM library WHERE id=?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(watermark, cursors[1], "the highest cursor actually cut");
    assert_eq!(
        state
            .services
            .library_changes(owner, library, watermark, 500)
            .await
            .unwrap()
            .events
            .len(),
        3,
        "the bound and everything after it stay"
    );
    assert!(
        matches!(
            state
                .services
                .library_changes(owner, library, watermark - 1, 500)
                .await,
            Err(ServiceError::Conflict)
        ),
        "a cursor below the watermark has missed events"
    );

    // And the floor wins over the age. Everything left is now ancient, and the
    // floor is two: exactly one may go.
    sqlx::query("UPDATE library_event SET changed_at=? WHERE library_id=?")
        .bind(now - 400 * day)
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let purged = state.services.purge_library_events(now).await.unwrap();
    assert_eq!(
        purged.events_removed, 1,
        "three rows, a floor of two: one may go however old they all are"
    );

    // A second pass changes nothing: the floor is reached and stays reached.
    assert_eq!(
        state.services.purge_library_events(now).await.unwrap(),
        Default::default(),
        "a library at its floor is not trimmed again"
    );
    let watermark: i64 = sqlx::query_scalar("SELECT events_purged_through FROM library WHERE id=?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        state
            .services
            .library_changes(owner, library, watermark, 500)
            .await
            .unwrap()
            .events
            .len(),
        2,
        "the floor kept a usable tail rather than an empty feed"
    );
}
