//! Authorization Code with PKCE for native clients.

use super::*;

#[tokio::test]
async fn pkce_authorization_grants_a_native_session_exactly_once() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let user = state
        .db
        .create_account("pkce-user", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    // One indexed track, so the scoped token can be shown writing rather
    // than merely not being refused.
    let music = config.data_dir.join("pkce-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            user,
            "Pkce",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(user), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    let mut input = browse_input(700, "Paired", "Handshake", "Loopback", Some(1), Some(1));
    input.relative_path = "pkce-0.flac".into();
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
        .catalog_snapshot(user, &[])
        .await
        .unwrap()
        .songs[0]
        .id;
    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "pkce-user", password).await;

    let verifier = "H1r8mQ2xY7pL4vC0nB6zK9tW3sD5gJ8fA2eR7uI1oP4";
    let challenge = waveflow_server::auth::oauth::challenge_for(verifier);
    let redirect_uri = "http://127.0.0.1:49152/callback";

    let authorize = |body: serde_json::Value, bearer: Option<String>| {
        let router = router.clone();
        async move {
            let mut request =
                Request::post("/api/v2/oauth/authorize").header("content-type", "application/json");
            if let Some(bearer) = bearer {
                request = request.header("authorization", format!("Bearer {bearer}"));
            }
            router
                .oneshot(request.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap()
        }
    };
    let exchange = |body: serde_json::Value| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::post("/api/v2/oauth/token")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };
    let grant = serde_json::json!({
        "client_id": "com.waveflow.desktop",
        "redirect_uri": redirect_uri,
        "code_challenge": challenge,
        "code_challenge_method": "S256",
        "state": "opaque-state",
        "device_name": "WaveFlow Desktop"
    });

    // Granting requires the browser session: the consent screen is authenticated.
    assert_eq!(
        authorize(grant.clone(), None).await.status(),
        StatusCode::UNAUTHORIZED
    );

    // A narrowed credential may mint one, and what it mints is narrowed the
    // same way: the grant records the caller's scopes and the redeemed session
    // is issued under them. `write` in, `write` out.
    //
    // Before the scopes travelled, the session came back carrying the
    // account's whole authority whatever asked for it, so a token deliberately
    // issued without `admin` reached `Admin` in two requests.
    let scoped = json_body(
        router
            .clone()
            .oneshot(
                Request::post("/api/v2/admin/users/pkce-user/tokens")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"name": "agent", "scopes": ["write"]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let scoped = scoped["secret"].as_str().expect("the secret").to_owned();
    let mut narrowed = grant.clone();
    narrowed["device_name"] = "Scoped Agent".into();
    let granted_narrow = authorize(narrowed, Some(scoped.clone())).await;
    assert_eq!(granted_narrow.status(), StatusCode::OK);
    let narrow_code = code_from(
        json_body(granted_narrow).await["redirect_to"]
            .as_str()
            .unwrap(),
    );
    let narrow_session = json_body(
        exchange(serde_json::json!({
            "code": narrow_code,
            "code_verifier": verifier,
            "client_id": "com.waveflow.desktop",
            "redirect_uri": redirect_uri
        }))
        .await,
    )
    .await;
    let narrow_access = narrow_session["access_token"].as_str().unwrap().to_owned();
    let narrow_refresh = narrow_session["refresh_token"].as_str().unwrap().to_owned();
    // The client is told what it holds, rather than having to discover the
    // limit by being refused.
    assert_eq!(
        narrow_session["user"]["scopes"],
        serde_json::json!(["write"])
    );

    // `pkce-user` is an administrator — it minted the token above — so the
    // only thing that can refuse an admin route to this session is the scope
    // list it inherited from the token that authorized it. Drop the carrying
    // and this answers 200.
    let admin_route = |bearer: String| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::get("/api/v2/admin/users")
                        .header("authorization", format!("Bearer {bearer}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(
        admin_route(narrow_access.clone()).await,
        StatusCode::FORBIDDEN,
        "a session redeemed from a `write` grant must not answer to `admin`"
    );
    // It is the narrowing that refuses and not the account: the same account's
    // password session still reaches the same route.
    assert_eq!(admin_route(token.clone()).await, StatusCode::OK);

    // And it is a working session, not a broken one: what `write` names, it does.
    let writes = router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(format!("/api/v2/ratings/track/{track}"))
                .header("authorization", format!("Bearer {narrow_access}"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({"rating": 3}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(writes.status(), StatusCode::NO_CONTENT);
    // And the write landed: a route that had quietly become a no-op would
    // still answer without refusing.
    let ratings = state.services.ratings(user).await.unwrap();
    assert_eq!(ratings.len(), 1);
    assert_eq!(ratings[0].entity_id, track);
    assert_eq!(ratings[0].rating, 3);

    // Rotation must not widen either: the refreshed session answers to exactly
    // the scopes the original was issued under.
    let rotated = json_body(
        router
            .clone()
            .oneshot(json_request(
                "/api/v2/auth/refresh",
                serde_json::json!({"refresh_token": narrow_refresh}),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(rotated["user"]["scopes"], serde_json::json!(["write"]));
    let rotated = rotated["access_token"].as_str().unwrap().to_owned();
    assert_eq!(
        admin_route(rotated).await,
        StatusCode::FORBIDDEN,
        "refreshing a narrowed session must not hand back a wide one"
    );

    // A redirect that could carry the code off the machine is refused.
    let mut remote = grant.clone();
    remote["redirect_uri"] = "http://evil.example.com/cb".into();
    assert_eq!(
        authorize(remote, Some(token.clone())).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // "plain" would defeat the point of PKCE.
    let mut plain = grant.clone();
    plain["code_challenge_method"] = "plain".into();
    assert_eq!(
        authorize(plain, Some(token.clone())).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let granted = authorize(grant.clone(), Some(token.clone())).await;
    assert_eq!(granted.status(), StatusCode::OK);
    let redirect_to = json_body(granted).await["redirect_to"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(redirect_to.starts_with(redirect_uri));
    assert!(redirect_to.contains("state=opaque-state"));
    let code = code_from(&redirect_to);

    // Without the verifier the code is useless, which is the whole point. The
    // attempt also burns the code: presenting one at all spends it, so a
    // verifier cannot be guessed across retries.
    let wrong = exchange(serde_json::json!({
        "code": code,
        "code_verifier": "Z9y8X7w6V5u4T3s2R1q0P9o8N7m6L5k4J3i2H1g0F9e",
        "client_id": "com.waveflow.desktop",
        "redirect_uri": redirect_uri
    }))
    .await;
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    let after_failure = exchange(serde_json::json!({
        "code": code,
        "code_verifier": verifier,
        "client_id": "com.waveflow.desktop",
        "redirect_uri": redirect_uri
    }))
    .await;
    assert_eq!(
        after_failure.status(),
        StatusCode::UNAUTHORIZED,
        "a failed exchange spends the code; the client restarts the flow"
    );

    // A fresh grant completes normally.
    let granted = authorize(grant.clone(), Some(token.clone())).await;
    assert_eq!(granted.status(), StatusCode::OK);
    let redirect_to = json_body(granted).await["redirect_to"]
        .as_str()
        .unwrap()
        .to_owned();
    let code = code_from(&redirect_to);

    let exchanged = exchange(serde_json::json!({
        "code": code,
        "code_verifier": verifier,
        "client_id": "com.waveflow.desktop",
        "redirect_uri": redirect_uri
    }))
    .await;
    assert_eq!(exchanged.status(), StatusCode::OK);
    let tokens = json_body(exchanged).await;
    let access = tokens["access_token"].as_str().unwrap().to_owned();
    assert_eq!(tokens["user"]["username"], "pkce-user");

    // The issued session is a real one.
    let albums = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {access}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(albums.status(), StatusCode::OK);

    // A replayed code must not yield a second session.
    let replay = exchange(serde_json::json!({
        "code": code,
        "code_verifier": verifier,
        "client_id": "com.waveflow.desktop",
        "redirect_uri": redirect_uri
    }))
    .await;
    assert_eq!(
        replay.status(),
        StatusCode::UNAUTHORIZED,
        "an authorization code is single use"
    );
}
