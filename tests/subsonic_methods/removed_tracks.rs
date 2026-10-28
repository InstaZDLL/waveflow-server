//! Writes on a track whose file has gone: refused as reads are, leaving
//! nothing behind, while what was written before survives for the file's
//! return.

use super::*;

/// What an account holds about one track, read straight from the tables the
/// writes touch: its star, its rating, its bookmark, its plays, what it is
/// playing now, and how far its sync journal has gone. A refused write must
/// leave every one of these as it found them.
async fn held_about(
    state: &waveflow_server::AppState,
    account: Uuid,
    track: Uuid,
) -> serde_json::Value {
    let (account, track) = (account.to_string(), track.to_string());
    let star: Option<i64> = sqlx::query_scalar(
        "SELECT starred_at FROM user_star WHERE user_id=? AND entity_type='track' AND entity_id=?",
    )
    .bind(&account)
    .bind(&track)
    .fetch_optional(state.db.pool())
    .await
    .unwrap();
    let rating: Option<i64> = sqlx::query_scalar(
        "SELECT rating FROM user_rating WHERE user_id=? AND entity_type='track' AND entity_id=?",
    )
    .bind(&account)
    .bind(&track)
    .fetch_optional(state.db.pool())
    .await
    .unwrap();
    let bookmark: Option<(i64, i64)> = sqlx::query_as(
        "SELECT position_ms, updated_at FROM bookmark WHERE user_id=? AND track_id=?",
    )
    .bind(&account)
    .bind(&track)
    .fetch_optional(state.db.pool())
    .await
    .unwrap();
    let plays: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM play_event WHERE user_id=? AND track_id=?")
            .bind(&account)
            .bind(&track)
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    let now_playing: Option<String> =
        sqlx::query_scalar("SELECT track_id FROM now_playing WHERE user_id=?")
            .bind(&account)
            .fetch_optional(state.db.pool())
            .await
            .unwrap();
    let journal: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_event WHERE user_id=?")
        .bind(&account)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    serde_json::json!({
        "star": star,
        "rating": rating,
        "bookmark": bookmark,
        "plays": plays,
        "now_playing": now_playing,
        "journal": journal,
    })
}

/// The saved queue, as stored: its tracks in order, the current one, and the
/// position.
async fn stored_queue(
    state: &waveflow_server::AppState,
    account: Uuid,
) -> (Vec<String>, Option<String>, i64) {
    let tracks: Vec<String> = sqlx::query_scalar(
        "SELECT track_id FROM play_queue_track WHERE user_id=? ORDER BY position",
    )
    .bind(account.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    let (current, position): (Option<String>, i64) =
        sqlx::query_as("SELECT current_track_id, position_ms FROM play_queue WHERE user_id=?")
            .bind(account.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    (tracks, current, position)
}

/// A track whose file was removed is unavailable: every read already answered
/// not found, and every write — bookmark, scrobble, star, rating, through the
/// façade and the native API alike — now does too, with nothing written, not
/// even in part. A saved queue is the one write that goes through: a client
/// keeps the gone track in the queue it holds, and refusing the whole queue
/// left it unable to save anything (DSub, 2026-10-28 campaign), so the queue is
/// stored without it. What was written before the file went is kept, and comes
/// back with the file.
#[tokio::test]
async fn writes_on_a_removed_track_are_refused_and_its_data_survives() {
    let (_temp, config, state) = test_app().await;
    // Drawn at run time: a literal reaching the hash is what CodeQL reports.
    let password = Uuid::new_v4().to_string();
    let owner = state
        .db
        .create_account(
            "removed-owner",
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
    let music = config.data_dir.join("removed-music");
    std::fs::create_dir_all(&music).unwrap();
    // Different lengths, so the two files differ in content and a file that
    // comes back is matched to its own track by its hash.
    write_test_wav_of_len(&music.join("Kept.wav"), 8_000 * 2);
    write_test_wav_of_len(&music.join("Gone.wav"), 8_000 * 3);
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Removed",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let record = waveflow_server::storage::catalog::LibraryRecord {
        id: library,
        name: "Removed".into(),
        root_path: root.clone(),
    };
    run_scan(&state, owner, record.clone()).await;
    let track_of = |path: &'static str| {
        let state = state.clone();
        async move {
            let id: String = sqlx::query_scalar("SELECT id FROM track WHERE relative_path=?")
                .bind(path)
                .fetch_one(state.db.pool())
                .await
                .unwrap();
            Uuid::parse_str(&id).unwrap()
        }
    };
    let (kept, gone) = (track_of("Kept.wav").await, track_of("Gone.wav").await);
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "removed-owner", &password).await;
    let call = |method: &'static str, extra: String| {
        let router = router.clone();
        let api_key = api_key.clone();
        async move {
            subsonic_json(&router, method, &api_key, &extra).await["subsonic-response"].clone()
        }
    };

    // Written while the file is there: the data the removal must not cost.
    for (method, extra) in [
        ("star", format!("&id={gone}")),
        ("setRating", format!("&id={gone}&rating=4")),
        ("createBookmark", format!("&id={gone}&position=5000")),
        ("scrobble", format!("&id={gone}&submission=true")),
    ] {
        let response = call(method, extra).await;
        assert_eq!(
            response["status"], "ok",
            "{method} before the removal: {response}"
        );
    }

    std::fs::remove_file(music.join("Gone.wav")).unwrap();
    run_scan(&state, owner, record.clone()).await;
    let available: i64 = sqlx::query_scalar("SELECT is_available FROM track WHERE id=?")
        .bind(gone.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(available, 0, "the precondition: the track is unavailable");
    let read = call("getSong", format!("&id={gone}")).await;
    assert_eq!(read["error"]["code"], 70, "reads already refused: {read}");
    let before = held_about(&state, owner, gone).await;
    assert!(!before["star"].is_null(), "the star is there: {before}");
    assert_eq!(before["rating"], 4, "{before}");

    // Every write through the façade: refused as the read is, nothing written.
    for (method, extra) in [
        ("createBookmark", format!("&id={gone}&position=9000")),
        ("deleteBookmark", format!("&id={gone}")),
        ("scrobble", format!("&id={gone}&submission=true")),
        ("scrobble", format!("&id={gone}&submission=false")),
        ("star", format!("&id={gone}")),
        ("unstar", format!("&id={gone}")),
        ("setRating", format!("&id={gone}&rating=1")),
        ("setRating", format!("&id={gone}&rating=0")),
    ] {
        let response = call(method, extra.clone()).await;
        assert_eq!(response["status"], "failed", "{method}{extra}: {response}");
        assert_eq!(response["error"]["code"], 70, "{method}{extra}: {response}");
        assert_eq!(
            held_about(&state, owner, gone).await,
            before,
            "{method}{extra} wrote something"
        );
    }

    // Several identifiers in one request, the track that is there first: the
    // refusal of the one that went must not leave the first written. Each
    // identifier is its own mutation, so checking them all comes before the
    // first is applied — or the first is recorded, and recorded again when the
    // client retries.
    let kept_before = held_about(&state, owner, kept).await;
    let far_future = now_ms() + 24 * 60 * 60 * 1_000;
    for (method, extra, code) in [
        (
            "scrobble",
            format!("&id={kept}&id={gone}&submission=true"),
            70,
        ),
        ("star", format!("&id={kept}&id={gone}"), 70),
        // And a time that is refused, on the second of two tracks that are there.
        (
            "scrobble",
            format!(
                "&id={kept}&id={kept}&time={}&time={far_future}&submission=true",
                now_ms()
            ),
            10,
        ),
    ] {
        let response = call(method, extra.clone()).await;
        assert_eq!(response["status"], "failed", "{method}{extra}: {response}");
        assert_eq!(
            response["error"]["code"], code,
            "{method}{extra}: {response}"
        );
        assert_eq!(
            held_about(&state, owner, kept).await,
            kept_before,
            "{method}{extra} wrote the track that is there"
        );
    }

    // And through the native API, which reaches the same services.
    for (verb, uri, body) in [
        ("PUT", format!("/api/v2/favorites/track/{gone}"), None),
        ("DELETE", format!("/api/v2/favorites/track/{gone}"), None),
        (
            "PUT",
            format!("/api/v2/ratings/track/{gone}"),
            Some(serde_json::json!({ "rating": 2 })),
        ),
        (
            "PUT",
            format!("/api/v2/bookmarks/{gone}"),
            Some(serde_json::json!({ "position_ms": 7000 })),
        ),
        ("DELETE", format!("/api/v2/bookmarks/{gone}"), None),
        (
            "POST",
            "/api/v2/scrobbles".to_owned(),
            Some(serde_json::json!({ "track_id": gone, "submission": true })),
        ),
    ] {
        let response = send_json(&router, verb, &uri, &token, body).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{verb} {uri}");
        assert_eq!(
            held_about(&state, owner, gone).await,
            before,
            "{verb} {uri} wrote something"
        );
    }

    // The queue is saved without the track that went, and a current track
    // that went leaves no current and no position behind it.
    let saved = call(
        "savePlayQueue",
        format!("&id={kept}&id={gone}&id={kept}&current={gone}&position=1234"),
    )
    .await;
    assert_eq!(saved["status"], "ok", "{saved}");
    assert_eq!(
        stored_queue(&state, owner).await,
        (vec![kept.to_string(), kept.to_string()], None, 0)
    );
    // A current track that is still there keeps its position.
    let saved = call(
        "savePlayQueue",
        format!("&id={gone}&id={kept}&current={kept}&position=4321"),
    )
    .await;
    assert_eq!(saved["status"], "ok", "{saved}");
    assert_eq!(
        stored_queue(&state, owner).await,
        (vec![kept.to_string()], Some(kept.to_string()), 4321)
    );
    // The native API takes the same path.
    let saved = send_json(
        &router,
        "PUT",
        "/api/v2/queue",
        &token,
        Some(serde_json::json!({ "track_ids": [gone, kept], "current": kept, "position_ms": 99 })),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        stored_queue(&state, owner).await,
        (vec![kept.to_string()], Some(kept.to_string()), 99)
    );
    // An id that never was a track is still refused, and the queue kept.
    let unknown = call(
        "savePlayQueue",
        format!("&id={kept}&id={}&current={kept}", Uuid::new_v4()),
    )
    .await;
    assert_eq!(unknown["error"]["code"], 70, "{unknown}");
    assert_eq!(
        stored_queue(&state, owner).await,
        (vec![kept.to_string()], Some(kept.to_string()), 99)
    );

    // The file comes back: the same track, with what was written before.
    write_test_wav_of_len(&music.join("Gone.wav"), 8_000 * 3);
    run_scan(&state, owner, record).await;
    assert_eq!(
        track_of("Gone.wav").await,
        gone,
        "the file comes back as itself"
    );
    let song = call("getSong", format!("&id={gone}")).await;
    assert_eq!(song["status"], "ok", "{song}");
    assert!(
        song["song"]["starred"].is_string(),
        "the star survived: {song}"
    );
    assert_eq!(song["song"]["userRating"], 4, "the rating survived: {song}");
    let bookmarks = call("getBookmarks", String::new()).await;
    let bookmark = bookmarks["bookmarks"]["bookmark"]
        .as_array()
        .unwrap()
        .iter()
        .find(|bookmark| bookmark["entry"]["id"] == gone.to_string())
        .cloned()
        .unwrap_or_else(|| panic!("the bookmark survived: {bookmarks}"));
    assert_eq!(bookmark["position"], 5000);
    // And writes on it are accepted again.
    let rated = call("setRating", format!("&id={gone}&rating=5")).await;
    assert_eq!(rated["status"], "ok", "{rated}");
}
