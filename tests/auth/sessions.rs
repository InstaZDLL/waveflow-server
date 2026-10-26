//! Bootstrap, sign-in, refresh and logout, for native clients and the browser,
//! and what a disabled account loses.

use super::*;

#[tokio::test]
async fn fresh_instance_bootstraps_accounts_library_and_encrypted_credential() {
    let (_temp, config, state) = test_app().await;
    let now = now_ms();
    let password_hash = security::hash_password("correct horse battery staple").unwrap();
    let admin_id = state
        .db
        .create_account("admin", &password_hash, AccountRole::Admin, now)
        .await
        .unwrap();

    let music = config.data_dir.join("music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            admin_id,
            "Main library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now,
        )
        .await
        .unwrap();

    let encrypted = state
        .secret_box
        .encrypt(b"dedicated-subsonic-secret")
        .unwrap();
    let api_key_hash = security::token_hash("wfsk_test-key");
    state
        .db
        .set_subsonic_credential(admin_id, admin_id, &encrypted, &api_key_hash, now)
        .await
        .unwrap();

    let member_role: String =
        sqlx::query_scalar("SELECT role FROM library_member WHERE library_id = ? AND user_id = ?")
            .bind(library_id.to_string())
            .bind(admin_id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(member_role, "owner");
    let listener_id = state
        .db
        .create_account("member", &password_hash, AccountRole::User, now)
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            admin_id,
            library_id,
            listener_id,
            LibraryRole::Listener,
            now,
        )
        .await
        .unwrap();
    assert!(state
        .db
        .remove_library_member(admin_id, library_id, listener_id, now)
        .await
        .unwrap());
    assert!(!state
        .db
        .remove_library_member(admin_id, library_id, admin_id, now)
        .await
        .unwrap());

    let row = sqlx::query(
        "SELECT password_nonce, password_ciphertext, api_key_hash \
         FROM subsonic_credential WHERE user_id = ?",
    )
    .bind(admin_id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    let nonce: Vec<u8> = row.get("password_nonce");
    let ciphertext: Vec<u8> = row.get("password_ciphertext");
    assert_eq!(row.get::<Vec<u8>, _>("api_key_hash").len(), 32);
    assert_eq!(
        state.secret_box.decrypt(&nonce, &ciphertext).unwrap(),
        b"dedicated-subsonic-secret"
    );
    assert!(!ciphertext
        .windows(b"dedicated-subsonic-secret".len())
        .any(|window| window == b"dedicated-subsonic-secret"));
    assert_eq!(std::fs::read(&config.instance_key_path).unwrap().len(), 32);

    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(journal_mode, "wal");
    assert_eq!(foreign_keys, 1);
    assert!(state.db.integrity_check().await.unwrap());
}

#[tokio::test]
async fn login_refresh_rotation_and_logout_work() {
    let (_temp, config, state) = test_app().await;
    let password_hash = security::hash_password("correct horse battery staple").unwrap();
    state
        .db
        .create_account("listener", &password_hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state);

    let login = json_request(
        "/api/v2/auth/login",
        serde_json::json!({
            "username": "listener",
            "password": "correct horse battery staple",
            "device_name": "Integration browser"
        }),
    );
    let response = router.clone().oneshot(login).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let login_body = json_body(response).await;
    let access = login_body["access_token"].as_str().unwrap().to_owned();
    let refresh = login_body["refresh_token"].as_str().unwrap().to_owned();
    assert!(access.starts_with("wfa_"));
    assert!(refresh.starts_with("wfr_"));

    let response = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/refresh",
            serde_json::json!({ "refresh_token": refresh }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let refreshed = json_body(response).await;
    let new_access = refreshed["access_token"].as_str().unwrap();
    let new_refresh = refreshed["refresh_token"].as_str().unwrap();
    assert_ne!(new_access, access);

    let reused = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/refresh",
            serde_json::json!({ "refresh_token": refresh }),
        ))
        .await
        .unwrap();
    assert_eq!(reused.status(), StatusCode::UNAUTHORIZED);

    let logout = Request::post("/api/v2/auth/logout")
        .header("authorization", format!("Bearer {new_access}"))
        .body(Body::empty())
        .unwrap();
    let response = router.oneshot(logout).await.unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(new_refresh.starts_with("wfr_"));
}

#[tokio::test]
async fn browser_session_uses_http_only_refresh_cookie_origin_and_csrf() {
    let (_temp, config, state) = test_app().await;
    let password = Uuid::new_v4().to_string();
    let password_hash = security::hash_password(&password).unwrap();
    state
        .db
        .create_account("web-listener", &password_hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state);

    let mut request = json_request(
        "/api/v2/web/auth/login",
        serde_json::json!({
            "username": "web-listener",
            "password": &password,
            "device_name": "Embedded web player"
        }),
    );
    request
        .headers_mut()
        .insert("origin", "http://waveflow.test".parse().unwrap());
    request
        .headers_mut()
        .insert("host", "waveflow.test".parse().unwrap());
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let refresh_cookie = cookies
        .iter()
        .find(|cookie| cookie.starts_with("waveflow-refresh="))
        .unwrap();
    assert!(refresh_cookie.contains("HttpOnly"));
    assert!(refresh_cookie.contains("SameSite=Strict"));
    assert!(refresh_cookie.contains("Path=/api/v2/web/auth"));
    let refresh_pair = refresh_cookie.split(';').next().unwrap().to_owned();
    let csrf_pair = cookies
        .iter()
        .find(|cookie| cookie.starts_with("waveflow-csrf="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf = csrf_pair.split_once('=').unwrap().1.to_owned();
    let body = json_body(response).await;
    assert!(body["access_token"].as_str().unwrap().starts_with("wfa_"));
    assert!(body.get("refresh_token").is_none());

    let missing_csrf = Request::post("/api/v2/web/auth/refresh")
        .header("origin", "http://waveflow.test")
        .header("host", "waveflow.test")
        .header("cookie", format!("{refresh_pair}; {csrf_pair}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(missing_csrf).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    let wrong_csrf = Request::post("/api/v2/web/auth/refresh")
        .header("origin", "http://waveflow.test")
        .header("host", "waveflow.test")
        .header("cookie", format!("{refresh_pair}; {csrf_pair}"))
        .header("x-waveflow-csrf", "wfcsrf_wrong")
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        router.clone().oneshot(wrong_csrf).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    let refresh = Request::post("/api/v2/web/auth/refresh")
        .header("origin", "http://waveflow.test")
        .header("host", "waveflow.test")
        .header("cookie", format!("{refresh_pair}; {csrf_pair}"))
        .header("x-waveflow-csrf", csrf)
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(refresh).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let refreshed_cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let refreshed_refresh = refreshed_cookies
        .iter()
        .find(|cookie| cookie.starts_with("waveflow-refresh="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let refreshed_csrf = refreshed_cookies
        .iter()
        .find(|cookie| cookie.starts_with("waveflow-csrf="))
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let refreshed_csrf_value = refreshed_csrf.split_once('=').unwrap().1;

    let logout = Request::post("/api/v2/web/auth/logout")
        .header("origin", "http://waveflow.test/")
        .header("host", "ignored.invalid")
        .header("cookie", format!("{refreshed_refresh}; {refreshed_csrf}"))
        .header("x-waveflow-csrf", refreshed_csrf_value)
        .body(Body::empty())
        .unwrap();
    let logout = router.clone().oneshot(logout).await.unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    let expired = logout
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|value| value.to_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(expired.len(), 2);
    assert!(expired.iter().all(|cookie| cookie.contains("Max-Age=0")));
    assert!(expired.iter().any(|cookie| {
        cookie.starts_with("waveflow-refresh=")
            && cookie.contains("Path=/api/v2/web/auth")
            && cookie.contains("HttpOnly")
    }));

    let logout_without_refresh = Request::post("/api/v2/web/auth/logout")
        .header("origin", "http://waveflow.test")
        .header("cookie", &refreshed_csrf)
        .header("x-waveflow-csrf", refreshed_csrf_value)
        .body(Body::empty())
        .unwrap();
    let logout_without_refresh = router
        .clone()
        .oneshot(logout_without_refresh)
        .await
        .unwrap();
    assert_eq!(logout_without_refresh.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        logout_without_refresh
            .headers()
            .get_all("set-cookie")
            .iter()
            .count(),
        2
    );

    let mut foreign = json_request(
        "/api/v2/web/auth/login",
        serde_json::json!({
            "username": "web-listener",
            "password": &password,
            "device_name": "Foreign page"
        }),
    );
    foreign
        .headers_mut()
        .insert("origin", "https://attacker.invalid".parse().unwrap());
    foreign
        .headers_mut()
        .insert("host", "waveflow.test".parse().unwrap());
    assert_eq!(
        router.oneshot(foreign).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
}

/// RFC-023 decision 4: nothing an account handed out keeps working once it is
/// disabled — an API token, a session and its refresh token, nor an
/// authorization code granted before and redeemed after.
///
/// Each was already refused by the query that reads it (`a.disabled = 0` in
/// the token lookups, the account check in `issue_session_for_account`), with
/// no test saying so. Re-enabling the account at the end proves the refusals
/// were the account's and not the credentials going stale.
#[tokio::test]
async fn a_disabled_account_s_tokens_and_grants_all_stop_working() {
    let (_temp, config, state) = test_app().await;
    let (owner, api_token) = admin_with_api_token(&state, "disabled-owner").await;
    let router = waveflow_server::app(&config, state.clone());
    let albums = |bearer: String| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::get("/api/v2/albums")
                        .header("authorization", format!("Bearer {bearer}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
    };
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let redirect_uri = "http://127.0.0.1:49152/callback";
    let grant = serde_json::json!({
        "client_id": "com.waveflow.desktop",
        "redirect_uri": redirect_uri,
        "code_challenge": waveflow_server::auth::oauth::challenge_for(&verifier),
        "code_challenge_method": "S256",
        "device_name": "Disabled Desktop"
    });
    let authorize = || async {
        let granted = send_json(
            &router,
            "POST",
            "/api/v2/oauth/authorize",
            &api_token,
            Some(grant.clone()),
        )
        .await;
        assert_eq!(granted.status(), StatusCode::OK);
        code_from(json_body(granted).await["redirect_to"].as_str().unwrap())
    };
    let exchange = |code: String| {
        let router = router.clone();
        let verifier = verifier.clone();
        async move {
            router
                .oneshot(json_request(
                    "/api/v2/oauth/token",
                    serde_json::json!({
                        "code": code,
                        "code_verifier": verifier,
                        "client_id": "com.waveflow.desktop",
                        "redirect_uri": redirect_uri
                    }),
                ))
                .await
                .unwrap()
        }
    };
    let refresh = |refresh_token: String| {
        let router = router.clone();
        async move {
            router
                .oneshot(json_request(
                    "/api/v2/auth/refresh",
                    serde_json::json!({ "refresh_token": refresh_token }),
                ))
                .await
                .unwrap()
                .status()
        }
    };

    // Before: the token works, a grant redeems into a session that works, and
    // a second grant is left unredeemed for after.
    assert_eq!(albums(api_token.clone()).await, StatusCode::OK);
    let session = exchange(authorize().await).await;
    assert_eq!(session.status(), StatusCode::OK);
    let session = json_body(session).await;
    let access = session["access_token"].as_str().unwrap().to_owned();
    let refresh_token = session["refresh_token"].as_str().unwrap().to_owned();
    assert_eq!(albums(access.clone()).await, StatusCode::OK);
    let pending = authorize().await;
    let spare = authorize().await;

    set_disabled(&state, owner, true).await;
    assert_eq!(albums(api_token.clone()).await, StatusCode::UNAUTHORIZED);
    assert_eq!(albums(access.clone()).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        refresh(refresh_token.clone()).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        exchange(pending).await.status(),
        StatusCode::UNAUTHORIZED,
        "a code granted before the account was disabled must not be cashed in after"
    );

    // Re-enabled, the same credentials answer again: what refused them was the
    // account, not their age.
    set_disabled(&state, owner, false).await;
    assert_eq!(albums(api_token).await, StatusCode::OK);
    assert_eq!(albums(access).await, StatusCode::OK);
    assert_eq!(refresh(refresh_token).await, StatusCode::OK);
    // And the session it redeems into is this account's, not merely a session.
    let spare = exchange(spare).await;
    assert_eq!(spare.status(), StatusCode::OK);
    let spare = json_body(spare).await;
    assert_eq!(spare["user"]["id"], owner.to_string());
    assert_eq!(spare["user"]["username"], "disabled-owner");
}
