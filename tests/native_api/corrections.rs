//! Track corrections and how they reach every reader: through scans, artists
//! and a clear.

use super::*;

/// A correction outlives the scan that would have erased it.
///
/// Writing tags had two obvious routes and both were wrong: rewriting the file
/// breaks the invariant that makes a scan safe, and writing the track row is
/// undone by the next scan's `title=excluded.title`. The correction therefore
/// lives beside the row, and this asserts the three things that buys — it
/// shows, it survives, and the file's hash never moves.
#[tokio::test]
async fn a_track_correction_survives_the_scan_that_would_have_erased_it() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let listener = state
        .db
        .create_account("tag-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("tag-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Mispelled Titel.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Tag library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Tag library".into(),
        root_path: root,
    };
    run_scan(&state, owner, library.clone()).await;

    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    let track_id = tracks[0].id;
    let scanned_hash = tracks[0].full_hash.clone();
    assert_eq!(tracks[0].title, "Mispelled Titel");

    let token = login_token(&router, "tagger", password).await;
    let patch = |token: String, body: serde_json::Value| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    Request::patch(format!("/api/v2/tracks/{track_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            (response.status(), json_body(response).await)
        }
    };

    let (status, corrected) = patch(
        token.clone(),
        serde_json::json!({ "title": "Misspelled Title", "year": 1998 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(corrected["title"], "Misspelled Title");
    assert_eq!(corrected["year"], 1998);
    // The file was not touched, which is what keeps a content-based link valid
    // across an edit.
    assert_eq!(corrected["full_hash"], scanned_hash);

    // The track answers the effective value and says nothing about where it
    // came from, so an editor cannot tell a corrected field from a scanned one
    // — nor offer to restore what it cannot read. That is what the overrides
    // route is for, and it is the read half of correcting a tag.
    let overrides = {
        let response = router
            .clone()
            .oneshot(
                Request::get(format!("/api/v2/tracks/{track_id}/overrides"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        json_body(response).await
    };
    // The scanned column is untouched by the correction: the projection merges
    // the two with COALESCE rather than writing over the file's value.
    assert_eq!(overrides["source"]["title"], "Mispelled Titel");
    assert_eq!(overrides["overrides"]["title"], "Misspelled Title");
    assert_eq!(overrides["overrides"]["year"], 1998);
    // A field nobody corrected is null, which is what its column holds.
    assert!(overrides["overrides"]["comment"].is_null());
    assert!(overrides["overrides"]["artists"].is_null());
    // Artists and genres are deliberately absent from `source`: correcting
    // either rewrites `track_participant` and the display string, so once a
    // correction exists the file's own credits are no longer in the database.
    assert!(overrides["source"].get("artists").is_none());

    // The headline. A scan applies `title=excluded.title` over the track row,
    // so a correction stored there would be gone by now.
    //
    // Forced full, because nothing about the file changed and an ordinary scan
    // would take the skip path — proving only that the apply never ran.
    state.db.request_full_scan_everywhere().await.unwrap();
    run_scan(&state, owner, library).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks[0].id, track_id);
    assert_eq!(tracks[0].full_hash, scanned_hash, "the file never moved");
    let after_scan = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(after_scan[0].title, "Misspelled Title");
    assert_eq!(after_scan[0].year, Some(1998));

    // The index holds a copy of the title, and a corrected track that still
    // answered to the old spelling would be corrected only to the eye.
    let found = state
        .services
        .catalog_search(
            owner,
            &[],
            "Misspelled",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        found.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
        vec![track_id],
        "the search index follows the correction"
    );

    // It is a library fact, so it lands on the library feed rather than in one
    // account's journal.
    let feed = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v2/libraries/{library_id}/events?after=0&limit=500"
            ))
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let feed = json_body(feed).await;
    assert!(
        feed["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|event| event["entity_id"] == track_id.to_string() && event["action"] == "upsert"),
        "the correction is announced on the feed"
    );

    // A listener may read the library and may not rewrite what everyone else
    // reads. Blurred onto 404, like every refusal that would otherwise confirm
    // what a caller cannot reach.
    sqlx::query(
        "INSERT INTO library_member (library_id, user_id, role, created_at) \
         VALUES (?, ?, 'listener', ?)",
    )
    .bind(library_id.to_string())
    .bind(listener.to_string())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let listener_token = login_token(&router, "tag-listener", password).await;
    let (status, _) = patch(
        listener_token.clone(),
        serde_json::json!({ "title": "Nope" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // An empty patch writes nothing, and it is still a request to correct this
    // track: a listener is refused it the same way, or `{}` would answer
    // differently from every other patch they send.
    let (status, _) = patch(listener_token.clone(), serde_json::json!({})).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // And a manager may, which is the half of the rule the refusal above cannot
    // show: were it `Owner` alone, everything asserted so far would still pass.
    let manager = state
        .db
        .create_account("tag-manager", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO library_member (library_id, user_id, role, created_at) \
         VALUES (?, ?, 'manager', ?)",
    )
    .bind(library_id.to_string())
    .bind(manager.to_string())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let manager_token = login_token(&router, "tag-manager", password).await;
    let (status, by_manager) = patch(
        manager_token,
        serde_json::json!({ "title": "Corrected By Manager" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(by_manager["title"], "Corrected By Manager");
    // #177. The manager mentioned the title and nothing else, so the year the
    // owner corrected is still there. Wholesale, it would have been dropped.
    assert_eq!(by_manager["year"], 1998);

    // An empty body mentions nothing, so it changes nothing.
    let (status, untouched) = patch(token.clone(), serde_json::json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(untouched["title"], "Corrected By Manager");
    assert_eq!(untouched["year"], 1998);

    // `null` is what removes a correction, and the file's own spelling comes
    // back.
    let (status, cleared) = patch(token, serde_json::json!({ "title": null, "year": null })).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cleared["title"], "Mispelled Titel");
    assert!(cleared["year"].is_null());
}

/// A corrected artist agrees with itself everywhere it is written down.
///
/// This is what makes artists and genres different from a title: they are also
/// rows in `track_participant` and `track_genre`, rebuilt by every scan from
/// the file. Correcting the display string alone would have a track answer
/// `artist` and `artists[]` differently, and stay findable under the name it
/// was corrected away from.
#[tokio::test]
async fn a_corrected_artist_agrees_with_itself_everywhere() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("credits-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("credit-fix-music");
    std::fs::create_dir_all(&music).unwrap();
    // Tagged by FFmpeg, so the track starts with real credits and real genres
    // to be corrected away from.
    generate_audio_fixture(&music.join("Mistagged.flac"), "flac", "flac");
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Credit fix library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Credit fix library".into(),
        root_path: root,
    };
    run_scan(&state, owner, library.clone()).await;
    state
        .db
        .consolidate_catalog_derivations(library_id)
        .await
        .unwrap();

    let track_id = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;
    let before = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(
        before[0]
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha", "Beta"],
        "the file credits two artists to begin with"
    );

    let token = login_token(&router, "credits-owner", password).await;
    let response = router
        .clone()
        .oneshot(
            Request::patch(format!("/api/v2/tracks/{track_id}"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "artists": ["Corrected Performer"],
                        "genres": ["Corrected Genre"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Everything the track says about itself, checked together — the display
    // string, the structured list, and the index.
    let agrees = |songs: Vec<waveflow_server::services::SongItem>, when: &str| {
        let song = &songs[0];
        assert_eq!(
            song.artist.as_deref(),
            Some("Corrected Performer"),
            "{when}"
        );
        assert_eq!(
            song.artists
                .iter()
                .map(|artist| artist.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Corrected Performer"],
            "{when}: the structured list has to agree with the display string"
        );
        assert_eq!(song.genres, vec!["Corrected Genre".to_owned()], "{when}");
    };
    agrees(
        state
            .services
            .songs_by_ids(owner, &[track_id])
            .await
            .unwrap(),
        "right after the correction",
    );

    let found = state
        .services
        .catalog_search(
            owner,
            &[],
            "Corrected",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        found.songs.iter().map(|song| song.id).collect::<Vec<_>>(),
        vec![track_id],
        "a corrected credit has to be findable under its new name"
    );

    // And the scan that rebuilds both relations from the file has to leave them
    // corrected, which is the whole reason it consults the override.
    //
    // A full scan, and not an ordinary one: nothing about the file changed, so
    // an ordinary scan takes the skip path and never re-applies the track at
    // all. Asserting against that would prove only that the apply did not run.
    state.db.request_full_scan_everywhere().await.unwrap();
    run_scan(&state, owner, library).await;
    agrees(
        state
            .services
            .songs_by_ids(owner, &[track_id])
            .await
            .unwrap(),
        "after a rescan rebuilt the relations from the file",
    );

    // A search for the name it was corrected away from must come back empty.
    // Asserting only that the new name is findable would pass while the index
    // still carried the old one alongside it.
    let stale = state
        .services
        .catalog_search(
            owner,
            &[],
            "Alpha",
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert!(
        stale.songs.is_empty(),
        "the corrected track must not answer to its old credit: {:?}",
        stale.songs.iter().map(|song| song.id).collect::<Vec<_>>()
    );

    // Dropping one list with `null` while keeping the other. Treating the pair
    // together would leave the dropped one corrected — the hole a single check
    // over both lists had, seen one field at a time.
    let patch_body = |body: serde_json::Value| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let response = router
                .oneshot(
                    Request::patch(format!("/api/v2/tracks/{track_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
        }
    };
    let names = |song: &waveflow_server::services::SongItem| {
        song.artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<_>>()
    };

    patch_body(serde_json::json!({ "artists": ["Solo"], "genres": ["Ambient"] })).await;
    // `null` removes a list correction. Leaving the field out would keep it.
    patch_body(serde_json::json!({ "artists": ["Solo"], "genres": null })).await;
    let one_dropped = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(names(&one_dropped[0]), vec!["Solo".to_owned()], "kept");
    assert_eq!(
        one_dropped[0].genres,
        vec!["Electronic".to_owned(), "Test".to_owned()],
        "the dropped list goes back to the file while the kept one stays"
    );

    patch_body(serde_json::json!({ "genres": ["Ambient"], "artists": null })).await;
    let other_dropped = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(
        names(&other_dropped[0]),
        vec!["Alpha".to_owned(), "Beta".to_owned()],
        "and the same the other way round"
    );
    assert_eq!(other_dropped[0].genres, vec!["Ambient".to_owned()]);

    // A genre named twice, in any case, is one genre. `track_genre` keys on the
    // pair, and the correction used to insert the second spelling and fail on
    // that key, refusing the whole edit.
    patch_body(serde_json::json!({ "genres": ["Jazz", "Jazz", "jazz"] })).await;
    let repeated = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    // One genre; which of its spellings it is shown under is the genre's own
    // business, not this correction's.
    assert_eq!(repeated[0].genres.len(), 1);
    assert!(repeated[0].genres[0].eq_ignore_ascii_case("jazz"));

    // Removing the correction hands the track back to its file. The rows the
    // correction wrote replaced what the tags said, so this can only be right
    // by re-reading them — and it must not wait for a scan. Removed by an
    // explicit `null` on each list: `{}` leaves every correction where it is.
    let cleared = router
        .oneshot(
            Request::patch(format!("/api/v2/tracks/{track_id}"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"artists":null,"genres":null}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cleared.status(), StatusCode::OK);
    let restored = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(
        restored[0]
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alpha", "Beta"],
        "clearing a correction restores the file's own credits"
    );
    assert_eq!(restored[0].artist.as_deref(), Some("Alpha; Beta"));
    assert_eq!(
        restored[0].genres,
        vec!["Electronic".to_owned(), "Test".to_owned()],
        "and its own genres"
    );
}

/// A correction can be removed from a track that never came from a scan.
///
/// Dropping a list correction hands the track back to its file, which the apply
/// re-reads — and the apply used to demand the scan that had last seen the
/// track. A received file has no such scan until one happens to walk past it,
/// so that demand would have answered 503 to every correction made on a file
/// before its first scan.
#[tokio::test]
async fn a_correction_clears_on_a_track_that_never_came_from_a_scan() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("unscanned-tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("unscanned-tag-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Unscanned.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Unscanned library",
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
            name: "Unscanned library".into(),
            root_path: root,
        },
    )
    .await;
    let track_id = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()[0]
        .id;

    // The state a received file is written in: on disk, in the catalogue, and
    // never walked over by a scan.
    sqlx::query("UPDATE track SET last_seen_scan_id=NULL WHERE id=?")
        .bind(track_id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();

    let token = login_token(&router, "unscanned-tagger", password).await;
    let patch = |body: serde_json::Value| {
        let router = router.clone();
        let token = token.clone();
        async move {
            router
                .oneshot(
                    Request::patch(format!("/api/v2/tracks/{track_id}"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    assert_eq!(
        patch(serde_json::json!({ "artists": ["Corrected Name"] }))
            .await
            .status(),
        StatusCode::OK
    );
    // The drop: this is the path that re-reads the file, and the one that used
    // to need a scan id.
    let dropped = patch(serde_json::json!({ "artists": null })).await;
    assert_eq!(
        dropped.status(),
        StatusCode::OK,
        "removing a correction must not require a scan the track never had"
    );

    let restored = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert!(
        restored[0]
            .artists
            .iter()
            .all(|artist| artist.name != "Corrected Name"),
        "the track went back to what its file says"
    );
    let stamp: Option<String> =
        sqlx::query_scalar("SELECT last_seen_scan_id FROM track WHERE id=?")
            .bind(track_id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(
        stamp.is_none(),
        "and re-deriving its tags did not invent a scan for it"
    );
}
