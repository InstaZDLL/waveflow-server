//! Bookmarks and the play queue, the methods the Subsonic API files under
//! bookmarks.

use super::*;

/// Bookmarks are the last Subsonic mutation that had nowhere to write. They go
/// through the domain services like every other piece of user data, which is
/// what puts them in the sync journal and the bootstrap snapshot rather than
/// only in one client's view.
#[tokio::test]
async fn bookmarks_round_trip_sync_and_isolate_tenants() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_bookmark-key";
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("bookmark-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"bookmark-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("bookmark-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();

    let seed = |account: Uuid, name: &'static str, offset: usize| {
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
            state.db.start_scan_job(scan, 1, false).await.unwrap();
            let mut input = browse_input(
                offset,
                "Long Form",
                "Bookmark Album",
                "Bookmark Artist",
                Some(1),
                Some(1),
            );
            input.relative_path = format!("{name}.flac");
            input.quick_hash = format!("{:064x}", offset + 20_000);
            input.full_hash = format!("{:064x}", offset + 21_000);
            state
                .db
                .apply_catalog_track(library, scan, &input, None, false)
                .await
                .unwrap();
            state.db.finish_scan_job(scan, 0).await.unwrap();
            state
                .services
                .catalog_snapshot(account, &[])
                .await
                .unwrap()
                .songs[0]
                .id
        }
    };
    let track = seed(owner, "bookmark-own", 200).await;
    let foreign_track = seed(outsider, "bookmark-foreign", 201).await;

    let router = waveflow_server::app(&config, state.clone());

    // Nothing set yet: the container is present and empty, as it always was.
    let empty = subsonic_json(&router, "getBookmarks", api_key, "").await;
    assert!(empty["subsonic-response"]["bookmarks"].is_object());
    assert!(empty["subsonic-response"]["bookmarks"]
        .get("bookmark")
        .is_none());

    let created = subsonic_json(
        &router,
        "createBookmark",
        api_key,
        &format!("&id={track}&position=125000&comment=where%20I%20stopped"),
    )
    .await;
    assert_eq!(created["subsonic-response"]["status"], "ok");
    // A mutation with no result answers the bare envelope.
    assert!(created["subsonic-response"].get("createBookmark").is_none());

    let listed = subsonic_json(&router, "getBookmarks", api_key, "").await;
    let bookmark = &listed["subsonic-response"]["bookmarks"]["bookmark"][0];
    assert_eq!(bookmark["position"], 125_000);
    assert_eq!(bookmark["username"], "bookmark-owner");
    assert_eq!(bookmark["comment"], "where I stopped");
    assert!(bookmark["created"].is_string());
    assert!(bookmark["changed"].is_string());
    // The entry is a full media item, carrying the position it is bookmarked at.
    assert_eq!(bookmark["entry"]["id"], track.to_string());
    assert_eq!(bookmark["entry"]["title"], "Long Form");
    assert_eq!(bookmark["entry"]["bookmarkPosition"], 125_000);
    // It goes through the shared projection, so the modern fields are there too.
    assert_eq!(bookmark["entry"]["samplingRate"], 44_100);
    assert!(bookmark["entry"]["artists"].is_array());

    // A bookmark answers "where did I stop in this file", so setting it again
    // moves it rather than adding a second one.
    subsonic_json(
        &router,
        "createBookmark",
        api_key,
        &format!("&id={track}&position=250000"),
    )
    .await;
    let moved = subsonic_json(&router, "getBookmarks", api_key, "").await;
    let moved = &moved["subsonic-response"]["bookmarks"]["bookmark"];
    assert_eq!(moved.as_array().unwrap().len(), 1);
    assert_eq!(moved[0]["position"], 250_000);
    // Omitting the comment clears it rather than keeping the old one.
    assert!(moved[0].get("comment").is_none());

    // It reaches the sync surfaces because it is a domain mutation, not a
    // facade-local one: a desktop client sees it without a second contract.
    let snapshot = state.services.sync_snapshot(owner, 50).await.unwrap();
    assert_eq!(snapshot.bookmarks.len(), 1);
    assert_eq!(snapshot.bookmarks[0].position_ms, 250_000);
    assert_eq!(snapshot.bookmarks[0].song.id, track);
    let changes = state.sync.changes(owner, 0, 100).await.unwrap();
    assert!(
        changes
            .changes
            .iter()
            .any(|change| change.entity_type == "bookmark" && change.entity_id == track),
        "no bookmark event in the journal: {:?}",
        changes
            .changes
            .iter()
            .map(|change| change.entity_type.clone())
            .collect::<Vec<_>>()
    );

    // And the native bootstrap carries them, so a desktop client that only
    // ever calls /sync/snapshot receives bookmarks without a second contract.
    let token = login_token(&router, "bookmark-owner", "correct horse battery staple").await;
    let native = router
        .clone()
        .oneshot(
            Request::get("/api/v2/sync/snapshot")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    let native = json_body(native).await;
    assert_eq!(native["bookmarks"].as_array().unwrap().len(), 1);
    assert_eq!(native["bookmarks"][0]["position_ms"], 250_000);
    assert_eq!(native["bookmarks"][0]["song"]["id"], track.to_string());

    // A position before the start of the file is not a position. The service
    // refuses it, and the facade reports the parameter error rather than an
    // internal one.
    assert!(matches!(
        state.services.set_bookmark(owner, track, -1, None).await,
        Err(ServiceError::Invalid)
    ));
    let negative = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={track}&position=-1"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(negative).await["subsonic-response"]["error"]["code"],
        10
    );

    // XML is the default encoding and nests the entry as an element rather
    // than as a JSON object, so it gets its own assertion.
    let xml = router
        .clone()
        .oneshot(
            Request::get(
                "/rest/getBookmarks.view?u=bookmark-owner&p=bookmark-secret&v=1.16.1&c=golden",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(xml.status(), StatusCode::OK);
    let xml = body_text(xml).await;
    assert!(xml.contains("<bookmark "));
    assert!(xml.contains("username=\"bookmark-owner\""));
    assert!(xml.contains("position=\"250000\""));
    assert!(xml.contains("<entry "));
    assert!(xml.contains("bookmarkPosition=\"250000\""));

    // Another account's track is not bookmarkable, and the refusal does not
    // confirm that the track exists.
    let foreign = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={foreign_track}&position=1000"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::OK);
    assert_eq!(
        json_body(foreign).await["subsonic-response"]["error"]["code"],
        70
    );
    let unknown = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={}&position=1000",
                Uuid::nil()
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(unknown).await["subsonic-response"]["error"]["code"],
        70
    );
    assert!(state.services.bookmarks(outsider).await.unwrap().is_empty());

    subsonic_json(&router, "deleteBookmark", api_key, &format!("&id={track}")).await;
    assert!(state.services.bookmarks(owner).await.unwrap().is_empty());
    // Deleting one that is not there is not an error: the caller asked for the
    // track to carry no bookmark, and it does not.
    subsonic_json(&router, "deleteBookmark", api_key, &format!("&id={track}")).await;
}

/// A play queue holds the largest playlist (RFC-022, what was still open), on
/// both surfaces: a `savePlayQueue` of that many identifiers fits one form
/// POST, the native queue route takes the same list in one JSON body beyond
/// the 16 KiB every other body is held to, and one track more is refused.
#[tokio::test]
async fn a_play_queue_holds_the_largest_playlist_on_both_surfaces() {
    let (_temp, config, state) = test_app().await;
    let password = Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("long-queue", &hash, AccountRole::User, now_ms())
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
    let (_library, ids) = seed_titles(
        &state,
        &config,
        owner,
        "long-queue",
        4_600,
        &["Again", "Last"],
    )
    .await;
    let (track, last) = (ids[0], ids[1]);
    let ceiling = waveflow_server::services::MAX_QUEUE_TRACKS;
    assert!(
        ceiling >= waveflow_server::services::MAX_PLAYLIST_TRACKS,
        "a playlist played whole must fit the queue"
    );
    let router = waveflow_server::app(&config, state.clone());

    // Subsonic: one form POST naming the track as many times as the queue
    // holds, the last entry a different track so the order is checked.
    let mut body = format!("apiKey={api_key}&v=1.16.1&c=golden&f=json&current={track}");
    for _ in 1..ceiling {
        body.push_str(&format!("&id={track}"));
    }
    body.push_str(&format!("&id={last}"));
    assert!(body.len() <= waveflow_server::subsonic::MAX_FORM_BYTES);
    let saved = router
        .clone()
        .oneshot(
            Request::post("/rest/savePlayQueue.view?f=json")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let saved = json_body(saved).await;
    assert_eq!(saved["subsonic-response"]["status"], "ok", "{saved}");
    let queue = state.services.queue(owner).await.unwrap().unwrap();
    assert_eq!(queue.songs.len(), ceiling);
    assert_eq!(queue.songs.last().unwrap().id, last);

    // Native: the same list in one JSON body, then one track more.
    let token = login_token(&router, "long-queue", &password).await;
    let mut track_ids = vec![track; ceiling - 1];
    track_ids.push(last);
    let body = serde_json::json!({ "track_ids": track_ids, "current": track, "position_ms": 0 });
    assert!(body.to_string().len() > 16 * 1024);
    let saved = send_json(&router, "PUT", "/api/v2/queue", &token, Some(body)).await;
    assert_eq!(saved.status(), StatusCode::NO_CONTENT);
    track_ids.push(track);
    let refused = send_json(
        &router,
        "PUT",
        "/api/v2/queue",
        &token,
        Some(serde_json::json!({ "track_ids": track_ids, "position_ms": 0 })),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // A track this account cannot read still refuses the whole queue.
    let foreign = send_json(
        &router,
        "PUT",
        "/api/v2/queue",
        &token,
        Some(serde_json::json!({ "track_ids": [track, Uuid::new_v4()], "position_ms": 0 })),
    )
    .await;
    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        state
            .services
            .queue(owner)
            .await
            .unwrap()
            .unwrap()
            .songs
            .len(),
        ceiling,
        "a refused save changes nothing"
    );
}
