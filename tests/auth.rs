//! Bootstrapping, sessions, tokens and the authorization code flow.
//!
//! Split out of `v2_foundations.rs`.

use axum::body::Body;
use axum::http::Method;
use axum::http::Request;
use axum::http::StatusCode;
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::authentication::now_ms;
use waveflow_server::database::AccountRole;
use waveflow_server::database::LibraryRole;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::security;

// Not every target uses every fixture, and a shared module is not dead
// code for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

/// Pulls the authorization code out of the redirect the consent step returns.
fn code_from(redirect_to: &str) -> String {
    redirect_to
        .split("code=")
        .nth(1)
        .expect("the redirect carries a code")
        .split('&')
        .next()
        .expect("the code is delimited")
        .to_owned()
}

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

#[tokio::test]
async fn setup_and_native_administration_cover_users_credentials_and_libraries() {
    let (_temp, config, state) = test_app().await;
    let admin_password = Uuid::new_v4().to_string();
    let listener_password = Uuid::new_v4().to_string();
    let music = config.data_dir.join("admin-library");
    std::fs::create_dir_all(&music).unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let status = router
        .clone()
        .oneshot(Request::get("/api/v2/setup").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(json_body(status).await["required"], true);

    let missing_origin = json_request(
        "/api/v2/setup",
        serde_json::json!({
            "username": "first-admin",
            "password": &admin_password
        }),
    );
    assert_eq!(
        router
            .clone()
            .oneshot(missing_origin)
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );

    let mut request = json_request(
        "/api/v2/setup",
        serde_json::json!({
            "username": "first-admin",
            "password": &admin_password
        }),
    );
    request
        .headers_mut()
        .insert("origin", "http://waveflow.test".parse().unwrap());
    request
        .headers_mut()
        .insert("host", "waveflow.test".parse().unwrap());
    assert_eq!(
        router.clone().oneshot(request).await.unwrap().status(),
        StatusCode::CREATED
    );

    let mut repeated = json_request(
        "/api/v2/setup",
        serde_json::json!({
            "username": "second-admin",
            "password": &admin_password
        }),
    );
    repeated
        .headers_mut()
        .insert("origin", "http://waveflow.test".parse().unwrap());
    repeated
        .headers_mut()
        .insert("host", "waveflow.test".parse().unwrap());
    // Already initialised: the request is well formed, it collides with state.
    assert_eq!(
        router.clone().oneshot(repeated).await.unwrap().status(),
        StatusCode::CONFLICT
    );

    let admin_token = login_token(&router, "first-admin", &admin_password).await;
    let create_user = Request::post("/api/v2/admin/users")
        .header("authorization", format!("Bearer {admin_token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "username": "native-listener",
                "web_password": &listener_password,
                "role": "user"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.clone().oneshot(create_user).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let listener_id = Uuid::parse_str(json_body(response).await["id"].as_str().unwrap()).unwrap();

    let credential = Request::put("/api/v2/admin/users/native-listener/subsonic-credential")
        .header("authorization", format!("Bearer {admin_token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({ "password": "dedicated subsonic password" }).to_string(),
        ))
        .unwrap();
    let response = router.clone().oneshot(credential).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let api_key = json_body(response).await["api_key"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(api_key.starts_with("wfsk_"));
    assert!(state
        .services
        .credential_by_api_key(&api_key)
        .await
        .unwrap()
        .is_some());

    let create_library = Request::post("/api/v2/libraries")
        .header("authorization", format!("Bearer {admin_token}"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "name": "Native library",
                "path": std::fs::canonicalize(&music).unwrap().to_string_lossy(),
                "visibility": "shared"
            })
            .to_string(),
        ))
        .unwrap();
    let response = router.clone().oneshot(create_library).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let library_id =
        Uuid::parse_str(json_body(response).await["library_id"].as_str().unwrap()).unwrap();

    let set_member = Request::put(format!(
        "/api/v2/libraries/{library_id}/members/{listener_id}"
    ))
    .header("authorization", format!("Bearer {admin_token}"))
    .header("content-type", "application/json")
    .body(Body::from(
        serde_json::json!({ "role": "listener" }).to_string(),
    ))
    .unwrap();
    assert_eq!(
        router.clone().oneshot(set_member).await.unwrap().status(),
        StatusCode::NO_CONTENT
    );

    let listener_token = login_token(&router, "native-listener", &listener_password).await;
    let libraries = router
        .clone()
        .oneshot(
            Request::get("/api/v2/libraries")
                .header("authorization", format!("Bearer {listener_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let libraries = json_body(libraries).await;
    assert_eq!(libraries.as_array().unwrap().len(), 1);
    assert_eq!(libraries[0]["id"], library_id.to_string());

    let forbidden = router
        .oneshot(
            Request::get("/api/v2/admin/users")
                .header("authorization", format!("Bearer {listener_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn long_lived_native_api_tokens_authenticate_and_honor_revocation() {
    let (_temp, config, state) = test_app().await;
    let now = now_ms();
    let password_hash =
        security::hash_password(&security::generate_token("test-password-")).unwrap();
    let user_id = state
        .db
        .create_account("native-token-user", &password_hash, AccountRole::User, now)
        .await
        .unwrap();
    let token = security::generate_token("wfapi_");
    state
        .db
        .create_api_token(
            user_id,
            "integration token",
            &security::token_hash(&token),
            &[],
            now,
        )
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let accepted = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);
    let last_used_at: Option<i64> =
        sqlx::query_scalar("SELECT last_used_at FROM api_token WHERE token_hash = ?")
            .bind(security::token_hash(&token).as_slice())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(last_used_at.is_some_and(|last_used| last_used >= now));

    assert!(state
        .db
        .revoke_api_token_by_hash(&security::token_hash(&token), now_ms())
        .await
        .unwrap());
    let revoked = router
        .oneshot(
            Request::get("/api/v2/albums")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
}

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
    let challenge = waveflow_server::oauth::challenge_for(verifier);
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
    let code = redirect_to
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned();

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
    let code = redirect_to
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned();

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
