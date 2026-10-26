//! API tokens, the bookmarks they reach, and what a narrowed token is refused.

use super::*;

/// Bookmarks and API tokens were reachable from one surface each: bookmarks
/// only from Subsonic, tokens only from a shell on the host.
#[tokio::test]
async fn native_bookmarks_and_api_tokens_round_trip() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let admin = state
        .db
        .create_account("token-admin", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("token-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Tokens",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    let mut input = browse_input(600, "Long Read", "Chapters", "Narrator", Some(1), Some(1));
    input.relative_path = "token-0.flac".into();
    input.quick_hash = format!("{:064x}", 61_000);
    input.full_hash = format!("{:064x}", 62_000);
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "token-admin",
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
    let track = state
        .services
        .catalog_snapshot(admin, &[])
        .await
        .unwrap()
        .songs[0]
        .id;

    let json_request = |method: Method, path: String, body: serde_json::Value| {
        let router = router.clone();
        let access = access.clone();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("authorization", format!("Bearer {access}"))
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let get = |path: String| {
        let router = router.clone();
        let access = access.clone();
        async move {
            router
                .oneshot(
                    Request::get(path)
                        .header("authorization", format!("Bearer {access}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    // Setting a bookmark twice moves it rather than adding a second, and the
    // comment is replaced rather than patched.
    let response = json_request(
        Method::PUT,
        format!("/api/v2/bookmarks/{track}"),
        serde_json::json!({"position_ms": 90_000, "comment": "chapter two"}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let response = json_request(
        Method::PUT,
        format!("/api/v2/bookmarks/{track}"),
        serde_json::json!({"position_ms": 180_000}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let listed = json_body(get("/api/v2/bookmarks".into()).await).await;
    assert_eq!(listed.as_array().expect("a list").len(), 1);
    assert_eq!(listed[0]["position_ms"], 180_000);
    assert!(listed[0]["comment"].is_null(), "the comment was replaced");

    // A negative position is not a position.
    let response = json_request(
        Method::PUT,
        format!("/api/v2/bookmarks/{track}"),
        serde_json::json!({"position_ms": -1}),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // The facade sees the same bookmark: one domain method, two surfaces.
    assert_eq!(
        state.services.bookmarks(admin).await.unwrap()[0].position_ms,
        180_000
    );

    // Deleting is idempotent, for the same reason it is on the facade.
    for _ in 0..2 {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/v2/bookmarks/{track}"))
                    .header("authorization", format!("Bearer {access}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }
    assert!(state.services.bookmarks(admin).await.unwrap().is_empty());

    // An API token can now be issued without a shell on the host. The secret
    // appears once and is never listed.
    let created = json_request(
        Method::POST,
        "/api/v2/admin/users/token-admin/tokens".into(),
        serde_json::json!({"name": "backup script", "scopes": ["catalog:read"]}),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = json_body(created).await;
    let secret = created["secret"].as_str().expect("the secret is returned");
    assert!(secret.starts_with("wfapi_"));
    let token_id = created["id"].as_str().expect("the record carries its id");
    assert_eq!(created["scopes"][0], "catalog:read");

    let listed = json_body(get("/api/v2/admin/users/token-admin/tokens".into()).await).await;
    assert_eq!(listed.as_array().expect("a list").len(), 1);
    assert_eq!(listed[0]["name"], "backup script");
    assert!(
        listed[0].get("secret").is_none() && listed[0].get("token_hash").is_none(),
        "a listing must not carry the secret: {listed}"
    );
    assert!(listed[0]["revoked_at"].is_null());

    // The token authenticates, and stops doing so once revoked.
    let with_token = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(with_token.status(), StatusCode::OK);

    // Scopes are enforced, not decorated. The token names `catalog:read`, so it
    // reads the catalogue and nothing else, even though the account behind it
    // is an administrator. Storing a scope list, returning it from the API and
    // printing it from the CLI while ignoring it is worse than having none: the
    // operator believes the token is limited.
    let admin_route = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admin_route.status(), StatusCode::FORBIDDEN);
    // The session it was issued from still reaches that route, so the refusal
    // belongs to the token and not to the account.
    assert_eq!(
        get("/api/v2/admin/users".into()).await.status(),
        StatusCode::OK
    );

    // Reading is all a `catalog:read` token may do. Before the scope check
    // reached the mutations it could still write playlists, shares, ratings,
    // the queue and these very bookmarks: only the administrative door was
    // closed, which shut the worst case and left the principle open.
    let write_attempt = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/api/v2/bookmarks/{track}"))
                .header("authorization", format!("Bearer {secret}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"position_ms": 1}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write_attempt.status(), StatusCode::FORBIDDEN);
    // And it still reads, so the refusal is the mutation and not the token.
    let read_attempt = router
        .clone()
        .oneshot(
            Request::get("/api/v2/bookmarks")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_attempt.status(), StatusCode::OK);

    // The media routes go through the same door now, and reading needs no
    // scope, so a read-only token still plays. Requiring a literal `read`
    // scope would strand every token an operator has already issued.
    let ticket = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v2/tracks/{track}/stream-ticket"))
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ticket.status(), StatusCode::OK);
    // An unauthenticated one is still refused, so the door did not open.
    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/api/v2/tracks/{track}/stream-ticket"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    // `write` admits the mutation and stops at the instance: the two levels
    // are separate, and `admin` implies `write` rather than the reverse.
    let writer = json_body(
        json_request(
            Method::POST,
            "/api/v2/admin/users/token-admin/tokens".into(),
            serde_json::json!({"name": "sync agent", "scopes": ["write"]}),
        )
        .await,
    )
    .await;
    let writer = writer["secret"].as_str().unwrap().to_owned();
    let allowed = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/api/v2/bookmarks/{track}"))
                .header("authorization", format!("Bearer {writer}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"position_ms": 1}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::NO_CONTENT);
    let refused = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {writer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    state.services.delete_bookmark(admin, track).await.unwrap();

    // A scope list grants the union of its entries: naming `admin` beside
    // another scope admits these routes, because a token that explicitly
    // carries a permission must not be refused it. The stored form is
    // normalised, so what the listing shows is what authorization compares.
    let combined = json_body(
        json_request(
            Method::POST,
            "/api/v2/admin/users/token-admin/tokens".into(),
            serde_json::json!({"name": "ops", "scopes": ["  admin  ", "catalog:read"]}),
        )
        .await,
    )
    .await;
    assert_eq!(combined["scopes"][0], "admin", "scopes are stored trimmed");
    let combined = combined["secret"].as_str().unwrap().to_owned();
    let admitted = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {combined}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(admitted.status(), StatusCode::OK);

    // A token issued without scopes is unrestricted, which is what the CLI has
    // always produced and what existing tokens carry.
    let unscoped = json_body(
        json_request(
            Method::POST,
            "/api/v2/admin/users/token-admin/tokens".into(),
            serde_json::json!({"name": "full access"}),
        )
        .await,
    )
    .await;
    let unscoped = unscoped["secret"].as_str().unwrap().to_owned();
    let allowed = router
        .clone()
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {unscoped}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(allowed.status(), StatusCode::OK);

    let revoked = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/v2/admin/users/token-admin/tokens/{token_id}"))
                .header("authorization", format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::NO_CONTENT);
    let after = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {secret}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(after.status(), StatusCode::UNAUTHORIZED);

    // Revoking it again is not found: it is already not working.
    let again = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::DELETE)
                .uri(format!("/api/v2/admin/users/token-admin/tokens/{token_id}"))
                .header("authorization", format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);

    // Only an administrator mints one.
    let listener = state
        .db
        .create_account("token-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    assert!(matches!(
        state
            .services
            .create_api_token(listener, &[], "token-admin", "stolen", &[])
            .await,
        Err(ServiceError::Forbidden)
    ));
}

/// Every refusal a narrowed token actually meets is one the document declares.
///
/// The scope check has been enforced since the tokens landed, and nineteen
/// mutations answered 403 without saying so anywhere a client generator could
/// see. `annotate_scope_refusals` closes that by construction for mutations,
/// which is worth exactly nothing on its own: a pass that injects a response
/// and a test that reads the same document back agree with each other whether
/// or not either matches the server.
///
/// So this asks the server. It mints a `catalog:read` token, walks every
/// secured operation the document publishes, and requires that anything
/// answering 403 also declares 403. The oracle is the running router; the
/// document is what is on trial.
///
/// That is also what makes it catch the case the injection cannot see.
/// `GET /api/v2/uploads/{session_id}` asks for `Access::Write` — reading a
/// transfer's state belongs to the write flow — so it refuses a narrow token
/// while being a read, and a pass that annotates mutations walks straight past
/// it. No list here knows that. The sweep found it by being told 403.
#[tokio::test]
async fn every_refusal_a_narrow_token_meets_is_documented() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    state
        .db
        .create_account("sweep-admin", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "sweep-admin",
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

    // An administrator's token, narrowed to reading. The account behind it may
    // do everything; the credential may not, which is the whole point.
    let minted = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v2/admin/users/sweep-admin/tokens")
                .header("authorization", format!("Bearer {access}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"name": "sweep", "scopes": ["catalog:read"]}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(minted.status(), StatusCode::CREATED);
    let narrow = json_body(minted).await["secret"]
        .as_str()
        .unwrap()
        .to_owned();

    let document = json_body(
        router
            .clone()
            .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
            .await
            .unwrap(),
    )
    .await;

    // Enough of a value that the extractor in front of the handler accepts it:
    // a rejected path parameter never reaches `authenticated`, and the
    // implication below simply does not fire for that route.
    let canvas_hash = "a".repeat(64);
    let fill = |path: &str| {
        let mut filled = String::new();
        let mut rest = path;
        while let Some(open) = rest.find('{') {
            let close = rest.find('}').expect("a balanced parameter");
            filled.push_str(&rest[..open]);
            filled.push_str(match &rest[open + 1..close] {
                "entity_type" => "track",
                "username" => "sweep-admin",
                "index" => "0",
                "canvas_hash" => canvas_hash.as_str(),
                _ => "00000000-0000-4000-8000-000000000000",
            });
            rest = &rest[close + 1..];
        }
        filled.push_str(rest);
        filled
    };

    let mut refused = Vec::new();
    let mut undocumented = Vec::new();
    let paths = document["paths"].as_object().expect("a path map");
    for (path, item) in paths {
        if !path.starts_with("/api/v2/") {
            continue;
        }
        for method in ["get", "put", "post", "delete", "patch"] {
            let Some(operation) = item.get(method) else {
                continue;
            };
            // A route that carries no credential has none to have refused.
            if operation["security"].as_array().is_some_and(Vec::is_empty) {
                continue;
            }
            let request = Request::builder()
                .method(Method::from_bytes(method.to_uppercase().as_bytes()).unwrap())
                .uri(fill(path))
                .header("authorization", format!("Bearer {narrow}"))
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap();
            let status = router.clone().oneshot(request).await.unwrap().status();
            if status != StatusCode::FORBIDDEN {
                continue;
            }
            refused.push(format!("{} {path}", method.to_uppercase()));
            if operation["responses"]["403"].is_null() {
                undocumented.push(format!("{} {path}", method.to_uppercase()));
            }
        }
    }

    assert!(
        undocumented.is_empty(),
        "these refuse a narrowed token without documenting 403: {undocumented:#?}"
    );

    // An implication passes whenever nothing satisfies it, so the sweep has to
    // show it reached the routes it claims to judge. Both of these fail if the
    // requests stop arriving — a changed fixture, an extractor rejecting every
    // filled path — rather than letting the test go quietly vacuous.
    //
    // Twenty and not the forty-odd secured operations, because axum runs the
    // extractors before the handler body: a route taking `Json<T>` with a
    // required field answers 422 to this sweep's `{}` and never reaches
    // `authenticated`. Sending each route a body it would accept would mean
    // keeping a hand-written fixture per route here — which is the list this
    // test exists to not have. So it judges what it can reach, and the number
    // records how much that is.
    assert!(
        refused.len() >= 20,
        "the sweep reached too few refusals to mean anything: {refused:#?}"
    );
    assert!(
        refused
            .iter()
            .any(|route| route == "GET /api/v2/uploads/{session_id}"),
        "the read that asks for write authority is what this sweep is for: {refused:#?}"
    );
}
