//! An account's own data — playlists, shares, favourites, queue, history — kept
//! to that account.

use super::*;

#[tokio::test]
async fn native_user_data_endpoints_round_trip_and_isolate_tenants() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("data-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("data-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("data-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "User data",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan_id = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 2, false).await.unwrap();
    for (index, (title, artist)) in [
        ("First Light", "Lumen Drift"),
        ("Slow Tide", "Écho Solaire"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(
            index,
            title,
            "Aurora Fields",
            artist,
            Some(index as i64 + 1),
            Some(1),
        );
        input.album_artist = Some("Lumen Drift".into());
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "data-owner", password).await;
    let intruder_token = login_token(&router, "data-intruder", password).await;

    let send =
        |method: &'static str, uri: String, token: String, body: Option<serde_json::Value>| {
            let router = router.clone();
            async move {
                let request = Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("authorization", format!("Bearer {token}"));
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

    // Collect the track ids through the native browse surface.
    let albums = send("GET", "/api/v2/albums".into(), owner_token.clone(), None).await;
    let albums = json_body(albums).await;
    let album_id = albums[0]["id"].as_str().unwrap().to_owned();
    let detail = send(
        "GET",
        format!("/api/v2/albums/{album_id}"),
        owner_token.clone(),
        None,
    )
    .await;
    let detail = json_body(detail).await;
    let first = detail["songs"][0]["id"].as_str().unwrap().to_owned();
    let second = detail["songs"][1]["id"].as_str().unwrap().to_owned();
    let first_artist_id = detail["songs"][0]["artist_id"]
        .as_str()
        .expect("the fixture track has an artist")
        .to_owned();
    let second_artist_id = detail["songs"][1]["artist_id"]
        .as_str()
        .expect("the second fixture track has an artist")
        .to_owned();
    assert_ne!(first_artist_id, second_artist_id);

    // Individual tracks can be resolved for favorites and queue hydration,
    // while the same public id remains opaque to another tenant.
    let track = send(
        "GET",
        format!("/api/v2/tracks/{first}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(track.status(), StatusCode::OK);
    let track = json_body(track).await;
    assert_eq!(track["id"], first);
    assert_eq!(track["artist_id"], first_artist_id);
    let foreign_track = send(
        "GET",
        format!("/api/v2/tracks/{first}"),
        intruder_token.clone(),
        None,
    )
    .await;
    assert_eq!(foreign_track.status(), StatusCode::NOT_FOUND);

    // Playlists: create, read back, mutate, then delete.
    let created = send(
        "POST",
        "/api/v2/playlists".into(),
        owner_token.clone(),
        Some(serde_json::json!({ "name": "Evening", "track_ids": [first] })),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    let playlist_id = created["id"].as_str().unwrap().to_owned();
    assert_eq!(created["songs"].as_array().unwrap().len(), 1);
    assert_eq!(created["songs"][0]["id"], first);
    assert_eq!(created["songs"][0]["artist_id"], first_artist_id);

    let listed = send("GET", "/api/v2/playlists".into(), owner_token.clone(), None).await;
    assert_eq!(json_body(listed).await.as_array().unwrap().len(), 1);

    let updated = send(
        "PATCH",
        format!("/api/v2/playlists/{playlist_id}"),
        owner_token.clone(),
        Some(serde_json::json!({ "comment": "late night", "add": [second] })),
    )
    .await;
    assert_eq!(updated.status(), StatusCode::OK);
    let updated = json_body(updated).await;
    assert_eq!(updated["comment"], "late night");
    assert_eq!(updated["songs"].as_array().unwrap().len(), 2);
    assert_eq!(updated["songs"][0]["id"], first);
    assert_eq!(updated["songs"][0]["artist_id"], first_artist_id);
    assert_eq!(updated["songs"][1]["id"], second);
    assert_eq!(updated["songs"][1]["artist_id"], second_artist_id);

    // Favorites round-trip through the dedicated collection.
    let starred = send(
        "PUT",
        format!("/api/v2/favorites/track/{first}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(starred.status(), StatusCode::NO_CONTENT);
    let favorites = send("GET", "/api/v2/favorites".into(), owner_token.clone(), None).await;
    let favorites = json_body(favorites).await;
    assert_eq!(favorites.as_array().unwrap().len(), 1);
    assert_eq!(favorites[0]["entity_type"], "track");
    assert_eq!(favorites[0]["entity_id"], first);

    let unstarred = send(
        "DELETE",
        format!("/api/v2/favorites/track/{first}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(unstarred.status(), StatusCode::NO_CONTENT);
    let favorites = send("GET", "/api/v2/favorites".into(), owner_token.clone(), None).await;
    assert!(json_body(favorites).await.as_array().unwrap().is_empty());

    // Ratings are read back through the browse surface, not inferred.
    let rated = send(
        "PUT",
        format!("/api/v2/ratings/track/{first}"),
        owner_token.clone(),
        Some(serde_json::json!({ "rating": 4 })),
    )
    .await;
    assert_eq!(rated.status(), StatusCode::NO_CONTENT);
    let detail = send(
        "GET",
        format!("/api/v2/albums/{album_id}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(json_body(detail).await["songs"][0]["user_rating"], 4);

    // Out-of-range ratings and unknown entity kinds are refused, not stored.
    let invalid = send(
        "PUT",
        format!("/api/v2/ratings/track/{first}"),
        owner_token.clone(),
        Some(serde_json::json!({ "rating": 6 })),
    )
    .await;
    assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let unknown_kind = send(
        "PUT",
        format!("/api/v2/favorites/banana/{first}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(unknown_kind.status(), StatusCode::UNPROCESSABLE_ENTITY);

    for invalid_time in [-1, now_ms().saturating_add(10 * 60 * 1_000)] {
        let invalid = send(
            "POST",
            "/api/v2/scrobbles".into(),
            owner_token.clone(),
            Some(serde_json::json!({
                "track_id": first,
                "submission": true,
                "played_at": invalid_time
            })),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    let scrobbled = send(
        "POST",
        "/api/v2/scrobbles".into(),
        owner_token.clone(),
        Some(serde_json::json!({ "track_id": first, "submission": true })),
    )
    .await;
    assert_eq!(scrobbled.status(), StatusCode::NO_CONTENT);
    for invalid_limit in [-1, MAX_HISTORY_LIMIT + 1] {
        assert!(matches!(
            state.services.history(owner, invalid_limit).await,
            Err(ServiceError::Invalid)
        ));
    }

    // The queue survives a write/read round-trip.
    let saved = send(
        "PUT",
        "/api/v2/queue".into(),
        owner_token.clone(),
        Some(serde_json::json!({
            "track_ids": [first, second],
            "current": first,
            "position_ms": 4200,
            "client": "test"
        })),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::NO_CONTENT);
    let queue = send("GET", "/api/v2/queue".into(), owner_token.clone(), None).await;
    let queue = json_body(queue).await;
    assert_eq!(queue["position_ms"], 4200);
    assert_eq!(queue["current"], first);
    assert_eq!(queue["songs"].as_array().unwrap().len(), 2);

    // Multiple shares retain their independent track ordering when the
    // aggregate loader batches all share rows.
    for track_ids in [vec![second.clone(), first.clone()], vec![first.clone()]] {
        let share = send(
            "POST",
            "/api/v2/shares".into(),
            owner_token.clone(),
            Some(serde_json::json!({ "track_ids": track_ids })),
        )
        .await;
        assert_eq!(share.status(), StatusCode::CREATED);
        assert!(json_body(share).await["url"].as_str().is_some());
    }
    let shares = send("GET", "/api/v2/shares".into(), owner_token.clone(), None).await;
    let shares = json_body(shares).await;
    assert!(shares
        .as_array()
        .unwrap()
        .iter()
        .all(|share| share.get("url").is_none()));
    let song_orders = shares
        .as_array()
        .unwrap()
        .iter()
        .map(|share| {
            share["track_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|track_id| track_id.as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert!(song_orders.contains(&vec![second.to_string(), first.to_string()]));
    assert!(song_orders.contains(&vec![first.to_string()]));
    let share_columns =
        sqlx::query_scalar::<_, String>("SELECT name FROM pragma_table_info('share')")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert!(share_columns.contains(&"token_hash".to_owned()));
    assert!(!share_columns.contains(&"token_nonce".to_owned()));
    assert!(!share_columns.contains(&"token_ciphertext".to_owned()));

    // An expiry set by mistake must be liftable. COALESCE alone made it
    // permanent: omitting the field and sending null were the same bind, so the
    // owner's only recourse was deleting the share and publishing a new URL.
    let expiring = send(
        "POST",
        "/api/v2/shares".into(),
        owner_token.clone(),
        Some(serde_json::json!({
            "track_ids": [first.clone()],
            "expires_at": now_ms() + 3_600_000
        })),
    )
    .await;
    let expiring_id = json_body(expiring).await["id"].as_str().unwrap().to_owned();
    let patched = send(
        "PATCH",
        format!("/api/v2/shares/{expiring_id}"),
        owner_token.clone(),
        Some(serde_json::json!({ "description": "kept" })),
    )
    .await;
    // Omitting the field still leaves it alone — clearing never fires by accident.
    assert!(json_body(patched).await["expires_at"].is_i64());
    let cleared = send(
        "PATCH",
        format!("/api/v2/shares/{expiring_id}"),
        owner_token.clone(),
        Some(serde_json::json!({ "clear": ["expires_at"] })),
    )
    .await;
    let cleared = json_body(cleared).await;
    assert!(cleared["expires_at"].is_null(), "expiry should be liftable");
    assert_eq!(cleared["description"], "kept", "clearing is per field");
    // An unknown name is refused rather than silently ignored, so a client
    // sending `expiresAt` learns it did nothing.
    let typo = send(
        "PATCH",
        format!("/api/v2/shares/{expiring_id}"),
        owner_token.clone(),
        Some(serde_json::json!({ "clear": ["expiresAt"] })),
    )
    .await;
    assert_eq!(typo.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // A foreign tenant can neither read nor mutate any of it.
    let foreign_playlists = send(
        "GET",
        "/api/v2/playlists".into(),
        intruder_token.clone(),
        None,
    )
    .await;
    assert!(json_body(foreign_playlists)
        .await
        .as_array()
        .unwrap()
        .is_empty());
    for (method, uri) in [
        ("GET", format!("/api/v2/playlists/{playlist_id}")),
        ("DELETE", format!("/api/v2/playlists/{playlist_id}")),
        ("PUT", format!("/api/v2/favorites/track/{first}")),
    ] {
        let response = send(method, uri, intruder_token.clone(), None).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "{method} must not reach another tenant's data"
        );
    }
    let foreign_queue = send("GET", "/api/v2/queue".into(), intruder_token.clone(), None).await;
    assert_eq!(foreign_queue.status(), StatusCode::OK);
    assert!(json_body(foreign_queue).await.is_null());

    // Deleting the playlist makes it unreachable for its owner too.
    let deleted = send(
        "DELETE",
        format!("/api/v2/playlists/{playlist_id}"),
        owner_token.clone(),
        None,
    )
    .await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let gone = send(
        "GET",
        format!("/api/v2/playlists/{playlist_id}"),
        owner_token,
        None,
    )
    .await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}
