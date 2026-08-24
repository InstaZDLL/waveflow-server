//! The facade methods that are neither browsing nor a wire shape.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::authentication::now_ms;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::services::ServiceError;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

/// The facade could not trigger a rescan the native API has always been able to
/// trigger, and answered a not-implemented error for surfaces clients open by
/// default. Both gaps are closed without inventing data.
#[tokio::test]
async fn facade_controls_scans_and_answers_its_remaining_methods() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_asymmetry-key";
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("asym-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"asym-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("asym-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();

    let seed = |account: Uuid, name: &'static str, titles: Vec<&'static str>| {
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
            state
                .db
                .start_scan_job(scan, titles.len() as i64, false)
                .await
                .unwrap();
            for (index, title) in titles.into_iter().enumerate() {
                let mut input = browse_input(
                    index + 120,
                    title,
                    "Asym Album",
                    "Asym Artist",
                    Some(1),
                    Some(1),
                );
                input.relative_path = format!("{name}-{index}.flac");
                input.quick_hash = format!("{:064x}", index + 7_000 + name.len() * 100);
                input.full_hash = format!("{:064x}", index + 8_000 + name.len() * 100);
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
    let library = seed(owner, "asym-own", vec!["One", "Two", "Three"]).await;
    seed(outsider, "asym-foreign", vec!["Hidden"]).await;

    let router = waveflow_server::app(&config, state.clone());

    // Idle, and counting only what this account can reach: the outsider's
    // fourth track must not appear in the owner's total.
    let status = subsonic_json(&router, "getScanStatus", api_key, "").await;
    let status = &status["subsonic-response"]["scanStatus"];
    assert_eq!(status["scanning"], false);
    assert_eq!(status["count"], 3);

    // A membership revoked between the lookup and the queuing must not leave
    // a job behind. The window cannot be interleaved deterministically, so
    // what is asserted is the property that makes it safe: the insert itself
    // requires a library_member row, and a non-member is exactly the state a
    // revocation leaves behind.
    assert!(state
        .db
        .create_scan_job_for_user(outsider, library, "manual")
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        state.services.start_library_scan(outsider, library).await,
        Err(ServiceError::NotFound)
    ));
    // Scoped to the owner's library: the outsider legitimately has jobs against
    // their own.
    let intruder_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scan_job WHERE requested_by = ? AND library_id = ?",
    )
    .bind(outsider.to_string())
    .bind(library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(intruder_jobs, 0, "a non-member queued a scan");

    // Membership is not authority. A listener reads the catalogue; a scan
    // walks the owner's files and takes the writer gate, so it is refused --
    // and refused the way everything unentitled is, indistinguishably from a
    // library that does not exist.
    let listener = state
        .db
        .create_account("asym-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(owner, library, listener, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    // The membership is real: the listener sees the library and its tracks.
    assert_eq!(
        state.db.libraries_for_user(listener).await.unwrap().len(),
        1
    );
    assert!(state
        .db
        .create_scan_job_for_user(listener, library, "manual")
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        state.services.start_library_scan(listener, library).await,
        Err(ServiceError::NotFound)
    ));
    // startScan names no library, so it skips the read-only ones instead of
    // failing: an account whose every library is read-only queues nothing and
    // succeeds, exactly like one that reaches no library at all.
    assert!(state
        .services
        .start_visible_scans(listener)
        .await
        .unwrap()
        .is_empty());
    let listener_jobs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM scan_job WHERE requested_by = ?")
            .bind(listener.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(listener_jobs, 0, "a listener queued a scan");
    // Promotion is all it takes, and it is read from the row rather than
    // cached anywhere.
    state
        .db
        .add_library_member(owner, library, listener, LibraryRole::Manager, now_ms())
        .await
        .unwrap();
    assert!(state
        .db
        .create_scan_job_for_user(listener, library, "manual")
        .await
        .unwrap()
        .is_some());
    state
        .db
        .remove_library_member(owner, library, listener, now_ms())
        .await
        .unwrap();

    // An account that can reach no library has nothing to scan. That is an
    // empty result, not an error: there is no missing resource to report, and
    // every other catalogue-wide method answers such an account the same way.
    let stranger = state
        .db
        .create_account("asym-stranger", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    assert!(state
        .services
        .start_visible_scans(stranger)
        .await
        .unwrap()
        .is_empty());

    // Container-only aliases for browse-by-folder clients. The payload is the
    // ID3 one; only the wrapper name differs, as for getAlbumList.
    let search2 = subsonic_json(&router, "search2", api_key, "&query=One&songCount=10").await;
    assert!(search2["subsonic-response"]["searchResult2"]["song"].is_array());
    assert!(search2["subsonic-response"].get("searchResult3").is_none());
    // Nothing is starred yet, so the container is present but carries no list.
    let starred = subsonic_json(&router, "getStarred", api_key, "").await;
    assert!(starred["subsonic-response"]["starred"].is_object());
    assert!(starred["subsonic-response"]["starred"]
        .get("song")
        .is_none());
    assert!(starred["subsonic-response"].get("starred2").is_none());

    // One favorite of each kind, so the renamed container is exercised with
    // content: an alias that only ever answers empty proves nothing about the
    // JSON array rules its new name needs.
    let snapshot = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    for (entity, id) in [
        ("artist", snapshot.artists[0].artist.id),
        ("album", snapshot.albums[0].id),
        ("track", snapshot.songs[0].id),
    ] {
        state
            .services
            .set_star(owner, entity, id, true)
            .await
            .unwrap();
    }
    let starred = subsonic_json(&router, "getStarred", api_key, "").await;
    let starred = &starred["subsonic-response"]["starred"];
    for field in ["artist", "album", "song"] {
        let entries = starred[field]
            .as_array()
            .unwrap_or_else(|| panic!("starred.{field} is not an array: {starred}"));
        assert_eq!(entries.len(), 1, "{field}");
    }
    // The ID3 method answers the same payload under its own container.
    let starred2 = subsonic_json(&router, "getStarred2", api_key, "").await;
    let starred2 = &starred2["subsonic-response"]["starred2"];
    for field in ["artist", "album", "song"] {
        assert_eq!(starred2[field], starred[field], "{field}");
    }

    // Surfaces WaveFlow does not compute answer the standard empty container
    // rather than a not-implemented error.
    for (method, container) in [
        ("getTopSongs", "topSongs"),
        ("getSimilarSongs", "similarSongs"),
        ("getSimilarSongs2", "similarSongs2"),
        ("getInternetRadioStations", "internetRadioStations"),
    ] {
        let response = subsonic_json(&router, method, api_key, "&id=whatever&count=5").await;
        assert_eq!(response["subsonic-response"]["status"], "ok", "{method}");
        assert_eq!(
            response["subsonic-response"][container],
            serde_json::json!({}),
            "{method}"
        );
    }

    // No avatars are stored, so the data is missing rather than the method.
    let avatar = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAvatar.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&username=asym-owner"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(avatar.status(), StatusCode::OK);
    assert_eq!(
        json_body(avatar).await["subsonic-response"]["error"]["code"],
        70
    );

    // A method that really is unimplemented still says so.
    let unknown = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getPodcasts.view?apiKey={api_key}&v=1.16.1&c=golden&f=json"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(unknown).await["subsonic-response"]["error"]["code"],
        0
    );

    // Starting a scan comes last: it runs a real scan over a library root that
    // holds no files, which marks the fabricated tracks unavailable. Every
    // assertion above reads the catalogue and would race it.
    //
    // The response carries the same shape as getScanStatus, so a client that
    // only calls startScan still learns the state.
    let started = subsonic_json(&router, "startScan", api_key, "").await;
    let started = &started["subsonic-response"]["scanStatus"];
    assert!(started["scanning"].is_boolean());
    assert!(started["count"].is_number());
    // The work is real: a job now exists for the owner's library beyond the one
    // the fixture created.
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scan_job WHERE library_id = ?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(queued >= 2, "startScan queued nothing: {queued} jobs");
    let foreign_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scan_job sj JOIN library l ON l.id = sj.library_id \
         WHERE l.name = 'asym-foreign'",
    )
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        foreign_jobs, 1,
        "startScan reached a library the account cannot see"
    );
}

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
