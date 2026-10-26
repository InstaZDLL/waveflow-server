//! What one correction patch changes, keeps, and records about the device that
//! sent it.

use super::*;

/// A correction leaves alone every field it does not mention.
///
/// #177. The patch used to replace the whole set of corrections, so one client
/// correcting a title silently erased a comment another had corrected. Three
/// states are what fix it: a field left out keeps its correction, `null`
/// removes it, a value sets it.
///
/// The desktop's request is replayed as `drain.rs` sends it, because that client
/// is why no version header was needed: it spells its six fields out, `null`
/// included, so those have to keep meaning what they meant — while the three it
/// has no input for now survive it.
#[tokio::test]
async fn a_correction_leaves_alone_what_it_does_not_mention() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("partial-tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("partial-tag-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Partial.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Partial library",
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
            name: "Partial library".into(),
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

    let token = login_token(&router, "partial-tagger", password).await;
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
                .status()
        }
    };
    // The row as it stands, not the merged track: a correction and the file's
    // value look identical in the catalogue, and this test is about which is
    // which.
    let stored = || {
        let state = state.clone();
        async move {
            serde_json::to_value(
                state
                    .services
                    .track_overrides(owner, track_id)
                    .await
                    .unwrap()
                    .overrides,
            )
            .unwrap()
        }
    };
    let identifier = "8f3c7d52-5b0a-4d5e-9c1e-2a6b4f0d1e3a";

    // Another client corrects the three fields the desktop has no input for.
    assert_eq!(
        patch(serde_json::json!({
            "sort_title": "Partial, The",
            "comment": "Remastered",
            "musicbrainz_recording_id": identifier,
        }))
        .await,
        StatusCode::OK
    );

    // Then the desktop corrects the title and the year, as it sends them: every
    // field its editor has, the empty ones as an explicit `null`.
    assert_eq!(
        patch(serde_json::json!({
            "title": "Partial (Live)",
            "artists": null,
            "genres": null,
            "year": 2001,
            "track_number": null,
            "disc_number": null,
        }))
        .await,
        StatusCode::OK
    );
    let after_desktop = stored().await;
    assert_eq!(after_desktop["title"], "Partial (Live)");
    assert_eq!(after_desktop["year"], 2001);
    assert_eq!(
        after_desktop["comment"], "Remastered",
        "another client's correction survives a patch that never mentioned it"
    );
    assert_eq!(after_desktop["sort_title"], "Partial, The");
    assert_eq!(after_desktop["musicbrainz_recording_id"], identifier);

    // `{}` mentions nothing, so it changes nothing: not the correction, not the
    // row, and not the library feed, where an `upsert` for a change that never
    // happened would send every client to refetch the track.
    let events = || {
        let state = state.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM library_event WHERE library_id=?")
                .bind(library_id.to_string())
                .fetch_one(state.db.pool())
                .await
                .unwrap()
        }
    };
    // A sentinel rather than the stamp the last write left: two writes can land
    // in the same millisecond, and a comparison with that stamp would then pass
    // whether or not the row was rewritten.
    sqlx::query("UPDATE track_override SET updated_at=1 WHERE track_id=?")
        .bind(track_id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let events_before = events().await;
    assert_eq!(patch(serde_json::json!({})).await, StatusCode::OK);
    assert_eq!(stored().await, after_desktop, "an empty patch is a no-op");
    assert_eq!(events().await, events_before, "and announces nothing");
    let stamp: i64 = sqlx::query_scalar("SELECT updated_at FROM track_override WHERE track_id=?")
        .bind(track_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(stamp, 1, "and writes nothing");

    // `null` removes one correction, and only that one.
    assert_eq!(
        patch(serde_json::json!({ "comment": null })).await,
        StatusCode::OK
    );
    let without_comment = stored().await;
    assert!(without_comment["comment"].is_null());
    assert_eq!(without_comment["title"], "Partial (Live)");
    assert_eq!(without_comment["sort_title"], "Partial, The");

    // A blank string reads as `null`: a removal, never a stored blank.
    assert_eq!(
        patch(serde_json::json!({ "sort_title": "   " })).await,
        StatusCode::OK
    );
    let without_sort_title = stored().await;
    assert!(without_sort_title["sort_title"].is_null());
    assert_eq!(without_sort_title["title"], "Partial (Live)");

    // A value outside what is allowed refuses the whole patch before it writes
    // anything, the value beside it included.
    assert_eq!(
        patch(serde_json::json!({ "title": "Never Written", "year": 0 })).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(stored().await["title"], "Partial (Live)");

    // The desktop emptying its whole form removes its own corrections, and
    // still leaves the identifier it cannot see.
    assert_eq!(
        patch(serde_json::json!({
            "title": null,
            "artists": null,
            "genres": null,
            "year": null,
            "track_number": null,
            "disc_number": null,
        }))
        .await,
        StatusCode::OK
    );
    let cleared_by_desktop = stored().await;
    assert!(cleared_by_desktop["title"].is_null());
    assert!(cleared_by_desktop["year"].is_null());
    assert_eq!(cleared_by_desktop["musicbrainz_recording_id"], identifier);

    // Removing the last correction removes the row, rather than leaving one that
    // holds nothing but NULLs and still claims the track carries a correction.
    assert_eq!(
        patch(serde_json::json!({ "musicbrainz_recording_id": null })).await,
        StatusCode::OK
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM track_override WHERE track_id=?")
        .bind(track_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// A list correction is kept by a patch that does not mention it.
///
/// The lists are where getting this wrong costs most. A correction to either is
/// materialised into `track_participant` and `track_genre`, so dropping one by
/// accident re-reads the file and rewrites the credits. A patch about the title
/// must touch neither.
#[tokio::test]
async fn a_list_correction_is_kept_by_a_patch_that_does_not_mention_it() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("kept-list-tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("kept-list-music");
    std::fs::create_dir_all(&music).unwrap();
    // A file that credits somebody, or "the correction went back to the file"
    // and "the correction credits nobody" would both read as an empty list, and
    // the second assertion below would pass for the wrong reason.
    generate_audio_fixture(&music.join("Kept.flac"), "flac", "flac");
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Kept list library",
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
            name: "Kept list library".into(),
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

    let token = login_token(&router, "kept-list-tagger", password).await;
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
                .status()
        }
    };
    let names = |song: &waveflow_server::services::SongItem| {
        song.artists
            .iter()
            .map(|artist| artist.name.clone())
            .collect::<Vec<_>>()
    };

    assert_eq!(
        patch(serde_json::json!({ "artists": ["Solo"], "genres": ["Ambient"] })).await,
        StatusCode::OK
    );
    assert_eq!(
        patch(serde_json::json!({ "title": "Retitled" })).await,
        StatusCode::OK
    );
    let retitled = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert_eq!(retitled[0].title, "Retitled");
    assert_eq!(
        names(&retitled[0]),
        vec!["Solo".to_owned()],
        "the artists correction the patch did not mention is still applied"
    );
    assert_eq!(retitled[0].genres, vec!["Ambient".to_owned()]);

    // `[]` is a value, not a removal: the track credits nobody, which is a
    // correction rather than the absence of one — and the genres it did not
    // mention stay.
    assert_eq!(
        patch(serde_json::json!({ "artists": [] })).await,
        StatusCode::OK
    );
    let nobody = state
        .services
        .songs_by_ids(owner, &[track_id])
        .await
        .unwrap();
    assert!(
        names(&nobody[0]).is_empty(),
        "an empty list credits nobody rather than going back to the file"
    );
    assert_eq!(nobody[0].genres, vec!["Ambient".to_owned()]);
    let stored = state
        .services
        .track_overrides(owner, track_id)
        .await
        .unwrap()
        .overrides;
    assert_eq!(stored.artists, Some(Vec::new()));
}

/// A correction names the device that made it, on the library feed.
///
/// The same reason as an upload: without it the client that typed the
/// correction reads it back as a change somebody else made, and has no way to
/// tell one from the other.
#[tokio::test]
async fn a_correction_names_the_device_that_made_it() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("attributing-tagger", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("attributing-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Attributed.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Attributing",
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
            name: "Attributing".into(),
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
    let (token, device) = login_session(&router, "attributing-tagger", password).await;

    let corrected = router
        .clone()
        .oneshot(
            Request::patch(format!("/api/v2/tracks/{track_id}"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-waveflow-device-id", &device)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "title": "Named" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(corrected.status(), StatusCode::OK);

    let feed = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v2/libraries/{library_id}/events?after=0&limit=100"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(feed.status(), StatusCode::OK);
    let body = json_body(feed).await;
    let events = body["events"].as_array().unwrap();
    let last = events.last().expect("the correction is on the feed");
    assert_eq!(last["entity_id"], track_id.to_string());
    assert_eq!(
        last["origin_device_id"], device,
        "the device that typed the correction is named"
    );
    // The scan that created the track came first, and belongs to nobody.
    assert!(
        events[0]["origin_device_id"].is_null(),
        "a scan is nobody's request in particular"
    );
}
