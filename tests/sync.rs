//! The two change feeds: the user journal and the library event stream —
//! claims, cursors and tenancy.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Request;
use axum::http::StatusCode;
use futures_util::StreamExt;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;
use waveflow_server::services::ServiceError;
use waveflow_server::services::MAX_QUEUE_TRACKS;
use waveflow_server::services::MAX_SHARE_TRACKS;
use waveflow_server::sync::MutationContext;
use waveflow_server::sync::SyncError;
use waveflow_server::sync::MAX_SYNC_LIMIT;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

#[tokio::test]
async fn sync_journal_is_idempotent_cursor_based_and_tenant_isolated() {
    let (_temp, config, state) = test_app().await;
    let password = Uuid::new_v4().to_string();
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("sync-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("sync-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("sync-newcomer", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("sync-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Sync library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    state
        .db
        .apply_catalog_track(
            library,
            scan,
            &browse_input(
                0,
                "Synchronized Song",
                "Remote Album",
                "Remote Artist",
                Some(1),
                Some(1),
            ),
            None,
            false,
        )
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let track = state.db.list_tracks_for_user(owner, library).await.unwrap()[0].id;

    let mut notices = state.sync.subscribe();
    let router = waveflow_server::app(&config, state.clone());
    let login = |username: &'static str| {
        let router = router.clone();
        let password = password.clone();
        async move {
            let response = router
                .oneshot(json_request(
                    "/api/v2/auth/login",
                    serde_json::json!({
                        "username": username,
                        "password": password,
                        "device_name": format!("{username} desktop")
                    }),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            json_body(response).await
        }
    };
    let owner_login = login("sync-owner").await;
    let owner_token = owner_login["access_token"].as_str().unwrap().to_owned();
    let device_id = owner_login["device_id"].as_str().unwrap().to_owned();
    let intruder_login = login("sync-intruder").await;
    let intruder_token = intruder_login["access_token"].as_str().unwrap().to_owned();
    let intruder_device_id = intruder_login["device_id"].as_str().unwrap().to_owned();

    for invalid_limit in [0, MAX_SYNC_LIMIT + 1, i64::MAX] {
        assert!(matches!(
            state.sync.changes(owner, 0, invalid_limit).await,
            Err(SyncError::Invalid)
        ));
    }
    assert!(matches!(
        state.sync.changes(owner, -1, 1).await,
        Err(SyncError::Invalid)
    ));
    let direct_foreign_device = state
        .services
        .set_star_with_context(
            owner,
            "track",
            track,
            true,
            MutationContext {
                operation_id: Uuid::new_v4(),
                origin_device_id: Some(Uuid::parse_str(&intruder_device_id).unwrap()),
            },
        )
        .await;
    assert!(matches!(direct_foreign_device, Err(ServiceError::Invalid)));

    let mutate =
        |method: &'static str, uri: String, operation_id: Uuid, body: Option<serde_json::Value>| {
            let router = router.clone();
            let owner_token = owner_token.clone();
            let device_id = device_id.clone();
            async move {
                let request = Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", format!("Bearer {owner_token}"))
                    .header("x-waveflow-operation-id", operation_id.to_string())
                    .header("x-waveflow-device-id", device_id);
                let request = match body {
                    Some(body) => request
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                    None => request.body(Body::empty()).unwrap(),
                };
                router.oneshot(request).await.unwrap()
            }
        };

    // Retrying a mutation with the same operation UUID must neither duplicate
    // the business row nor append another event.
    let favorite_operation = Uuid::new_v4();
    for _ in 0..2 {
        let response = mutate(
            "PUT",
            format!("/api/v2/favorites/track/{track}"),
            favorite_operation,
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    let star_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_star WHERE user_id=?")
        .bind(owner.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(star_count, 1);

    let mismatched_replay = mutate(
        "POST",
        "/api/v2/playlists".into(),
        favorite_operation,
        Some(serde_json::json!({ "name": "Wrong replay type", "track_ids": [track] })),
    )
    .await;
    // Same operation id, different intent: a conflict, not a malformed body.
    // The distinction matters to a client draining an offline queue — a 422
    // means fix the payload, a 409 means mint a new operation id.
    assert_eq!(mismatched_replay.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(mismatched_replay).await["code"],
        "conflict",
        "conflicts must be distinguishable from validation errors"
    );

    let inverted_favorite = mutate(
        "DELETE",
        format!("/api/v2/favorites/track/{track}"),
        favorite_operation,
        None,
    )
    .await;
    assert_eq!(inverted_favorite.status(), StatusCode::CONFLICT);
    let star_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_star WHERE user_id=?")
        .bind(owner.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(star_count, 1);

    let scrobble_operation = Uuid::new_v4();
    for _ in 0..2 {
        let response = mutate(
            "POST",
            "/api/v2/scrobbles".into(),
            scrobble_operation,
            Some(serde_json::json!({ "track_id": track, "submission": true })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    let play_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM play_event WHERE user_id=?")
        .bind(owner.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(play_count, 1, "a retried scrobble must stay idempotent");

    let create_operation = Uuid::new_v4();
    let mut playlist_ids = Vec::new();
    for _ in 0..2 {
        let response = mutate(
            "POST",
            "/api/v2/playlists".into(),
            create_operation,
            Some(serde_json::json!({ "name": "Synced", "track_ids": [track] })),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        playlist_ids.push(json_body(response).await["id"].as_str().unwrap().to_owned());
    }
    assert_eq!(playlist_ids[0], playlist_ids[1]);
    let different_playlist = mutate(
        "POST",
        "/api/v2/playlists".into(),
        create_operation,
        Some(serde_json::json!({
            "name": "Different playlist",
            "track_ids": [track]
        })),
    )
    .await;
    assert_eq!(different_playlist.status(), StatusCode::CONFLICT);
    let playlist_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM playlist WHERE owner_user_id=?")
            .bind(owner.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(playlist_count, 1);

    let share_operation = Uuid::new_v4();
    let share_request = || {
        mutate(
            "POST",
            "/api/v2/shares".into(),
            share_operation,
            Some(serde_json::json!({
                "track_ids": [track],
                "description": "Synchronized share"
            })),
        )
    };
    // The identifier is read before the response is dropped: a replay that
    // answered 201 with a second share would pass a status check and be exactly
    // the failure this is here to catch.
    let lost_response = share_request().await;
    assert_eq!(lost_response.status(), StatusCode::CREATED);
    let lost_id = json_body(lost_response).await["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let replayed_share = share_request().await;
    assert_eq!(replayed_share.status(), StatusCode::CREATED);
    let replayed_share = json_body(replayed_share).await;
    let share_id = replayed_share["id"].as_str().unwrap();
    assert_eq!(
        share_id, lost_id,
        "a replayed operation returns the share the original created"
    );
    let share_url = replayed_share["url"].as_str().unwrap();
    let public_share = router
        .clone()
        .oneshot(Request::get(share_url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(public_share.status(), StatusCode::OK);

    let listed_shares = router
        .clone()
        .oneshot(
            Request::get("/api/v2/shares")
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let listed_shares = json_body(listed_shares).await;
    assert_eq!(listed_shares[0]["id"], share_id);
    assert!(listed_shares[0].get("url").is_none());

    let mut notice_cursors = Vec::new();
    for _ in 0..4 {
        let notice = tokio::time::timeout(std::time::Duration::from_secs(10), notices.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(notice.0, owner);
        notice_cursors.push(notice.1.cursor);
    }
    assert!(notice_cursors.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(matches!(
        notices.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));

    let mut after = 0;
    let mut paged_changes = Vec::new();
    loop {
        let changes = router
            .clone()
            .oneshot(
                Request::get(format!("/api/v2/sync/changes?after={after}&limit=1"))
                    .header("authorization", format!("Bearer {owner_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(changes.status(), StatusCode::OK);
        let page = json_body(changes).await;
        let page_changes = page["changes"].as_array().unwrap();
        if page_changes.is_empty() {
            assert!(!page["has_more"].as_bool().unwrap());
            break;
        }
        let returned_cursor = page_changes[0]["cursor"].as_i64().unwrap();
        assert_eq!(page["next_cursor"], returned_cursor);
        paged_changes.push(page_changes[0].clone());
        after = returned_cursor;
        if !page["has_more"].as_bool().unwrap() {
            break;
        }
    }
    assert_eq!(paged_changes.len(), 4);
    assert_eq!(
        paged_changes[0]["operation_id"],
        favorite_operation.to_string()
    );

    // Another account writes last, so the journal's global cursor now sits
    // above this user's. The socket must still report the user's own position:
    // it exists to say "your state moved", and notifying on the global cursor
    // would wake every client on every other account's write, each false wake
    // costing a /changes round trip that returns nothing.
    let noise_operation = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sync_operation (user_id, operation_id, created_at) VALUES (?, ?, ?)")
        .bind(intruder_login["user"]["id"].as_str().unwrap())
        .bind(&noise_operation)
        .bind(now_ms())
        .execute(state.db.pool())
        .await
        .unwrap();
    let noise_cursor: i64 = sqlx::query_scalar(
        "INSERT INTO sync_event (event_id, user_id, operation_id, entity_type, entity_id, \
                                 action, payload_json, changed_at) \
         VALUES (?, ?, ?, 'favorite', ?, 'upsert', '{}', ?) RETURNING cursor",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(intruder_login["user"]["id"].as_str().unwrap())
    .bind(&noise_operation)
    .bind(Uuid::new_v4().to_string())
    .bind(now_ms())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert!(
        noise_cursor > paged_changes[3]["cursor"].as_i64().unwrap(),
        "the other account must now hold the journal's highest cursor"
    );

    // The real WebSocket route sends the durable cursor immediately when a
    // reconnecting client is behind. The lagged-receiver branch is covered by
    // the focused `http` unit test using the same serve-path helper.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server_router = router.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, server_router).await.unwrap();
    });
    let mut socket_request = format!("ws://{address}/api/v2/sync/socket?after=0")
        .into_client_request()
        .unwrap();
    socket_request.headers_mut().insert(
        "authorization",
        format!("Bearer {owner_token}").parse().unwrap(),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(socket_request)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let notice = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let notice: serde_json::Value = serde_json::from_str(notice.to_text().unwrap()).unwrap();
    assert_eq!(
        notice["cursor"], paged_changes[3]["cursor"],
        "the socket reports this user's cursor, not the journal's"
    );
    assert_ne!(
        notice["cursor"].as_i64().unwrap(),
        noise_cursor,
        "falling back to the global cursor would wake clients for other accounts"
    );
    socket.close(None).await.unwrap();
    server.abort();

    let snapshot = router
        .clone()
        .oneshot(
            Request::get("/api/v2/sync/snapshot")
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let snapshot = json_body(snapshot).await;
    assert_eq!(snapshot["favorites"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["history"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["playlists"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["shares"].as_array().unwrap().len(), 1);
    assert!(snapshot["shares"][0].get("url").is_none());
    let cursor = snapshot["cursor"].as_i64().unwrap();

    let ack = router
        .clone()
        .oneshot(
            Request::put("/api/v2/sync/ack")
                .header("authorization", format!("Bearer {owner_token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "device_id": device_id, "cursor": cursor }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ack.status(), StatusCode::NO_CONTENT);

    let future_ack = router
        .clone()
        .oneshot(
            Request::put("/api/v2/sync/ack")
                .header("authorization", format!("Bearer {owner_token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "device_id": device_id,
                        "cursor": cursor + 1_000
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(future_ack.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let foreign_device = Request::put(format!("/api/v2/favorites/track/{track}"))
        .header("authorization", format!("Bearer {owner_token}"))
        .header("x-waveflow-operation-id", Uuid::new_v4().to_string())
        .header("x-waveflow-device-id", &intruder_device_id)
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router
            .clone()
            .oneshot(foreign_device)
            .await
            .unwrap()
            .status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let foreign = router
        .clone()
        .oneshot(
            Request::get("/api/v2/sync/changes?after=0")
                .header("authorization", format!("Bearer {intruder_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // The other account sees its own event and nothing else — the owner's
    // entries never cross, even though they share one cursor sequence.
    let foreign = json_body(foreign).await;
    let foreign_changes = foreign["changes"].as_array().unwrap();
    assert_eq!(foreign_changes.len(), 1);
    assert_eq!(foreign_changes[0]["operation_id"], noise_operation);

    // `cursor` is one global sequence, so a second account's first event lands
    // far above zero. Deriving the retention floor from that account's own
    // MIN(cursor) reported its perfectly valid cursor as expired — a bug no
    // single-tenant test could see, since there the two are the same number.
    let intruder_id = intruder_login["user"]["id"].as_str().unwrap().to_owned();
    let latecomer_operation = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO sync_operation (user_id, operation_id, created_at) VALUES (?, ?, ?)")
        .bind(&intruder_id)
        .bind(&latecomer_operation)
        .bind(now_ms())
        .execute(state.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO sync_event (event_id, user_id, operation_id, entity_type, entity_id, \
                                 action, payload_json, changed_at) \
         VALUES (?, ?, ?, 'favorite', ?, 'upsert', '{}', ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&intruder_id)
    .bind(&latecomer_operation)
    .bind(Uuid::new_v4().to_string())
    .bind(now_ms())
    .execute(state.db.pool())
    .await
    .unwrap();
    let latecomer = router
        .clone()
        .oneshot(
            Request::get("/api/v2/sync/changes?after=0")
                .header("authorization", format!("Bearer {intruder_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        latecomer.status(),
        StatusCode::OK,
        "a newcomer's cursor is not expired just because others wrote first"
    );
    // A 200 alone would also pass on an empty page, which is what a wrongly
    // filtered query would return. Check the event actually came back, and at
    // a cursor above zero — the whole point is that it sits high in the global
    // sequence while the caller resumes from nothing.
    let latecomer = json_body(latecomer).await;
    let delivered = latecomer["changes"].as_array().unwrap();
    let served = delivered
        .iter()
        .find(|change| change["operation_id"] == latecomer_operation)
        .expect("the late event must be served, not swallowed by an expiry check");
    assert!(
        served["cursor"].as_i64().unwrap() > 0,
        "the event sits in the shared sequence, not at the caller's origin"
    );

    // Retention contract. The journal is append-only in v2.0, so this cannot
    // happen in production — the gap is forced here by deleting the head of the
    // journal, the way a future compaction would. A client resuming from below
    // the surviving floor must be told to re-snapshot rather than handed the
    // tail, which would look like a successful catch-up over skipped events.
    let floor: i64 = sqlx::query_scalar("SELECT MIN(cursor) FROM sync_event WHERE user_id=?")
        .bind(owner.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM sync_event WHERE user_id=? AND cursor<=?")
        .bind(owner.to_string())
        .bind(floor + 1)
        .execute(state.db.pool())
        .await
        .unwrap();
    let expired = router
        .clone()
        .oneshot(
            Request::get("/api/v2/sync/changes?after=0")
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(expired).await["code"],
        "cursor_expired",
        "a re-snapshot signal must be distinguishable from an idempotency conflict"
    );

    // A cursor at or above the surviving floor is still served — this covers a
    // client that never fell behind, NOT a recovery path.
    //
    // Recovering from `cursor_expired` by resuming at `floor + 1` would be
    // wrong: the compacted events are gone, so the projection would stay
    // permanently short of whatever they carried, with nothing to signal it.
    // A full snapshot is the only correct recovery, which is what the error
    // code asks for.
    let resumed = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/sync/changes?after={}", floor + 1))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resumed.status(), StatusCode::OK);

    // And the recovery itself must terminate. An account with no events of its
    // own is the case that loops: a per-user snapshot watermark hands it cursor
    // 0, which sits below the journal floor, so it re-snapshots, is refused
    // again, and never progresses. The watermark is global, so the cursor a
    // snapshot returns is always resumable.
    let newcomer_login = login("sync-newcomer").await;
    let newcomer_token = newcomer_login["access_token"].as_str().unwrap().to_owned();
    let newcomer_device_id = newcomer_login["device_id"].as_str().unwrap().to_owned();
    let recovery = router
        .clone()
        .oneshot(
            Request::get("/api/v2/sync/snapshot")
                .header("authorization", format!("Bearer {newcomer_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(recovery.status(), StatusCode::OK);
    let recovery_cursor = json_body(recovery).await["cursor"].as_i64().unwrap();
    // The watermark is the journal's own high-water mark, not a value that
    // merely happens to clear the floor. Asserting resumability alone would
    // also pass on any number above the floor, including a per-user one that
    // clears it by luck on this fixture.
    let journal_max: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(cursor), 0) FROM sync_event")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        recovery_cursor, journal_max,
        "a snapshot resumes from the journal's high-water mark"
    );
    let after_recovery = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/sync/changes?after={recovery_cursor}"))
                .header("authorization", format!("Bearer {newcomer_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        after_recovery.status(),
        StatusCode::OK,
        "a snapshot cursor must always be resumable, or recovery never ends"
    );
    // The same cursor must be acknowledgeable, otherwise a client that recovers
    // correctly still reports a failed ACK on every cycle.
    let acked = router
        .oneshot(
            Request::put("/api/v2/sync/ack")
                .header("authorization", format!("Bearer {newcomer_token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "device_id": &newcomer_device_id,
                        "cursor": recovery_cursor
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(acked.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn sync_claim_precedes_state_validation_and_invalid_claims_roll_back() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password(&Uuid::new_v4().to_string()).unwrap();
    let owner = state
        .db
        .create_account("claim-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let listener = state
        .db
        .create_account("claim-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("claim-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Claim library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    state
        .db
        .apply_catalog_track(
            library,
            scan,
            &browse_input(
                0,
                "Claimed Song",
                "Claimed Album",
                "Claimed Artist",
                Some(1),
                Some(1),
            ),
            None,
            false,
        )
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let track = state.db.list_tracks_for_user(owner, library).await.unwrap()[0].id;

    let playlist_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    let playlist = state
        .services
        .create_playlist_with_context(owner, "Claimed playlist", &[track], playlist_context)
        .await
        .unwrap();
    state
        .services
        .delete_playlist(owner, playlist.id)
        .await
        .unwrap();
    let missing_playlist_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    assert!(matches!(
        state
            .services
            .update_playlist_with_context(
                owner,
                playlist.id,
                Some("Changed after deletion"),
                None,
                None,
                &[],
                &[],
                Default::default(),
                missing_playlist_context,
            )
            .await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        state
            .services
            .update_playlist_with_context(
                owner,
                playlist.id,
                Some("Divergent replay after deletion"),
                None,
                None,
                &[],
                &[],
                Default::default(),
                playlist_context,
            )
            .await,
        Err(ServiceError::Conflict)
    ));

    state
        .db
        .add_library_member(owner, library, listener, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    let inaccessible_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    state
        .services
        .set_star_with_context(listener, "track", track, true, inaccessible_context)
        .await
        .unwrap();
    assert!(state
        .db
        .remove_library_member(owner, library, listener, now_ms())
        .await
        .unwrap());
    state
        .services
        .set_star_with_context(listener, "track", track, true, inaccessible_context)
        .await
        .unwrap();
    // The row survives the replay, but a revoked membership must stop exposing
    // it: favourites are filtered by visibility exactly like ratings are.
    assert!(state
        .services
        .starred_ids(listener)
        .await
        .unwrap()
        .iter()
        .all(|(_, entity_id, _)| *entity_id != track));
    assert!(matches!(
        state
            .services
            .set_rating_with_context(listener, "track", track, 5, inaccessible_context)
            .await,
        Err(ServiceError::Conflict)
    ));
    let fresh_inaccessible_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    assert!(matches!(
        state
            .services
            .set_rating_with_context(listener, "track", track, 5, fresh_inaccessible_context)
            .await,
        Err(ServiceError::NotFound)
    ));

    let invalid_replay_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    state
        .services
        .set_rating_with_context(owner, "track", track, 5, invalid_replay_context)
        .await
        .unwrap();
    assert!(matches!(
        state
            .services
            .set_rating_with_context(owner, "track", track, 4, invalid_replay_context)
            .await,
        Err(ServiceError::Conflict)
    ));

    let rolled_back_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    assert!(matches!(
        state
            .services
            .set_rating_with_context(owner, "track", track, 6, rolled_back_context)
            .await,
        Err(ServiceError::Invalid)
    ));
    let reservation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_operation WHERE user_id=? AND operation_id=?",
    )
    .bind(owner.to_string())
    .bind(rolled_back_context.operation_id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(reservation_count, 0);
    state
        .services
        .set_rating_with_context(owner, "track", track, 4, rolled_back_context)
        .await
        .unwrap();

    let oversized_queue = vec![track; MAX_QUEUE_TRACKS + 1];
    assert!(matches!(
        state
            .services
            .save_queue(owner, &oversized_queue, Some(track), 0, Some("limit-test"))
            .await,
        Err(ServiceError::Invalid)
    ));
    let oversized_share = vec![track; MAX_SHARE_TRACKS + 1];
    assert!(matches!(
        state
            .services
            .create_share(owner, &oversized_share, Some("limit-test"), None)
            .await,
        Err(ServiceError::Invalid)
    ));

    state
        .services
        .save_queue(
            owner,
            &[track, track],
            Some(track),
            0,
            Some("duplicate-test"),
        )
        .await
        .unwrap();
    let duplicate_queue = state.services.queue(owner).await.unwrap().unwrap();
    assert_eq!(
        duplicate_queue
            .songs
            .iter()
            .map(|song| song.id)
            .collect::<Vec<_>>(),
        vec![track, track]
    );
    let positions = sqlx::query_scalar::<_, i64>(
        "SELECT position FROM play_queue_track WHERE user_id=? ORDER BY position",
    )
    .bind(owner.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(positions, vec![0, 1]);

    let aggregate_playlist = state
        .services
        .create_playlist(owner, "Unavailable aggregate", &[track])
        .await
        .unwrap();
    let aggregate_share = state
        .services
        .create_share(owner, &[track], Some("Unavailable aggregate"), None)
        .await
        .unwrap();
    let empty_scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(empty_scan, 1, false).await.unwrap();
    assert_eq!(
        state
            .db
            .mark_unseen_unavailable(library, empty_scan)
            .await
            .unwrap(),
        1
    );
    state.db.finish_scan_job(empty_scan, 1).await.unwrap();
    let updated_playlist = state
        .services
        .update_playlist(
            owner,
            aggregate_playlist.id,
            Some("Unavailable aggregate renamed"),
            None,
            None,
            &[],
            &[],
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(updated_playlist.name, "Unavailable aggregate renamed");
    assert!(updated_playlist.songs.is_empty());
    let persisted_playlist_tracks: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM playlist_track WHERE playlist_id=?")
            .bind(aggregate_playlist.id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(persisted_playlist_tracks, 1);
    let unavailable_queue = state.services.queue(owner).await.unwrap().unwrap();
    assert!(unavailable_queue.songs.is_empty());
    // The saved row still names a current track. The projection must not, once
    // that track is no longer among the songs it hands back.
    assert!(unavailable_queue.current.is_none());
    assert!(state
        .services
        .shares(owner)
        .await
        .unwrap()
        .into_iter()
        .find(|share| share.id == aggregate_share.id)
        .unwrap()
        .songs
        .is_empty());
    // A visitor sees the same thing as the owner: the share survives its last
    // track going unavailable, rather than answering not-found after the visit
    // has already been counted.
    let visited = state
        .services
        .public_share(
            aggregate_share
                .url_token
                .as_deref()
                .expect("a freshly created share carries its token"),
        )
        .await
        .expect("a share outlives a track that went unavailable");
    assert!(visited.songs.is_empty());
    state.services.sync_snapshot(owner, 100).await.unwrap();
}

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
    let _ = stranger;
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
    let (status, _) = feed(stranger_token, 0, 500).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
