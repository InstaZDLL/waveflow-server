//! A canvas over HTTP: placed, read back and taken away, the ticket that opens
//! it, the route's ceiling, the track upsert that carries it, and who sees
//! nothing.

use super::*;

// ---------------------------------------------------------------------------
// The surfaces. RFC-009 decisions 3 and 4.
// ---------------------------------------------------------------------------

fn authorized(request: axum::http::request::Builder, token: &str) -> axum::http::request::Builder {
    request.header("authorization", format!("Bearer {token}"))
}

#[tokio::test]
async fn a_canvas_is_placed_read_back_and_taken_away_over_http() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-routes", 1).await;
    let router = waveflow_server::app(&config, state.clone());
    let bytes = canvas_bytes(temp.path(), "loop.mp4", "1", "black");
    let track = fixture.tracks[0];

    let placed = router
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{track}/canvas")),
                &fixture.token,
            )
            .body(Body::from(bytes.clone()))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(placed.status(), StatusCode::OK);
    let placed = json_body(placed).await;
    assert_eq!(placed["format"], "mp4");
    assert_eq!(placed["byte_size"], bytes.len() as i64);
    let hash = placed["hash"].as_str().unwrap().to_owned();
    assert_eq!(placed["url"], format!("/api/v2/canvas/{hash}"));

    // Addressed by content: this URL can never answer differently, so the
    // client is told to stop asking.
    let by_hash = router
        .clone()
        .oneshot(
            authorized(
                Request::get(format!("/api/v2/canvas/{hash}")),
                &fixture.token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(by_hash.status(), StatusCode::OK);
    assert_eq!(by_hash.headers()["content-type"], "video/mp4");
    assert_eq!(
        by_hash.headers()["cache-control"],
        "private, max-age=31536000, immutable"
    );
    let etag = by_hash.headers()["etag"].to_str().unwrap().to_owned();
    assert_eq!(etag, format!("\"{hash}\""));

    // Addressed by track: it resolves the link of the moment, which a member
    // can replace, so it stays revalidatable.
    let alias = router
        .clone()
        .oneshot(
            authorized(
                Request::get(format!("/api/v2/tracks/{track}/canvas")),
                &fixture.token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(alias.status(), StatusCode::OK);
    assert_eq!(alias.headers()["cache-control"], "private, no-cache");
    // The validator is the hash either way, which is what makes revalidating
    // the alias a 304 rather than a second transfer of the same bytes.
    assert_eq!(alias.headers()["etag"], etag);

    // Revalidation on both shapes. The alias is the one that has to answer 304
    // — it is the URL a client keeps asking about — but the canonical one must
    // too, or a client that does ask pays for the bytes twice.
    for path in [
        format!("/api/v2/tracks/{track}/canvas"),
        format!("/api/v2/canvas/{hash}"),
    ] {
        let revalidated = router
            .clone()
            .oneshot(
                authorized(Request::get(&path), &fixture.token)
                    .header("if-none-match", &etag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revalidated.status(), StatusCode::NOT_MODIFIED, "{path}");
        // The validator comes back with the 304, so the entry stays fresh
        // rather than being evicted for having lost its tag.
        assert_eq!(revalidated.headers()["etag"], etag, "{path}");
    }

    // A <video> element asks for ranges, and it asks the ticket URL. All three
    // ways in serve the same file through the same helper, so all three answer
    // 206 and carry the validator on the partial too.
    let ticket = json_body(
        router
            .clone()
            .oneshot(
                authorized(
                    Request::post(format!("/api/v2/tracks/{track}/canvas-ticket")),
                    &fixture.token,
                )
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await["url"]
        .as_str()
        .unwrap()
        .to_owned();
    for (path, bearer) in [
        (format!("/api/v2/canvas/{hash}"), true),
        (format!("/api/v2/tracks/{track}/canvas"), true),
        // No Authorization header at all: the sealed ticket is the credential.
        (ticket, false),
    ] {
        let request = Request::get(&path);
        let request = if bearer {
            authorized(request, &fixture.token)
        } else {
            request
        };
        let ranged = router
            .clone()
            .oneshot(
                request
                    .header("range", "bytes=0-15")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT, "{path}");
        assert_eq!(ranged.headers()["etag"], etag, "{path}");
        assert_eq!(ranged.headers()["content-length"], "16", "{path}");
    }

    let removed = router
        .clone()
        .oneshot(
            authorized(
                Request::delete(format!("/api/v2/tracks/{track}/canvas")),
                &fixture.token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);

    for gone in [
        format!("/api/v2/tracks/{track}/canvas"),
        format!("/api/v2/canvas/{hash}"),
    ] {
        let response = router
            .clone()
            .oneshot(
                authorized(Request::get(&gone), &fixture.token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{gone}");
    }
}

#[tokio::test]
async fn a_ticket_opens_the_canvas_it_was_minted_for_and_nothing_else() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-ticket", 1).await;
    let router = waveflow_server::app(&config, state.clone());
    let track = fixture.tracks[0];
    state
        .services
        .place_canvas(
            fixture.owner,
            track,
            &canvas_bytes(temp.path(), "loop.mp4", "1", "black"),
            None,
        )
        .await
        .unwrap();

    let minted = router
        .clone()
        .oneshot(
            authorized(
                Request::post(format!("/api/v2/tracks/{track}/canvas-ticket")),
                &fixture.token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(minted.status(), StatusCode::OK);
    let url = json_body(minted).await["url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/api/v2/canvas-stream/"));

    // What <video src> does: no Authorization header at all.
    let played = router
        .clone()
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(played.status(), StatusCode::OK);
    assert_eq!(played.headers()["content-type"], "video/mp4");

    // The two kinds do not answer for each other, which is the whole reason the
    // kind is sealed inside the payload. An audio ticket for this very track,
    // minted for this very account and unexpired, does not open the canvas.
    let audio = waveflow_server::auth::stream_ticket::mint(
        &state.secret_box,
        waveflow_server::auth::stream_ticket::TicketKind::Audio,
        fixture.owner,
        track,
        now_ms() + 60_000,
    )
    .unwrap();
    let refused = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/canvas-stream/{audio}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);

    // And the reverse: the canvas ticket does not open the audio route.
    let canvas_ticket = url.rsplit('/').next().unwrap();
    let refused = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/stream/{canvas_ticket}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_route_carries_its_own_ceiling_and_the_neighbour_sees_nothing() {
    let (temp, config, state) = canvas_app(|canvas| {
        canvas.max_bytes = 4096;
        canvas.library_quota_bytes = 1024 * 1024;
    })
    .await;
    let owner = fixture(&config, &state, "canvas-ceiling-http", 1).await;
    let router = waveflow_server::app(&config, state.clone());
    let track = owner.tracks[0];

    // Over the route's own ceiling. The router's global 16 KiB limit would have
    // let this through, so what refuses it is the limit this route carries.
    let oversized = router
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{track}/canvas")),
                &owner.token,
            )
            .body(Body::from(vec![0u8; 8192]))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Under the ceiling and still not a loop: the request was well formed, and
    // what it carried is not something this server will keep.
    let garbage = router
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{track}/canvas")),
                &owner.token,
            )
            .body(Body::from(vec![7u8; 2048]))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(garbage.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // A real one, so the neighbour has something to fail to see.
    let (_temp2, config2, state2) = canvas_app(|_| {}).await;
    let roomy = fixture(&config2, &state2, "canvas-roomy", 1).await;
    let router2 = waveflow_server::app(&config2, state2.clone());
    let placed = state2
        .services
        .place_canvas(
            roomy.owner,
            roomy.tracks[0],
            &canvas_bytes(temp.path(), "loop.mp4", "1", "black"),
            None,
        )
        .await
        .unwrap();
    let neighbour = fixture(&config2, &state2, "canvas-neighbour-2", 1).await;

    // Knowing the hash establishes nothing, and neither does knowing the track.
    for path in [
        format!("/api/v2/canvas/{}", placed.hash),
        format!("/api/v2/tracks/{}/canvas", roomy.tracks[0]),
    ] {
        let response = router2
            .clone()
            .oneshot(
                authorized(Request::get(&path), &neighbour.token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    // And they cannot put one on somebody else's track either.
    let intruding = router2
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{}/canvas", roomy.tracks[0])),
                &neighbour.token,
            )
            .body(Body::from(vec![7u8; 512]))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(intruding.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn placing_and_removing_a_canvas_travels_in_the_track_upsert() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-feed", 1).await;
    let router = waveflow_server::app(&config, state.clone());
    let (token, device) =
        login_session(&router, "canvas-feed", "correct horse battery staple").await;
    let track = fixture.tracks[0];
    let library = fixture.library;

    // Where the feed already stood, so what follows is what this test caused
    // rather than what the scan left behind.
    let before = library_events(&router, &token, library).await.len();

    let placed = router
        .clone()
        .oneshot(
            authorized(
                Request::put(format!("/api/v2/tracks/{track}/canvas")),
                &token,
            )
            .header("x-waveflow-device-id", &device)
            .body(Body::from(canvas_bytes(
                temp.path(),
                "loop.mp4",
                "1",
                "black",
            )))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(placed.status(), StatusCode::OK);

    let events = library_events(&router, &token, library).await;
    assert_eq!(events.len(), before + 1, "one event, and only one");
    let event = events.last().unwrap();
    // RFC-009 decision 7: no event for the blob, because the bytes behind a
    // hash never change. Only the link is a change, and the link is part of the
    // track — so it travels in the track's own upsert.
    assert_eq!(event["entity_type"], "track");
    assert_eq!(event["action"], "upsert");
    assert_eq!(event["entity_id"], track.to_string());
    // And the payload does not grow. `full_hash` is there because nothing else
    // carries it; a canvas link is read off the track the client refetches.
    assert!(
        event["payload"]["full_hash"].is_string(),
        "the payload is the one a tag correction sends"
    );
    assert!(
        event["payload"].get("canvas").is_none(),
        "no canvas field: that would make the payload a partial projection"
    );
    // So the client that just placed it does not read it back as a discovery.
    assert_eq!(event["origin_device_id"], device);

    // Taking it away is a change too, and it is the same shape.
    let removed = router
        .clone()
        .oneshot(
            authorized(
                Request::delete(format!("/api/v2/tracks/{track}/canvas")),
                &token,
            )
            .header("x-waveflow-device-id", &device)
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::NO_CONTENT);
    let events = library_events(&router, &token, library).await;
    assert_eq!(events.len(), before + 2);
    assert_eq!(events.last().unwrap()["action"], "upsert");

    // A removal that found nothing changed nothing, and says nothing.
    let again = router
        .clone()
        .oneshot(
            authorized(
                Request::delete(format!("/api/v2/tracks/{track}/canvas")),
                &token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        library_events(&router, &token, library).await.len(),
        before + 2,
        "nothing happened, so nothing was announced"
    );
}

async fn library_events(
    router: &axum::Router,
    token: &str,
    library: uuid::Uuid,
) -> Vec<serde_json::Value> {
    let feed = router
        .clone()
        .oneshot(
            authorized(
                Request::get(format!(
                    "/api/v2/libraries/{library}/events?after=0&limit=500"
                )),
                token,
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(feed.status(), StatusCode::OK);
    json_body(feed).await["events"]
        .as_array()
        .expect("the feed answers a list")
        .clone()
}

#[tokio::test]
async fn the_two_doors_are_independent_in_both_directions() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-doors", 1).await;
    let bytes = canvas_bytes(temp.path(), "loop.mp4", "1", "black");
    let track = fixture.tracks[0];

    // The case that made this a separate flag. `fixture` opened the canvas door
    // and left the upload one shut, which is the read-only server the desktop
    // named: it refuses to grow in audio and still takes a loop.
    // Stated rather than inherited from the fixture, so the situation this test
    // is about is visible in the test.
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();
    state
        .services
        .place_canvas(fixture.owner, track, &bytes, None)
        .await
        .unwrap();

    // And the reverse: opening the library to files says nothing about loops.
    // A flag inherited from the other would be the operator opting into
    // something he never named.
    state
        .services
        .remove_canvas(fixture.owner, track, None)
        .await
        .unwrap();
    state
        .db
        .set_library_accepts_canvas(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    assert!(matches!(
        state
            .services
            .place_canvas(fixture.owner, track, &bytes, None)
            .await,
        Err(waveflow_server::services::ServiceError::Forbidden)
    ));
}

/// Whether a library takes canvases is on the list of libraries, apart from
/// whether it takes files, and it follows the operator's switch.
///
/// Two doors, and the list has to show both. A client that could read only
/// `accepts_uploads` would hide the canvas in exactly the library RFC-009 split
/// the flag for — one that refuses audio and takes loops — and a client that
/// could read neither would offer a canvas everywhere, to learn the answer only
/// after sending the whole file.
#[tokio::test]
async fn the_library_list_says_whether_a_library_takes_canvases() {
    let (_temp, config, state) = canvas_app(|_| {}).await;
    let fixture = fixture(&config, &state, "canvas-flag-reader", 1).await;
    let listed = || {
        let router = waveflow_server::app(&config, state.clone());
        let token = fixture.token.clone();
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .uri("/api/v2/libraries")
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            json_body(response).await
        }
    };

    let open = listed().await;
    assert_eq!(open[0]["accepts_canvas"], true, "the fixture opened it");
    assert_eq!(
        open[0]["accepts_uploads"], false,
        "and left the other door shut, which the list must not confuse with it"
    );
    state
        .db
        .set_library_accepts_canvas(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();
    assert_eq!(listed().await[0]["accepts_canvas"], false);
}

/// RFC-023 decision 4: a canvas ticket minted before its account was disabled
/// stops opening the canvas, as an audio ticket stops playing. The ticket is
/// sealed rather than looked up, so the redemption's own read is what asks.
#[tokio::test]
async fn a_disabled_account_s_canvas_ticket_opens_nothing() {
    let (temp, config, state) = canvas_app(|_| {}).await;
    let (owner, _token) = admin_with_api_token(&state, "canvas-disabled").await;
    let music = config.data_dir.join("canvas-disabled");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Looped.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "canvas-disabled",
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
            id: library,
            name: "canvas-disabled".into(),
            root_path: root,
        },
    )
    .await;
    state
        .db
        .set_library_accepts_canvas(owner, library, true, now_ms())
        .await
        .unwrap();
    let track = state.db.list_tracks_for_user(owner, library).await.unwrap()[0].id;
    state
        .services
        .place_canvas(
            owner,
            track,
            &canvas_bytes(temp.path(), "loop.mp4", "1", "black"),
            None,
        )
        .await
        .unwrap();
    let ticket = waveflow_server::auth::stream_ticket::mint(
        &state.secret_box,
        waveflow_server::auth::stream_ticket::TicketKind::Canvas,
        owner,
        track,
        now_ms() + 60_000,
    )
    .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let open = || {
        let router = router.clone();
        let ticket = ticket.clone();
        async move {
            router
                .oneshot(
                    Request::get(format!("/api/v2/canvas-stream/{ticket}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(open().await, StatusCode::OK);
    set_disabled(&state, owner, true).await;
    assert_eq!(open().await, StatusCode::NOT_FOUND);
    set_disabled(&state, owner, false).await;
    assert_eq!(open().await, StatusCode::OK);
}
