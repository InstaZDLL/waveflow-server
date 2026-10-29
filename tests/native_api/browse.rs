//! Browsing the catalogue: paging, search, tenancy and the technical fields of
//! a track.

use super::*;

#[tokio::test]
async fn native_browse_endpoints_page_search_and_isolate_tenants() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("browse-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("browse-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("browse-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Browse",
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
    // Tracks are applied out of sleeve order on purpose: the album drill-down
    // must sort them, not echo insertion order.
    for (index, (title, album, artist, track, disc)) in [
        (
            "Slow Tide",
            "Aurora Fields",
            "Lumen Drift",
            Some(2),
            Some(1),
        ),
        (
            "First Light",
            "Aurora Fields",
            "Lumen Drift",
            Some(1),
            Some(1),
        ),
        // Incomplete tags are common in real libraries and must not jump ahead.
        ("Hidden Track", "Aurora Fields", "Lumen Drift", None, None),
        (
            "Rivière Noire",
            "Nocturne Bleue",
            "Écho Solaire",
            Some(2),
            Some(1),
        ),
        (
            "Prélude",
            "Nocturne Bleue",
            "Écho Solaire",
            Some(1),
            Some(1),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(index, title, album, artist, track, disc);
        match title {
            // The album still has an artist, but this track has no credited
            // artist and therefore no track_artist row.
            "Hidden Track" => {
                input.artist = None;
                input.album_artist = Some("Lumen Drift".into());
            }
            // Materializes positions 0 and 1 while preserving the existing
            // album identity. The public primary must remain Écho Solaire.
            "Prélude" => {
                input.artist = Some("Écho Solaire; Lumen Drift".into());
                input.album_artist = Some("Écho Solaire".into());
            }
            _ => {}
        }
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state);
    let owner_token = login_token(&router, "browse-owner", password).await;
    let intruder_token = login_token(&router, "browse-intruder", password).await;

    let get = |uri: String, token: String| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::get(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    // Albums are ordered by title, so Aurora Fields precedes Nocturne Bleue.
    let albums = get("/api/v2/albums".into(), owner_token.clone()).await;
    assert_eq!(albums.status(), StatusCode::OK);
    let albums = json_body(albums).await;
    let albums = albums.as_array().unwrap();
    assert_eq!(albums.len(), 2);
    assert_eq!(albums[0]["title"], "Aurora Fields");
    assert_eq!(albums[1]["title"], "Nocturne Bleue");

    // Paging is applied in SQL, not after the fact.
    let page = get(
        "/api/v2/albums?limit=1&offset=1".into(),
        owner_token.clone(),
    )
    .await;
    let page = json_body(page).await;
    assert_eq!(page.as_array().unwrap().len(), 1);
    assert_eq!(page[0]["title"], "Nocturne Bleue");

    // The paging ceiling matches the Subsonic contract's 500-item cap.
    let rejected = get("/api/v2/albums?limit=501".into(), owner_token.clone()).await;
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let album_id = albums[0]["id"].as_str().unwrap().to_owned();
    let detail = get(format!("/api/v2/albums/{album_id}"), owner_token.clone()).await;
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = json_body(detail).await;
    assert_eq!(detail["title"], "Aurora Fields");
    let album_artist_id = detail["artist_id"]
        .as_str()
        .expect("the album artist has a public id")
        .to_owned();
    let songs = detail["songs"].as_array().unwrap();
    assert_eq!(songs.len(), 3);
    assert_eq!(songs[0]["artist_id"], album_artist_id);
    assert_eq!(songs[1]["artist_id"], album_artist_id);
    assert!(songs[2]["artist_id"].is_null());
    assert_eq!(songs[0]["title"], "First Light", "sleeve order wins");
    assert_eq!(songs[1]["title"], "Slow Tide");
    assert_eq!(
        songs[2]["title"], "Hidden Track",
        "an untagged track sorts last, not ahead of track 1"
    );

    let artists = get("/api/v2/artists".into(), owner_token.clone()).await;
    let artists = json_body(artists).await;
    let artists = artists.as_array().unwrap();
    assert_eq!(artists.len(), 2);
    let echo = artists
        .iter()
        .find(|artist| artist["name"] == "Écho Solaire")
        .expect("accented artist is listed");
    assert_eq!(echo["album_count"], 1);

    let artist_id = echo["id"].as_str().unwrap().to_owned();
    let detail = get(format!("/api/v2/artists/{artist_id}"), owner_token.clone()).await;
    let detail = json_body(detail).await;
    assert_eq!(detail["name"], "Écho Solaire");
    assert_eq!(detail["albums"].as_array().unwrap().len(), 1);
    assert_eq!(detail["albums"][0]["title"], "Nocturne Bleue");

    // The same albums on their own, without the page around them: what an
    // album page reads to list the artist's other records.
    let albums_of = get(
        format!("/api/v2/artists/{artist_id}/albums"),
        owner_token.clone(),
    )
    .await;
    assert_eq!(albums_of.status(), StatusCode::OK);
    let albums_of = json_body(albums_of).await;
    assert_eq!(
        albums_of, detail["albums"],
        "the page's albums, one for one"
    );

    // FTS5 folds diacritics, so an unaccented query still reaches "Écho Solaire".
    let found = get("/api/v2/search?q=echo".into(), owner_token.clone()).await;
    assert_eq!(found.status(), StatusCode::OK);
    let found = json_body(found).await;
    // An artist answers for their own name and nobody else's. "Lumen Drift"
    // shares a track with "Écho Solaire" and is credited on the songs this
    // query returns, which is exactly what used to put it here: the artist
    // half of a search was derived from the matching tracks rather than
    // matched, so searching one name returned everyone who had played with
    // its owner.
    let names = found["artists"]
        .as_array()
        .unwrap()
        .iter()
        .map(|artist| artist["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, vec!["Écho Solaire"]);
    assert_eq!(found["albums"].as_array().unwrap().len(), 1);
    assert_eq!(found["albums"][0]["title"], "Nocturne Bleue");
    assert_eq!(found["songs"].as_array().unwrap().len(), 2);
    let found_artist_id = found["artists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artist| artist["name"] == "Écho Solaire")
        .expect("the primary artist is included in search results")["id"]
        .as_str()
        .unwrap();
    for title in ["Prélude", "Rivière Noire"] {
        let song = found["songs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|song| song["title"] == title)
            .unwrap_or_else(|| panic!("missing search result {title}"));
        assert_eq!(song["artist_id"], found_artist_id, "{title}");
    }

    // The library projection uses TrackRecord rather than SongItem, but keeps
    // the same artist link so a client can open an artist from every track list.
    let tracks = get(
        format!("/api/v2/libraries/{library_id}/tracks"),
        owner_token.clone(),
    )
    .await;
    let tracks = json_body(tracks).await;
    assert_eq!(tracks.as_array().unwrap().len(), 5);
    let track = |title: &str| {
        tracks
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["title"] == title)
            .unwrap_or_else(|| panic!("missing library track {title}"))
    };
    assert_eq!(track("First Light")["artist_id"], album_artist_id);
    assert_eq!(track("Slow Tide")["artist_id"], album_artist_id);
    assert!(track("Hidden Track")["artist_id"].is_null());
    assert_eq!(track("Prélude")["artist_id"], artist_id);
    assert_eq!(track("Rivière Noire")["artist_id"], artist_id);

    // One track carries where its file sits, relative to the library, as the
    // library's own listing spells it; the album's song rows do not, so a
    // listing is not a path per row nobody browsing albums asked for.
    let prelude = track("Prélude");
    let single = get(
        format!("/api/v2/tracks/{}", prelude["id"].as_str().unwrap()),
        owner_token.clone(),
    )
    .await;
    assert_eq!(single.status(), StatusCode::OK);
    let single = json_body(single).await;
    assert_eq!(single["title"], "Prélude");
    assert_eq!(single["relative_path"], prelude["relative_path"]);
    let path = single["relative_path"].as_str().unwrap();
    assert!(!path.is_empty() && !path.contains('\\'), "{path}");
    assert!(
        !std::path::Path::new(path).is_absolute(),
        "the library root never leaves the server: {path}"
    );
    let album_songs = get(format!("/api/v2/albums/{album_id}"), owner_token.clone()).await;
    let album_songs = json_body(album_songs).await;
    assert!(album_songs["songs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|song| song.get("relative_path").is_none()));

    // Search-as-you-type: the trailing term matches as a prefix. This exact
    // case was reported from the Android client — "echo" returned songs,
    // albums and artists while "ech" returned nothing, because the native
    // surface still required whole tokens after the Subsonic one moved on.
    let partial = get("/api/v2/search?q=ech".into(), owner_token.clone()).await;
    assert_eq!(partial.status(), StatusCode::OK);
    let partial = json_body(partial).await;
    assert_eq!(partial["songs"].as_array().unwrap().len(), 2);
    assert_eq!(partial["albums"].as_array().unwrap().len(), 1);
    assert_eq!(partial["artists"].as_array().unwrap().len(), 1);

    // Extra terms still narrow rather than widen.
    let narrowed = get(
        "/api/v2/search?q=echo%20nonexistent".into(),
        owner_token.clone(),
    )
    .await;
    assert!(json_body(narrowed).await["songs"]
        .as_array()
        .unwrap()
        .is_empty());

    // A search with no usable term is an empty result, never a SQL error.
    let blank = get("/api/v2/search?q=%20".into(), owner_token.clone()).await;
    assert_eq!(blank.status(), StatusCode::OK);
    let blank = json_body(blank).await;
    assert!(blank["songs"].as_array().unwrap().is_empty());

    // A foreign tenant sees an empty catalogue and cannot probe ids.
    let foreign = get("/api/v2/albums".into(), intruder_token.clone()).await;
    assert_eq!(foreign.status(), StatusCode::OK);
    assert!(json_body(foreign).await.as_array().unwrap().is_empty());
    for uri in [
        format!("/api/v2/albums/{album_id}"),
        format!("/api/v2/artists/{artist_id}"),
        format!("/api/v2/artists/{artist_id}/albums"),
        format!("/api/v2/artists/{}/albums", uuid::Uuid::new_v4()),
    ] {
        let response = get(uri, intruder_token.clone()).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "foreign ids must not be distinguishable from missing ones"
        );
    }
    let foreign_search = get("/api/v2/search?q=echo".into(), intruder_token).await;
    let foreign_search = json_body(foreign_search).await;
    assert!(foreign_search["artists"].as_array().unwrap().is_empty());
    assert!(foreign_search["songs"].as_array().unwrap().is_empty());

    // Anonymous access is rejected before any catalogue work happens.
    let anonymous = router
        .clone()
        .oneshot(Request::get("/api/v2/albums").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

/// Every technical field reaches the wire, and says what the file says.
///
/// #211 reports these as never serialised. **That is no longer true**, and
/// checking rather than believing it changed what this test had to be: they
/// are all on `SongItem`, which derives `Serialize` with nothing skipped, and
/// `docs/guides/api-v2-guide.md` already names three of them. What was missing is
/// this — nothing asserted any of it, so the client could declare a field the
/// server had quietly stopped sending and the whole suite would agree with
/// the client.
///
/// Which is the trap the issue names: the client's types are hand-written
/// assertions about this wire, and its mocks are built from those types. A
/// test on that side confirms the type. So the values here are read **out of
/// the file on disk** — the WAV header's own bytes and the size the
/// filesystem reports — and compared with what the server answered. Nothing
/// is written down twice.
#[tokio::test]
async fn the_technical_fields_reach_the_wire_and_match_the_file() {
    let (_temp, config, state) = test_app().await;
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("fields-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("fields-music");
    std::fs::create_dir_all(&music).unwrap();
    let file = music.join("Properties.wav");
    write_test_wav(&file);
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Technical fields",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Technical fields".into(),
            root_path: root,
        },
    )
    .await;
    let track = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()
        .pop()
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "fields-owner", &password).await;
    let response = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{}", track.id))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let song = json_body(response).await;

    // The fmt chunk, at the offsets a canonical WAV puts it: channels at 22,
    // the sample rate at 24, the bits per sample at 34. Read here so the
    // assertions below belong to the file rather than to a number somebody
    // typed into both sides.
    let bytes = std::fs::read(&file).unwrap();
    // Every offset below is checked against its marker first. A fixture that
    // grows a chunk — a `LIST` before the data, say — would otherwise have
    // this test reading two arbitrary bytes and failing on a number, which
    // says the server is wrong about a file that moved underneath it.
    for (at, marker) in [
        (0usize, &b"RIFF"[..]),
        (8, b"WAVE"),
        (12, b"fmt "),
        (36, b"data"),
    ] {
        assert_eq!(
            &bytes[at..at + 4],
            marker,
            "the WAV fixture no longer puts {} at byte {at}",
            String::from_utf8_lossy(marker)
        );
    }
    let channels = u16::from_le_bytes([bytes[22], bytes[23]]) as i64;
    let sample_rate = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]) as i64;
    let bit_depth = u16::from_le_bytes([bytes[34], bytes[35]]) as i64;

    assert_eq!(song["sample_rate"].as_i64(), Some(sample_rate));
    assert_eq!(song["channels"].as_i64(), Some(channels));
    assert_eq!(song["bit_depth"].as_i64(), Some(bit_depth));
    assert_eq!(
        song["size"].as_i64(),
        Some(std::fs::metadata(&file).unwrap().len() as i64)
    );
    assert_eq!(song["suffix"].as_str(), Some("wav"));
    // **Compared with what the catalogue stored, not with the word "WAV".**
    // Pinning the spelling would pin the tag reader's vocabulary: a dependency
    // that renamed its container would fail this test for a reason that has
    // nothing to do with this surface. Comparing against the row is exact and
    // stays about the question — does the stored value reach the wire — and it
    // catches what a non-empty check cannot, a column carrying another track's
    // value or the suffix by mistake.
    assert_eq!(song["codec"].as_str(), track.codec.as_deref());
    assert!(
        track
            .codec
            .as_deref()
            .is_some_and(|codec| !codec.is_empty()),
        "the scan stored no codec at all, so the comparison above proves nothing"
    );

    // **Four fields tied together, rather than four numbers each asserted
    // alone.** Uncompressed PCM has one bitrate and it is arithmetic:
    // channels times depth times rate. A projection that dropped one of them,
    // or a scanner that read one from the wrong offset, breaks the equality —
    // where four separate assertions would each still pass against a
    // plausible-looking wrong value.
    let expected_kbps = sample_rate * bit_depth * channels / 1000;
    assert_eq!(song["bitrate"].as_i64(), Some(expected_kbps));

    // And the duration follows from the data chunk at the same rate.
    let data_bytes = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]) as i64;
    let frames = data_bytes / (channels * bit_depth / 8);
    assert_eq!(
        song["duration_ms"].as_i64(),
        Some(frames * 1000 / sample_rate)
    );
}
