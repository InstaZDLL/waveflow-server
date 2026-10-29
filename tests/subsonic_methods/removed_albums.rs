//! An album whose every file has gone: not found to a read or a write, like
//! the tracks it held, and back under the same identity when they return.

use super::*;

/// The album ids of a JSON list, in order.
fn album_ids(albums: &serde_json::Value) -> Vec<String> {
    albums
        .as_array()
        .map(|albums| {
            albums
                .iter()
                .map(|album| album["id"].as_str().unwrap().to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// An album row outlives its files, because the tracks it holds are kept for
/// their return, and every album read went through a projection that counted
/// the available tracks without asking whether there were any. So an album
/// whose files had all gone stayed in every list, empty, and opened on nothing
/// (reported while fixing the album search, 2026-10-28). Now it is not found
/// anywhere an album is read — the façade's lists, the artist's albums,
/// `getAlbum`, the starred albums, the native list and detail — and a star or
/// a rating on it is refused like one on a removed track. What was written
/// before the files went is kept, and comes back with them.
#[tokio::test]
async fn an_album_whose_files_all_went_is_not_found_and_comes_back() {
    let (_temp, config, state) = test_app().await;
    // Drawn at run time: a literal reaching the hash is what CodeQL reports.
    let password = Uuid::new_v4().to_string();
    let owner = state
        .db
        .create_account(
            "album-owner",
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
    let music = config.data_dir.join("album-music");
    let set_aside = config.data_dir.join("album-set-aside");
    std::fs::create_dir_all(&music).unwrap();
    std::fs::create_dir_all(&set_aside).unwrap();
    // Two tracks in the album that goes, so it is gone only once both are; one
    // in the album that stays, by the same artist, so the artist page is read
    // with an album still on it.
    write_tagged_flac(&music.join("Gone One.flac"), "Gone One", "Vanishing", 440);
    write_tagged_flac(&music.join("Gone Two.flac"), "Gone Two", "Vanishing", 550);
    write_tagged_flac(&music.join("Kept.flac"), "Kept", "Remaining", 660);
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(owner, "Albums", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let record = waveflow_server::storage::catalog::LibraryRecord {
        id: library,
        name: "Albums".into(),
        root_path: root.clone(),
    };
    run_scan(&state, owner, record.clone()).await;
    let album_of = |path: &'static str| {
        let state = state.clone();
        async move {
            let id: String = sqlx::query_scalar("SELECT album_id FROM track WHERE relative_path=?")
                .bind(path)
                .fetch_one(state.db.pool())
                .await
                .unwrap();
            id
        }
    };
    let (gone, kept) = (album_of("Gone One.flac").await, album_of("Kept.flac").await);
    assert_eq!(album_of("Gone Two.flac").await, gone);
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "album-owner", &password).await;
    let call = |method: &'static str, extra: String| {
        let router = router.clone();
        let api_key = api_key.clone();
        async move {
            subsonic_json(&router, method, &api_key, &extra).await["subsonic-response"].clone()
        }
    };
    let held = || {
        let state = state.clone();
        let gone = gone.clone();
        async move {
            sqlx::query_as::<_, (String, i64)>(
                "SELECT entity_type, COALESCE((SELECT rating FROM user_rating r \
                   WHERE r.user_id=s.user_id AND r.entity_type='album' AND r.entity_id=s.entity_id), 0) \
                 FROM user_star s WHERE s.user_id=? AND s.entity_type='album' AND s.entity_id=?",
            )
            .bind(owner.to_string())
            .bind(&gone)
            .fetch_all(state.db.pool())
            .await
            .unwrap()
        }
    };

    // Written while the files are there: the data the removal must not cost.
    for (method, extra) in [
        ("star", format!("&albumId={gone}")),
        ("setRating", format!("&id={gone}&rating=4")),
    ] {
        let response = call(method, extra).await;
        assert_eq!(response["status"], "ok", "{method}: {response}");
    }
    let before = held().await;
    assert_eq!(before, vec![("album".to_owned(), 4)]);
    let artist = call("getAlbum", format!("&id={kept}")).await["album"]["artistId"]
        .as_str()
        .unwrap()
        .to_owned();

    // One file out of two leaves the album where it was.
    std::fs::rename(music.join("Gone One.flac"), set_aside.join("Gone One.flac")).unwrap();
    run_scan(&state, owner, record.clone()).await;
    let listed = call("getAlbumList2", "&type=alphabeticalByName".into()).await;
    assert_eq!(
        album_ids(&listed["albumList2"]["album"]),
        vec![kept.clone(), gone.clone()],
        "{listed}"
    );

    std::fs::rename(music.join("Gone Two.flac"), set_aside.join("Gone Two.flac")).unwrap();
    run_scan(&state, owner, record.clone()).await;
    let left: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM album WHERE id=?")
        .bind(&gone)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(left, 1, "the precondition: the album row is kept");

    for list in ["alphabeticalByName", "newest", "starred", "highest"] {
        let listed = call("getAlbumList2", format!("&type={list}")).await;
        let ids = album_ids(&listed["albumList2"]["album"]);
        assert!(!ids.contains(&gone), "getAlbumList2 {list}: {listed}");
        let listed = call("getAlbumList", format!("&type={list}")).await;
        let ids = album_ids(&listed["albumList"]["album"]);
        assert!(!ids.contains(&gone), "getAlbumList {list}: {listed}");
    }
    let page = call("getArtist", format!("&id={artist}")).await;
    assert_eq!(
        album_ids(&page["artist"]["album"]),
        vec![kept.clone()],
        "{page}"
    );
    for method in ["getAlbum", "getMusicDirectory"] {
        let read = call(method, format!("&id={gone}")).await;
        assert_eq!(read["error"]["code"], 70, "{method}: {read}");
    }
    let starred = call("getStarred2", String::new()).await;
    assert!(
        !album_ids(&starred["starred2"]["album"]).contains(&gone),
        "{starred}"
    );

    // A write on it is refused, and writes nothing.
    for (method, extra) in [
        ("unstar", format!("&albumId={gone}")),
        ("star", format!("&id={gone}")),
        ("setRating", format!("&id={gone}&rating=1")),
    ] {
        let response = call(method, extra).await;
        assert_eq!(response["error"]["code"], 70, "{method}: {response}");
        assert_eq!(held().await, before, "{method} wrote something");
    }

    // The native API answers the same.
    let listed = send_json(&router, "GET", "/api/v2/albums", &token, None).await;
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(album_ids(&json_body(listed).await), vec![kept.clone()]);
    let detail = send_json(
        &router,
        "GET",
        &format!("/api/v2/albums/{gone}"),
        &token,
        None,
    )
    .await;
    assert_eq!(detail.status(), StatusCode::NOT_FOUND);
    let unstar = send_json(
        &router,
        "DELETE",
        &format!("/api/v2/favorites/album/{gone}"),
        &token,
        None,
    )
    .await;
    assert_eq!(unstar.status(), StatusCode::NOT_FOUND);
    assert_eq!(held().await, before, "the native unstar wrote something");

    // The files come back: the album with them, under its own id, starred
    // and rated as it was.
    std::fs::rename(set_aside.join("Gone One.flac"), music.join("Gone One.flac")).unwrap();
    std::fs::rename(set_aside.join("Gone Two.flac"), music.join("Gone Two.flac")).unwrap();
    run_scan(&state, owner, record).await;
    let starred = call("getStarred2", String::new()).await;
    assert_eq!(
        album_ids(&starred["starred2"]["album"]),
        vec![gone.clone()],
        "{starred}"
    );
    let read = call("getAlbum", format!("&id={gone}")).await;
    assert_eq!(read["album"]["songCount"], 2, "{read}");
    assert_eq!(read["album"]["userRating"], 4, "{read}");
}
