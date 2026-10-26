//! Native playlists: duplicate entries, public playlists read across accounts,
//! moving an entry.

use super::*;

// A playlist holds a track as many times as its owner puts it there (RFC-020
/// decision 1), on every native path that writes one: creation, appending,
/// removal by position, and the `.m3u8` round trip.
///
/// `playlist_track` was keyed on the pair, so the second copy of a track
/// violated the key and surfaced as a database failure — a 503 for what is an
/// ordinary edit — and an import reported the copy as a duplicate it had to
/// drop.
#[tokio::test]
async fn a_playlist_holds_one_track_more_than_once_on_every_native_path() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("dup-owner", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let (_library, ids) = seed_titles(
        &state,
        &config,
        owner,
        "dup-music",
        3_000,
        &["Once", "Twice"],
    )
    .await;
    let (a, b) = (ids[0].to_string(), ids[1].to_string());
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "dup-owner", &password).await;

    // One request naming one track twice.
    let created = send_json(
        &router,
        "POST",
        "/api/v2/playlists",
        &token,
        Some(serde_json::json!({ "name": "Again", "track_ids": [a, b, a] })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    assert_eq!(
        song_ids(&created["songs"]),
        vec![a.as_str(), b.as_str(), a.as_str()]
    );
    let id = created["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v2/playlists/{id}");

    // Appending a track the playlist already holds.
    let added = send_json(
        &router,
        "PATCH",
        &uri,
        &token,
        Some(serde_json::json!({ "add": [b] })),
    )
    .await;
    assert_eq!(added.status(), StatusCode::OK);
    assert_eq!(
        song_ids(&json_body(added).await["songs"]),
        vec![a.as_str(), b.as_str(), a.as_str(), b.as_str()]
    );

    // Removal is by position, so one copy goes and the other stays.
    let removed = send_json(
        &router,
        "PATCH",
        &uri,
        &token,
        Some(serde_json::json!({ "remove_indexes": [2] })),
    )
    .await;
    assert_eq!(removed.status(), StatusCode::OK);
    assert_eq!(
        song_ids(&json_body(removed).await["songs"]),
        vec![a.as_str(), b.as_str(), b.as_str()]
    );
    let reread = json_body(send_json(&router, "GET", &uri, &token, None).await).await;
    assert_eq!(
        song_ids(&reread["songs"]),
        vec![a.as_str(), b.as_str(), b.as_str()]
    );

    // The export writes every copy, and importing it places every copy.
    let exported =
        body_text(send_json(&router, "GET", &format!("{uri}/export"), &token, None).await).await;
    assert_eq!(
        exported.matches("./browse-3001.flac").count(),
        2,
        "{exported}"
    );
    let imported = router
        .clone()
        .oneshot(
            Request::post("/api/v2/playlists/import?name=Round%20again")
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "audio/x-mpegurl")
                .body(Body::from(exported))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(imported.status(), StatusCode::CREATED);
    let imported = json_body(imported).await;
    assert_eq!(imported["resolved"], 3);
    assert_eq!(imported["unresolved"], serde_json::json!([]));
    assert_eq!(
        song_ids(&imported["playlist"]["songs"]),
        vec![a.as_str(), b.as_str(), b.as_str()]
    );

    // A share is a list too, and the web client makes one of the play queue,
    // which repeats tracks freely: it was keyed on the pair as well.
    let shared = send_json(
        &router,
        "POST",
        "/api/v2/shares",
        &token,
        Some(serde_json::json!({ "track_ids": [a, a, b] })),
    )
    .await;
    assert_eq!(shared.status(), StatusCode::CREATED);
    assert_eq!(
        json_body(shared).await["track_ids"],
        serde_json::json!([a, a, b])
    );
}

/// A public playlist is readable by every other account, through that
/// account's own libraries, and writable by its owner alone (RFC-020
/// decision 2).
///
/// Every read used to filter on `owner_user_id`, so the flag Subsonic accepts
/// was stored and never had an effect. And what another account reads of it
/// is what it could read anyway: a track in a library the viewer is not a
/// member of is not shown, not counted, and not named anywhere in the answer.
#[tokio::test]
async fn a_public_playlist_is_read_by_other_accounts_through_their_own_libraries() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("public-owner", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let viewer = state
        .db
        .create_account("public-viewer", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("public-stranger", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let (shared, open) = seed_titles(
        &state,
        &config,
        owner,
        "public-shared",
        3_100,
        &["Open One", "Open Two"],
    )
    .await;
    let (_private, kept) = seed_titles(
        &state,
        &config,
        owner,
        "public-private",
        3_200,
        &["Kept Back"],
    )
    .await;
    state
        .db
        .add_library_member(
            owner,
            shared,
            viewer,
            waveflow_server::storage::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "public-owner", &password).await;
    let viewer_token = login_token(&router, "public-viewer", &password).await;
    let stranger_token = login_token(&router, "public-stranger", &password).await;
    let (open_one, open_two, hidden) = (
        open[0].to_string(),
        open[1].to_string(),
        kept[0].to_string(),
    );

    let created = json_body(
        send_json(
            &router,
            "POST",
            "/api/v2/playlists",
            &owner_token,
            Some(serde_json::json!({
                "name": "Open house",
                "track_ids": [open_one, hidden, open_two]
            })),
        )
        .await,
    )
    .await;
    let id = created["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v2/playlists/{id}");

    // Private, it is somebody else's and answers like nothing at all.
    assert_eq!(
        send_json(&router, "GET", &uri, &viewer_token, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );

    let published = send_json(
        &router,
        "PATCH",
        &uri,
        &owner_token,
        Some(serde_json::json!({ "public": true })),
    )
    .await;
    assert_eq!(published.status(), StatusCode::OK);

    // Read by the viewer through the viewer's libraries.
    let seen = send_json(&router, "GET", &uri, &viewer_token, None).await;
    assert_eq!(seen.status(), StatusCode::OK);
    let seen_text = body_text(seen).await;
    assert!(
        !seen_text.contains(&hidden),
        "a track outside the viewer's libraries leaked: {seen_text}"
    );
    let seen: serde_json::Value = serde_json::from_str(&seen_text).unwrap();
    assert_eq!(
        song_ids(&seen["songs"]),
        vec![open_one.as_str(), open_two.as_str()]
    );
    assert_eq!(seen["public"], true);
    assert_eq!(seen["owner_id"], owner.to_string());
    assert_eq!(seen["owner_username"], "public-owner");

    // The native list stays the caller's own unless it asks for the others.
    let own =
        json_body(send_json(&router, "GET", "/api/v2/playlists", &viewer_token, None).await).await;
    assert_eq!(own, serde_json::json!([]));
    let listed = json_body(
        send_json(
            &router,
            "GET",
            "/api/v2/playlists?include_public=true",
            &viewer_token,
            None,
        )
        .await,
    )
    .await;
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], id.as_str());
    assert_eq!(
        song_ids(&listed[0]["songs"]),
        vec![open_one.as_str(), open_two.as_str()]
    );

    // An account with no library in common sees the playlist and none of it.
    let stranger = json_body(send_json(&router, "GET", &uri, &stranger_token, None).await).await;
    assert_eq!(stranger["songs"], serde_json::json!([]));

    // The export is a read too, and reads through the same libraries.
    let exported = send_json(
        &router,
        "GET",
        &format!("{uri}/export"),
        &viewer_token,
        None,
    )
    .await;
    assert_eq!(exported.status(), StatusCode::OK);
    let exported = body_text(exported).await;
    assert!(exported.contains("Open One") && exported.contains("Open Two"));
    assert!(!exported.contains("Kept Back"), "{exported}");

    // Read-only to everybody but its owner, and answered as if absent.
    for (method, body) in [
        ("PATCH", Some(serde_json::json!({ "name": "Taken over" }))),
        ("PATCH", Some(serde_json::json!({ "add": [open_one] }))),
        ("DELETE", None),
    ] {
        assert_eq!(
            send_json(&router, method, &uri, &viewer_token, body)
                .await
                .status(),
            StatusCode::NOT_FOUND,
            "{method} by a non-owner"
        );
    }
    let intact = json_body(send_json(&router, "GET", &uri, &owner_token, None).await).await;
    assert_eq!(intact["name"], "Open house");
    assert_eq!(
        song_ids(&intact["songs"]),
        vec![open_one.as_str(), hidden.as_str(), open_two.as_str()]
    );

    // Withdrawn, it is gone again for everybody else.
    send_json(
        &router,
        "PATCH",
        &uri,
        &owner_token,
        Some(serde_json::json!({ "public": false })),
    )
    .await;
    assert_eq!(
        send_json(&router, "GET", &uri, &viewer_token, None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

/// Moving an entry of a playlist, the verb RFC-020 left open: positions are
/// the ones the caller was shown, a hidden entry keeps its place, one copy of
/// a repeated track moves and the other stays, a stale view is refused rather
/// than applied to another entry, and the move reaches the owner's journal.
#[tokio::test]
async fn a_playlist_entry_moves_to_the_position_it_was_shown_at() {
    let (_temp, config, state) = test_app().await;
    let password = uuid::Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("move-owner", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("move-other", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let (_library, ids) = seed_titles(
        &state,
        &config,
        owner,
        "move-music",
        3_500,
        &["First", "Hidden", "Second", "Third"],
    )
    .await;
    let [a, hidden, b, c] = [ids[0], ids[1], ids[2], ids[3]].map(|id| id.to_string());
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "move-owner", &password).await;
    let created = json_body(
        send_json(
            &router,
            "POST",
            "/api/v2/playlists",
            &token,
            Some(serde_json::json!({ "name": "Order", "track_ids": [a, hidden, b, c, a] })),
        )
        .await,
    )
    .await;
    let id = created["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/v2/playlists/{id}/move");
    // The file goes, so the listener is shown four entries: a, b, c, a.
    sqlx::query("UPDATE track SET is_available=0 WHERE id=?")
        .bind(&hidden)
        .execute(state.db.pool())
        .await
        .unwrap();

    // The last copy of `a` to the front, past the hidden entry.
    let moved = send_json(
        &router,
        "POST",
        &uri,
        &token,
        Some(serde_json::json!({ "from": 3, "to": 0, "track_id": a })),
    )
    .await;
    assert_eq!(moved.status(), StatusCode::OK);
    assert_eq!(
        song_ids(&json_body(moved).await["songs"]),
        vec![a.as_str(), a.as_str(), b.as_str(), c.as_str()]
    );
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT track_id FROM playlist_track WHERE playlist_id=? ORDER BY position",
    )
    .bind(&id)
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        stored,
        vec![a.clone(), a.clone(), hidden.clone(), b.clone(), c.clone()],
        "the hidden entry keeps its place among the others"
    );
    let journal: String = sqlx::query_scalar(
        "SELECT payload_json FROM sync_event WHERE entity_type='playlist' \
         ORDER BY cursor DESC LIMIT 1",
    )
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    let journal: serde_json::Value = serde_json::from_str(&journal).unwrap();
    assert_eq!(
        journal["track_ids"],
        serde_json::json!([a, a, hidden, b, c]),
        "a device mirrors the new order from the journal"
    );

    // To the end.
    let to_end = json_body(
        send_json(
            &router,
            "POST",
            &uri,
            &token,
            Some(serde_json::json!({ "from": 0, "to": 3 })),
        )
        .await,
    )
    .await;
    assert_eq!(
        song_ids(&to_end["songs"]),
        vec![a.as_str(), b.as_str(), c.as_str(), a.as_str()]
    );

    // A view gone stale: the caller believes `c` is first.
    let stale = send_json(
        &router,
        "POST",
        &uri,
        &token,
        Some(serde_json::json!({ "from": 0, "to": 2, "track_id": c })),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let beyond = send_json(
        &router,
        "POST",
        &uri,
        &token,
        Some(serde_json::json!({ "from": 4, "to": 0 })),
    )
    .await;
    assert_eq!(beyond.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Removal by the position shown, guarded the same way: a stale view is
    // refused, and the hidden entry stays where it is.
    let remove = format!("/api/v2/playlists/{id}/remove");
    let stale = send_json(
        &router,
        "POST",
        &remove,
        &token,
        Some(serde_json::json!({ "position": 1, "track_id": c })),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    let removed = send_json(
        &router,
        "POST",
        &remove,
        &token,
        Some(serde_json::json!({ "position": 3, "track_id": a })),
    )
    .await;
    assert_eq!(removed.status(), StatusCode::OK);
    assert_eq!(
        song_ids(&json_body(removed).await["songs"]),
        vec![a.as_str(), b.as_str(), c.as_str()]
    );
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT track_id FROM playlist_track WHERE playlist_id=? ORDER BY position",
    )
    .bind(&id)
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        stored,
        vec![a.clone(), hidden.clone(), b.clone(), c.clone()]
    );

    // Someone else's playlist, public or not, answers as a missing one.
    sqlx::query("UPDATE playlist SET public=1 WHERE id=?")
        .bind(&id)
        .execute(state.db.pool())
        .await
        .unwrap();
    let other_token = login_token(&router, "move-other", &password).await;
    let refused = send_json(
        &router,
        "POST",
        &uri,
        &other_token,
        Some(serde_json::json!({ "from": 0, "to": 1 })),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    let unchanged = json_body(
        send_json(
            &router,
            "GET",
            &format!("/api/v2/playlists/{id}"),
            &token,
            None,
        )
        .await,
    )
    .await;
    assert_eq!(
        song_ids(&unchanged["songs"]),
        vec![a.as_str(), b.as_str(), c.as_str()]
    );
    let refused = send_json(
        &router,
        "POST",
        &remove,
        &other_token,
        Some(serde_json::json!({ "position": 0 })),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
}
