//! The library event stream: what a scan changed.

use super::*;

/// The library half of the server can finally say what changed.
///
/// Until this feed existed a client's only way to learn a catalogue had moved
/// was to poll it, and a poll that compares counts catches an added track and
/// misses every retag. The property that matters most is the last one asserted
/// here: a file retagged outside the API keeps its track id while its
/// `full_hash` moves, and nothing else on the wire carries that.
#[tokio::test]
async fn the_library_feed_reports_what_a_scan_changed() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("feed-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let stranger = state
        .db
        .create_account("feed-stranger", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("feed-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Kept.wav"));
    write_test_wav_of_len(&music.join("Doomed.wav"), 1_600);
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Feed library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Feed library".into(),
        root_path: root,
    };
    run_scan(&state, owner, library.clone()).await;

    let owner_token = login_token(&router, "feed-owner", password).await;
    let feed = |token: String, after: i64, limit: i64| {
        let router = router.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(format!(
                        "/api/v2/libraries/{library_id}/events?after={after}&limit={limit}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            (response.status(), json_body(response).await)
        }
    };

    let (status, page) = feed(owner_token.clone(), 0, 500).await;
    assert_eq!(status, StatusCode::OK);
    // Nothing has been cut, so the client's margin is the whole feed. Reported
    // rather than left to be discovered: without it the first sign of trouble
    // is the refusal itself, and by then the resync has to happen right now
    // instead of at a moment the client would have chosen.
    assert_eq!(page["purged_through"], 0);
    let events = page["events"].as_array().unwrap();
    assert_eq!(events.len(), 2, "one upsert per track the scan applied");
    for event in events {
        assert_eq!(event["entity_type"], "track");
        assert_eq!(event["action"], "upsert");
        // Nothing else on the wire carries this, which is the whole point.
        assert_eq!(
            event["payload"]["full_hash"].as_str().unwrap().len(),
            64,
            "a track upsert carries the file's hash"
        );
    }
    assert_eq!(page["has_more"], false);
    let after_first_scan = page["next_cursor"].as_i64().unwrap();
    assert!(after_first_scan > 0);

    // Paging is by cursor, not by offset: one at a time reaches the same two.
    let (_, first_page) = feed(owner_token.clone(), 0, 1).await;
    assert_eq!(first_page["events"].as_array().unwrap().len(), 1);
    assert_eq!(first_page["has_more"], true);
    let (_, second_page) = feed(
        owner_token.clone(),
        first_page["next_cursor"].as_i64().unwrap(),
        1,
    )
    .await;
    assert_eq!(second_page["events"].as_array().unwrap().len(), 1);
    assert_eq!(second_page["has_more"], false);

    // A file that disappears is a fact the feed names, not a count it reports.
    std::fs::remove_file(music.join("Doomed.wav")).unwrap();
    run_scan(&state, owner, library.clone()).await;
    let (_, page) = feed(owner_token.clone(), after_first_scan, 500).await;
    let deletes: Vec<&serde_json::Value> = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["action"] == "delete")
        .collect();
    assert_eq!(deletes.len(), 1, "exactly the track whose file is gone");
    assert_eq!(deletes[0]["entity_type"], "track");

    // The headline: a file retagged outside the API keeps its track id while
    // its bytes move. The scan is the only witness, and this is where it says
    // so — a client comparing counts would see nothing at all.
    let kept_before = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .chain(events.iter())
        .find(|event| event["action"] == "upsert")
        .map(|event| event["payload"]["full_hash"].as_str().unwrap().to_owned());
    let after_delete = page["next_cursor"].as_i64().unwrap();
    write_test_wav_of_len(&music.join("Kept.wav"), 2_400);
    run_scan(&state, owner, library).await;
    let (_, page) = feed(owner_token, after_delete, 500).await;
    let retagged = page["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|event| event["action"] == "upsert")
        .expect("rewriting the file makes the scan apply it again");
    assert_ne!(
        Some(
            retagged["payload"]["full_hash"]
                .as_str()
                .unwrap()
                .to_owned()
        ),
        kept_before,
        "the hash in the feed has to be the file's new one"
    );

    // A caller who is not a member is told the library is not there, not that
    // it is forbidden: answering differently would confirm it exists.
    let stranger_token = login_token(&router, "feed-stranger", password).await;
    let (status, _) = feed(stranger_token.clone(), 0, 500).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Admitted, then revoked. Membership is read on every request and cached
    // nowhere, so losing it stops delivery at once — there is no subscriber
    // list to fall out of step with.
    sqlx::query(
        "INSERT INTO library_member (library_id, user_id, role, created_at) \
         VALUES (?, ?, 'listener', ?)",
    )
    .bind(library_id.to_string())
    .bind(stranger.to_string())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let (status, admitted) = feed(stranger_token.clone(), 0, 500).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!admitted["events"].as_array().unwrap().is_empty());
    sqlx::query("DELETE FROM library_member WHERE library_id = ? AND user_id = ?")
        .bind(library_id.to_string())
        .bind(stranger.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let (status, _) = feed(stranger_token, 0, 500).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "revocation is not deferred");

    // A cursor below what has been cut away has missed events, and the feed
    // says so rather than handing back a tail that would pass for a catch-up.
    // The watermark records what was purged; nothing purges yet, so the test
    // moves it by hand — which is exactly what a retention pass will do.
    let owner_token = login_token(&router, "feed-owner", password).await;
    sqlx::query("UPDATE library SET events_purged_through = 3 WHERE id = ?")
        .bind(library_id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let (status, _) = feed(owner_token.clone(), 2, 500).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // Standing exactly at the cut is not standing before it.
    let (status, page) = feed(owner_token, 3, 500).await;
    assert_eq!(status, StatusCode::OK);
    // And the page says where the cut is, while it is still being served
    // rather than after it is refused. This client asked from 3 and the cut is
    // at 3, so it holds no slack at all — one more purge and it is sent to the
    // snapshot — and it can read that off the page it just received.
    assert_eq!(page["purged_through"], 3);
    // The margin is measured against the cursor the client held, not against
    // `next_cursor`: that one is the end of the page just served, so it says
    // what the margin becomes once this page is processed.
    assert_eq!(3 - page["purged_through"].as_i64().unwrap(), 0);
    assert!(
        page["next_cursor"].as_i64().unwrap() > 3,
        "the page advanced it"
    );
}
