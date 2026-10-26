//! What the catalogue derives from the tags of many tracks: release identifiers
//! and canonical genres.

use super::*;

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
    let by_genre =
        json_body(native("/api/v2/songs/by-genre?genre=hip%20hop&limit=50".into()).await).await;
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
    let unused = json_body(native("/api/v2/songs/by-genre?genre=Polka".into()).await).await;
    assert!(unused.as_array().expect("a list").is_empty());
    // The genre is what the request is about, so its absence is a malformed
    // request and not an unfiltered catalogue.
    assert_eq!(
        native("/api/v2/songs/by-genre".into()).await.status(),
        StatusCode::BAD_REQUEST
    );
    // `/songs` is now the general listing, and it folds a genre the same way
    // as every surface above.
    //
    // It used to answer 404, asserted here, because #179 was closed by
    // renaming it: it read as the general listing while answering 400, and
    // that is what sent a reader looking for a bug. The rename made the
    // contract honest without making the listing exist. It exists now, so the
    // assertion says what is true rather than being deleted — and says it on
    // this test's own subject, which is that one genre reaches the same tracks
    // by any spelling, on any route.
    let listed =
        json_body(native("/api/v2/songs?sort=byGenre&genre=hip%20hop&limit=50".into()).await).await;
    assert_eq!(listed.as_array().expect("a list").len(), 3);
    // This fixture holds three tracks and all three are that one genre, so the
    // line above would answer three even from a filter that was dropped. A
    // genre nobody uses is what separates the two here.
    let unused_here =
        json_body(native("/api/v2/songs?sort=byGenre&genre=Polka&limit=50".into()).await).await;
    assert!(unused_here.as_array().expect("a list").is_empty());
    // And unfiltered it is a listing, not a refusal.
    assert_eq!(
        native("/api/v2/songs".into()).await.status(),
        StatusCode::OK
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
