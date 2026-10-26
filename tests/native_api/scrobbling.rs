//! Scrobble links and the uncertain-entry queue, over `/api/v2`.

use super::*;

/// A destination that never says whether it recorded anything.
///
/// The whole of decision 5 in three lines: the queue cannot know, so a person
/// has to. Local to this target because the doubles in `tests/scrobbling.rs`
/// belong to a file whose accounts are inserted with a placeholder hash and
/// never log in — which is precisely what a route test needs them to do.
struct AlwaysAmbiguous;

impl ScrobbleTarget for AlwaysAmbiguous {
    fn submit<'a>(
        &'a self,
        _envelope: &'a ScrobbleEnvelope,
        _secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async { ScrobbleVerdict::Ambiguous })
    }
}

/// An account poses and withdraws its own authorisation, and sees only its own.
///
/// RFC-010 decision 9: a destination's base URL belongs to the deployment, an
/// authorisation belongs to a person. So these routes are self-scoped, and the
/// thing a route can get wrong that a service method cannot is *whose* id it
/// passes — which is why a second account is here rather than only the first.
#[tokio::test]
async fn an_account_poses_and_withdraws_its_own_scrobble_link() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    for name in ["scrobble-owner", "scrobble-stranger"] {
        state
            .db
            .create_account(name, &hash, AccountRole::User, now_ms())
            .await
            .unwrap();
    }
    let router = waveflow_server::app(&config, state.clone());

    let bearer = |username: &'static str| {
        let router = router.clone();
        async move {
            let login = router
                .oneshot(json_request(
                    "/api/v2/auth/login",
                    serde_json::json!({
                        "username": username,
                        "password": "correct horse battery staple",
                        "device_name": "Integration"
                    }),
                ))
                .await
                .unwrap();
            json_body(login).await["access_token"]
                .as_str()
                .unwrap()
                .to_owned()
        }
    };
    let owner = bearer("scrobble-owner").await;
    let stranger = bearer("scrobble-stranger").await;

    let send = |method: Method, path: String, access: String, body: Option<serde_json::Value>| {
        let router = router.clone();
        async move {
            let mut request = Request::builder().method(method).uri(path);
            if !access.is_empty() {
                request = request.header("authorization", format!("Bearer {access}"));
            }
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

    // Nothing linked yet, and asking is not an error.
    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        owner.clone(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);

    let response = send(
        Method::PUT,
        "/api/v2/scrobble-links/listenbrainz/default".into(),
        owner.clone(),
        Some(serde_json::json!({"secret": "lb-token"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        owner.clone(),
        None,
    )
    .await;
    let links = json_body(response).await;
    assert_eq!(links.as_array().unwrap().len(), 1);
    // The spelling a client reads back is the one it may send in. Four of them
    // are held together by a unit test; this is the fifth reader.
    assert_eq!(links[0]["provider"], "listenbrainz");
    assert_eq!(links[0]["health"], "healthy");
    assert_eq!(links[0]["pending"], 0);
    // Counters, never content, and never the secret: decision 12.
    assert!(links[0].get("secret").is_none());

    // The stranger's own list stays empty, which is the half a single-account
    // test cannot distinguish from a route that ignores the caller entirely.
    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        stranger.clone(),
        None,
    )
    .await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);

    // A destination this server cannot name is refused before anything is
    // sealed, and refused as a bad request rather than a missing one.
    let response = send(
        Method::PUT,
        "/api/v2/scrobble-links/spotify/default".into(),
        owner.clone(),
        Some(serde_json::json!({"secret": "whatever"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // And an instance nobody declared is a different refusal, on a recipient
    // this server does know. A member picks among the names the server
    // publishes; a path naming one it never published is a resource that is not
    // there, not a request it cannot parse. Without this case the 422 above
    // would be the only refusal covered, and the two arrive from different
    // places for different reasons.
    let response = send(
        Method::PUT,
        "/api/v2/scrobble-links/listenbrainz/an-instance-nobody-declared".into(),
        owner.clone(),
        Some(serde_json::json!({"secret": "lb-token"})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // A secret that is nothing but spaces is refused for a destination this
    // server *does* know, which is the half the case above cannot reach: there
    // the provider parse answers first, and the secret is never looked at. The
    // spaces matter too — an empty string would pass a check written before the
    // trim rather than after it, and this one would not.
    let response = send(
        Method::PUT,
        "/api/v2/scrobble-links/listenbrainz/default".into(),
        owner.clone(),
        Some(serde_json::json!({"secret": "   "})),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // And the authorisation posed earlier is untouched: a refusal replaces
    // nothing.
    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        owner.clone(),
        None,
    )
    .await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 1);

    // And no credential route is reachable without one.
    let response = send(
        Method::GET,
        "/api/v2/scrobble-links".into(),
        String::new(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let response = send(
        Method::DELETE,
        "/api/v2/scrobble-links/listenbrainz/default".into(),
        owner.clone(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = send(Method::GET, "/api/v2/scrobble-links".into(), owner, None).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);
}

/// The two gestures decision 13 grants, over HTTP.
///
/// The domain half is pinned in `tests/scrobbling.rs`. What only this can show
/// is that the routes exist, name an entry the way the outside names it, and
/// answer a stranger's id with a 404 rather than a decision.
#[tokio::test]
async fn an_uncertain_entry_is_listed_and_answered_over_http() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("uncertain-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("uncertain-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Uncertain",
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
    let mut input = browse_input(240, "Ambiguous", "Unknowns", "Nobody", Some(1), Some(1));
    input.relative_path = "uncertain-0.flac".into();
    input.quick_hash = format!("{:064x}", 71_000);
    input.full_hash = format!("{:064x}", 72_000);
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let track = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs[0]
        .id;

    state
        .services
        .link_scrobble(owner, ScrobbleProvider::ListenBrainz, "default", "lb-token")
        .await
        .unwrap();
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::new(AlwaysAmbiguous) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(owner, track, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "uncertain-owner",
                "password": "correct horse battery staple",
                "device_name": "Integration"
            }),
        ))
        .await
        .unwrap();
    let access = json_body(login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let send = |method: Method, path: String| {
        let router = router.clone();
        let access = access.clone();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {access}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(response.status(), StatusCode::OK);
    let waiting = json_body(response).await;
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    let entry = waiting[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(waiting[0]["provider"], "listenbrainz");
    // The envelope stays where it is: a state, never an echo of what was heard.
    assert!(waiting[0].get("title").is_none());

    // **Somebody else's id, not nobody's.** This assertion used to send a fresh
    // `Uuid::new_v4()`, which belongs to no account at all — a handler that had
    // dropped the tenancy binding entirely and looked the entry up by
    // `public_id` alone would have answered 404 to that and passed. The thing a
    // route can get wrong that a service method cannot is *whose* id it passes,
    // so the id below is the owner's real entry, presented by a stranger.
    state
        .db
        .create_account("uncertain-stranger", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let stranger_login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "uncertain-stranger",
                "password": "correct horse battery staple",
                "device_name": "Integration"
            }),
        ))
        .await
        .unwrap();
    let stranger_access = json_body(stranger_login).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let send_as_stranger = |method: Method, path: String| {
        let router = router.clone();
        let access = stranger_access.clone();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {access}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    // Their own queue is empty, and the owner's entry is not theirs to answer.
    let response = send_as_stranger(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);
    let response = send_as_stranger(
        Method::POST,
        format!("/api/v2/scrobble-queue/uncertain/{entry}/retry"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let response = send_as_stranger(
        Method::DELETE,
        format!("/api/v2/scrobble-queue/uncertain/{entry}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    // And it is still there afterwards: refused, not quietly consumed.
    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 1);

    let response = send(
        Method::POST,
        format!("/api/v2/scrobble-queue/uncertain/{entry}/retry"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let retried = json_body(response).await;
    assert_ne!(
        retried["id"].as_str().unwrap(),
        entry,
        "a new entry, not the old one"
    );

    // Answered, so it stops asking.
    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);

    // The copy the retry queued comes back ambiguous in its turn — the double
    // above answers that way every time — and *that* is the entry this discards.
    // Until here the delete route was only ever measured refusing a stranger,
    // which a handler that refused everybody would satisfy just as well.
    state.services.drain_scrobble_outbox().await.unwrap();
    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    let waiting = json_body(response).await;
    assert_eq!(waiting.as_array().unwrap().len(), 1);
    let second = waiting[0]["id"].as_str().unwrap().to_owned();
    assert_ne!(second, entry, "the copy, not the listen it was copied from");

    let response = send(
        Method::DELETE,
        format!("/api/v2/scrobble-queue/uncertain/{second}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    // Thrown away rather than hidden from one listing: it is gone from the
    // queue, and discarding it again finds nothing to discard.
    let response = send(Method::GET, "/api/v2/scrobble-queue/uncertain".into()).await;
    assert_eq!(json_body(response).await.as_array().unwrap().len(), 0);
    let response = send(
        Method::DELETE,
        format!("/api/v2/scrobble-queue/uncertain/{second}"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
