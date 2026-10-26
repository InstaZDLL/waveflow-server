//! The browse methods, reading only what they render.

use super::*;

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
