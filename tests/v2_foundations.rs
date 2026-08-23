use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use sqlx::Row;
use tempfile::TempDir;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;
use uuid::Uuid;
use waveflow_server::{
    authentication::now_ms,
    catalog::{ApplyOutcome, CatalogTrackInput, LibraryRecord},
    database::{AccountRole, LibraryRole, LibraryVisibility},
    security,
    services::{ServiceError, MAX_HISTORY_LIMIT, MAX_QUEUE_TRACKS, MAX_SHARE_TRACKS},
    sync::{MutationContext, SyncError, MAX_SYNC_LIMIT},
    Config,
};

async fn test_app() -> (TempDir, Config, waveflow_server::AppState) {
    let temp = tempfile::tempdir().unwrap();
    let config = Config::for_data_dir(temp.path().join("data"));
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
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
async fn probes_and_openapi_are_available_without_scan_readiness() {
    let (_temp, mut config, state) = test_app().await;
    config.allowed_origins = vec!["http://127.0.0.1:9180".parse().unwrap()];
    let router = waveflow_server::app(&config, state);

    let health = router
        .clone()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(json_body(health).await["schema"], 2);

    let ready = router
        .clone()
        .oneshot(Request::get("/ready").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ready.status(), StatusCode::OK);
    assert_eq!(json_body(ready).await["database"], "ok");

    let cors = router
        .clone()
        .oneshot(
            Request::get("/health")
                .header("origin", "http://127.0.0.1:9180")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        cors.headers()["access-control-allow-origin"],
        "http://127.0.0.1:9180"
    );
    let rejected = router
        .clone()
        .oneshot(
            Request::get("/health")
                .header("origin", "https://untrusted.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(rejected
        .headers()
        .get("access-control-allow-origin")
        .is_none());

    let openapi = router
        .oneshot(Request::get("/openapi.json").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(openapi.status(), StatusCode::OK);
    let document = json_body(openapi).await;
    assert!(document["paths"]["/api/v2/auth/login"].is_object());
    for path in [
        "/api/v2/setup",
        "/api/v2/web/auth/login",
        "/api/v2/libraries",
        "/api/v2/admin/users",
        "/api/v2/sync/snapshot",
        "/api/v2/sync/changes",
        "/api/v2/sync/ack",
        "/api/v2/sync/socket",
        "/api/v2/transcode/status",
        "/api/v2/tracks/{track_id}/lyrics",
    ] {
        assert!(document["paths"][path].is_object(), "missing {path}");
    }

    // A client generated from this document alone must authenticate correctly
    // and keep replay safety. Neither is inferable from the paths.
    assert_eq!(
        document["components"]["securitySchemes"]["bearer"]["scheme"],
        "bearer"
    );
    assert_eq!(
        document["security"][0]["bearer"].as_array().map(Vec::len),
        Some(0)
    );

    // Endpoints carrying their own credential, or none, must opt out of the
    // global requirement — otherwise the document claims a token is needed to
    // log in, and that the stream ticket is not itself the credential.
    for (path, method) in [
        ("/health", "get"),
        ("/api/v2/auth/login", "post"),
        ("/api/v2/oauth/token", "post"),
        ("/api/v2/stream/{ticket}", "get"),
    ] {
        assert_eq!(
            document["paths"][path][method]["security"]
                .as_array()
                .map(Vec::len),
            Some(0),
            "{method} {path} should be documented as public"
        );
    }

    // The two mutation headers are what the native API offers over the Subsonic
    // facade; a document that omits them yields clients that never replay
    // safely.
    let headers = document["paths"]["/api/v2/scrobbles"]["post"]["parameters"]
        .as_array()
        .map(|parameters| {
            parameters
                .iter()
                .filter_map(|parameter| parameter["name"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        headers.iter().any(|name| name == "x-waveflow-operation-id"),
        "scrobbles should document the operation id header, got {headers:?}"
    );
    assert!(
        headers.iter().any(|name| name == "x-waveflow-device-id"),
        "scrobbles should document the device id header, got {headers:?}"
    );
    // Every user-data write can fail on a replayed operation id, so the 409 is
    // part of their contract — a client that only handles 2xx/4xx generically
    // would retry it forever.
    assert!(document["paths"]["/api/v2/scrobbles"]["post"]["responses"]["409"].is_object());
    // And the sync read has its own 409, with a different code and a different
    // recovery. Documenting one without the other would be worse than neither.
    assert!(document["paths"]["/api/v2/sync/changes"]["get"]["responses"]["409"].is_object());

    // Reads carry no operation id: annotating them would tell a generator to
    // send headers the handler never looks at.
    let read_parameters = document["paths"]["/api/v2/favorites"]["get"]["parameters"]
        .as_array()
        .map(|parameters| {
            parameters
                .iter()
                .filter_map(|parameter| parameter["name"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        !read_parameters
            .iter()
            .any(|name| name.starts_with("x-waveflow-")),
        "a read should not advertise mutation headers, got {read_parameters:?}"
    );
}

#[tokio::test]
async fn scanner_indexes_moves_and_marks_tracks_unavailable() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("scanner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("scan-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("First Track.wav"));
    std::fs::write(
        music.join("First Track.lrc"),
        "[00:01.25]First line\n[00:02.500]Second line",
    )
    .unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Scanner library",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Scanner library".into(),
        root_path: std::fs::canonicalize(&music).unwrap(),
    };

    run_scan(&state, owner, library.clone()).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].title, "First Track");
    assert!(tracks[0].available);
    let stable_id = tracks[0].id;
    let lyrics = state.services.lyrics(owner, stable_id).await.unwrap();
    assert_eq!(lyrics.structured_lyrics.len(), 1);
    assert!(lyrics.structured_lyrics[0].synced);
    assert_eq!(lyrics.structured_lyrics[0].lines[0].start, Some(1_250));
    assert_eq!(lyrics.structured_lyrics[0].lines[0].value, "First line");
    let found = state
        .db
        .search_tracks_for_user(owner, library_id, "First")
        .await
        .unwrap();
    assert_eq!(found[0].id, stable_id);

    // A sidecar can change while the audio bytes and timestamps stay exactly
    // the same. Its fingerprint must prevent the scanner's unchanged-file fast
    // path from preserving stale lyrics.
    std::fs::write(music.join("First Track.lrc"), "[00:03]Replacement").unwrap();
    run_scan(&state, owner, library.clone()).await;
    let lyrics = state.services.lyrics(owner, stable_id).await.unwrap();
    assert_eq!(lyrics.structured_lyrics[0].lines.len(), 1);
    assert_eq!(lyrics.structured_lyrics[0].lines[0].start, Some(3_000));
    assert_eq!(lyrics.structured_lyrics[0].lines[0].value, "Replacement");

    std::fs::create_dir_all(music.join("Moved")).unwrap();
    std::fs::rename(
        music.join("First Track.wav"),
        music.join("Moved").join("Renamed.wav"),
    )
    .unwrap();
    run_scan(&state, owner, library.clone()).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id, stable_id);
    assert_eq!(tracks[0].relative_path, "Moved/Renamed.wav");

    std::fs::remove_file(music.join("Moved").join("Renamed.wav")).unwrap();
    run_scan(&state, owner, library).await;
    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert!(!tracks[0].available);
}

#[tokio::test]
async fn scanner_batches_more_than_one_write_group_without_deduplicating_copies() {
    let (_temp, config, state) = test_app().await;
    let password_hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account(
            "batch-scanner",
            &password_hash,
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("batch-scan");
    std::fs::create_dir_all(&music).unwrap();
    for index in 0..30 {
        write_test_wav(&music.join(format!("Track {index:02}.wav")));
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Batch scanner",
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
            id: library_id,
            name: "Batch scanner".into(),
            root_path: root,
        },
    )
    .await;

    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 30);
    assert!(tracks.iter().all(|track| track.available));
    assert_eq!(
        state
            .db
            .search_tracks_for_user(owner, library_id, "Track")
            .await
            .unwrap()
            .len(),
        30
    );
}

#[tokio::test]
async fn scanner_indexes_dsd64_and_deduplicates_folder_artwork() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("formats", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("formats");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("One.wav"));
    write_test_wav(&music.join("Two.wav"));
    write_test_dsf(&music.join("Native DSD.dsf"));
    std::fs::write(music.join("Native DSD.lrc"), "[00:01]DSD words").unwrap();
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Formats",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Formats".into(),
        root_path: root,
    };
    run_scan(&state, owner, library.clone()).await;

    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 3);
    let dsd = tracks
        .iter()
        .find(|track| track.title == "Native DSD")
        .unwrap();
    assert_eq!(dsd.codec.as_deref(), Some("DSD64"));
    let dsd_lyrics = state.services.lyrics(owner, dsd.id).await.unwrap();
    assert_eq!(dsd_lyrics.structured_lyrics[0].lines[0].start, Some(1_000));
    assert_eq!(dsd_lyrics.structured_lyrics[0].lines[0].value, "DSD words");
    let dsd_depth: i64 = sqlx::query_scalar("SELECT bit_depth FROM track WHERE id = ?")
        .bind(dsd.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(dsd_depth, 1);

    let artwork_hashes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT artwork_hash FROM track WHERE library_id = ? AND artwork_hash IS NOT NULL",
    )
    .bind(library_id.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(artwork_hashes.len(), 1);
    let artwork_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM artwork")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(artwork_rows, 1);

    std::fs::write(music.join("Native DSD.lrc"), "[00:02]Updated DSD words").unwrap();
    run_scan(&state, owner, library).await;
    let dsd_lyrics = state.services.lyrics(owner, dsd.id).await.unwrap();
    assert_eq!(dsd_lyrics.structured_lyrics[0].lines[0].start, Some(2_000));
    assert_eq!(
        dsd_lyrics.structured_lyrics[0].lines[0].value,
        "Updated DSD words"
    );
}

#[tokio::test]
async fn compilation_and_multi_artist_materialization_is_deterministic() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("metadata", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("metadata");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Metadata",
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

    for (index, artist) in ["Alpha; Beta", "Gamma"].into_iter().enumerate() {
        let outcome = state
            .db
            .apply_catalog_track(
                library_id,
                scan_id,
                &catalog_input(index, artist),
                None,
                false,
            )
            .await
            .unwrap();
        assert_eq!(outcome, ApplyOutcome::Added);
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let albums: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM album WHERE library_id = ? AND title = 'Shared compilation'",
    )
    .bind(library_id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(albums, 1);
    let album_row =
        sqlx::query("SELECT album_artist_name, is_compilation FROM album WHERE library_id = ?")
            .bind(library_id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(
        album_row.get::<String, _>("album_artist_name"),
        "Various Artists"
    );
    assert_eq!(album_row.get::<i64, _>("is_compilation"), 1);

    let first_track_artists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM track_participant tp JOIN track t ON t.id = tp.track_id \
         WHERE t.library_id = ? AND tp.role = 'artist' \
           AND t.relative_path = 'track-0.flac'",
    )
    .bind(library_id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(first_track_artists, 2);
    let genres: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM genre WHERE library_id = ?")
        .bind(library_id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(genres, 2);
    assert_eq!(
        state
            .db
            .search_tracks_for_user(owner, library_id, "Beta")
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn ffmpeg_generated_catalog_format_matrix_is_indexed() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("matrix", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("format-matrix");
    std::fs::create_dir_all(&music).unwrap();
    for (extension, codec) in [
        ("mp3", "libmp3lame"),
        ("flac", "flac"),
        ("m4a", "aac"),
        ("ogg", "libvorbis"),
        ("wav", "pcm_s16le"),
        // AIFF arrives with the core bump: the desktop reads it through
        // symphonia's RIFF crate now, and this server reads the same list.
        ("aiff", "pcm_s16be"),
    ] {
        generate_audio_fixture(&music.join(format!("matrix.{extension}")), codec, extension);
    }
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Matrix", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    run_scan(
        &state,
        owner,
        LibraryRecord {
            id: library_id,
            name: "Matrix".into(),
            root_path: root,
        },
    )
    .await;

    let tracks = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 6);

    // AIFF is asserted apart, and on less. FFmpeg's AIFF muxer writes the
    // title and stops — `ffprobe` reports no album and no artist on the file
    // it just produced — so demanding them here would test the fixture
    // generator rather than the scanner. What the bump actually changed is
    // that the extension is admitted at all, and that is what this checks.
    let aiff = tracks
        .iter()
        .find(|track| track.relative_path == "matrix.aiff")
        .expect("aiff is admitted by the extension list");
    assert_eq!(aiff.title, "Matrix aiff");
    assert!(aiff.duration_ms > 0);
    let aiff_bytes = std::fs::read(music.join("matrix.aiff")).unwrap();
    assert_eq!(
        aiff.full_hash,
        blake3::hash(&aiff_bytes).to_hex().to_string()
    );

    for extension in ["mp3", "flac", "m4a", "ogg", "wav"] {
        let track = tracks
            .iter()
            .find(|track| track.relative_path == format!("matrix.{extension}"))
            .unwrap_or_else(|| panic!("missing {extension} from matrix"));
        assert_eq!(track.title, format!("Matrix {extension}"));
        assert_eq!(track.album.as_deref(), Some("WaveFlow format matrix"));
        assert_eq!(track.artist.as_deref(), Some("Alpha; Beta"));
        assert!(track.duration_ms > 0);

        // full_hash is published to clients as *the* reconciliation key, so its
        // algorithm is part of the contract. Check the served value against the
        // file rather than trusting the column name: a client computing BLAKE3
        // locally and getting something else would match nothing, silently.
        let bytes = std::fs::read(music.join(format!("matrix.{extension}"))).unwrap();
        assert_eq!(
            track.full_hash,
            blake3::hash(&bytes).to_hex().to_string(),
            "full_hash must be unkeyed BLAKE3 over the whole {extension} file"
        );
        assert_eq!(track.full_hash.len(), 64);
    }
}

#[tokio::test]
async fn catalog_and_scan_routes_blur_foreign_libraries() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("route-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("route-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("route-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Private.wav"));
    write_test_wav(&music.join("Private 2.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Private library",
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
            id: library_id,
            name: "Private library".into(),
            root_path: root,
        },
    )
    .await;
    let router = waveflow_server::app(&config, state);
    let owner_token = login_token(&router, "route-owner", password).await;
    let intruder_token = login_token(&router, "route-intruder", password).await;

    let owner_response = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/libraries/{library_id}/tracks"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(owner_response.status(), StatusCode::OK);
    assert_eq!(json_body(owner_response).await.as_array().unwrap().len(), 2);

    let page = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v2/libraries/{library_id}/tracks?limit=1&offset=1"
            ))
            .header("authorization", format!("Bearer {owner_token}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(json_body(page).await.as_array().unwrap().len(), 1);

    let invalid_page = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/libraries/{library_id}/tracks?limit=501"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_page.status(), StatusCode::UNPROCESSABLE_ENTITY);

    for method in ["GET", "POST"] {
        let uri = if method == "GET" {
            format!("/api/v2/libraries/{library_id}/tracks")
        } else {
            format!("/api/v2/libraries/{library_id}/scans")
        };
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", format!("Bearer {intruder_token}"))
            .body(Body::empty())
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn track_pages_are_stable_when_titles_and_fts_ranks_match() {
    let (_temp, config, state) = test_app().await;
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("stable-pages", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("stable-pages");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Stable pages",
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
    for index in 0..2 {
        state
            .db
            .apply_catalog_track(
                library_id,
                scan_id,
                &browse_input(
                    10_000 + index,
                    "Mirror Signal",
                    "Stable Paging",
                    "Deterministic Artist",
                    Some(index as i64 + 1),
                    Some(1),
                ),
                None,
                false,
            )
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state);
    let token = login_token(&router, "stable-pages", &password).await;
    let page_id = |query: &'static str| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(format!("/api/v2/libraries/{library_id}/tracks?{query}"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = json_body(response).await;
            let page = body.as_array().unwrap();
            assert_eq!(page.len(), 1);
            page[0]["id"].as_str().unwrap().to_owned()
        }
    };

    let normal = vec![
        page_id("limit=1&offset=0").await,
        page_id("limit=1&offset=1").await,
    ];
    let fts = vec![
        page_id("q=Mirror%20Signal&limit=1&offset=0").await,
        page_id("q=Mirror%20Signal&limit=1&offset=1").await,
    ];
    let mut expected = normal.clone();
    expected.sort();
    assert_eq!(normal, expected, "title ties use the UUID as final order");
    assert_eq!(fts, expected, "FTS rank ties use the UUID as final order");
    assert_ne!(expected[0], expected[1]);
}

#[tokio::test]
async fn media_streaming_ranges_transcodes_caches_and_isolates_tenants() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("media-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("media-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("media-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Range.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Media library",
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
            id: library_id,
            name: "Media library".into(),
            root_path: root,
        },
    )
    .await;
    let track = state
        .db
        .list_tracks_for_user(owner, library_id)
        .await
        .unwrap()
        .pop()
        .unwrap();
    let media = state.media.clone();
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "media-owner", password).await;
    let intruder_token = login_token(&router, "media-intruder", password).await;
    let uri = format!("/api/v2/tracks/{}/stream", track.id);

    let response = router
        .clone()
        .oneshot(
            Request::get(&uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=0-9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()["content-range"], "bytes 0-9/1644");
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(bytes.len(), 10);

    let unsatisfiable = router
        .clone()
        .oneshot(
            Request::get(&uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=99999-")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsatisfiable.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    let hidden = router
        .clone()
        .oneshot(
            Request::get(&uri)
                .header("authorization", format!("Bearer {intruder_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let transcode_uri = format!("{uri}?format=mp3&bitrate=96");
    let response = router
        .clone()
        .oneshot(
            Request::get(&transcode_uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["accept-ranges"], "none");
    assert_eq!(response.headers()["content-type"], "audio/mpeg");
    assert!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len()
            > 100
    );

    let cache_file = wait_for_cache_file(&config.transcode_cache_dir, "mp3").await;
    assert!(cache_file.metadata().unwrap().len() > 100);
    let cached_range = router
        .clone()
        .oneshot(
            Request::get(&transcode_uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=0-31")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cached_range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        cached_range
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len(),
        32
    );

    // Two consumers of the same missing cache key converge on one FFmpeg job:
    // the second waits for the per-key guard, then reads the committed file.
    let concurrent_uri = format!("{uri}?format=mp3&bitrate=112");
    let first_router = router.clone();
    let second_router = router.clone();
    let first_token = owner_token.clone();
    let second_token = owner_token.clone();
    let first_uri = concurrent_uri.clone();
    let first = async move {
        let response = first_router
            .oneshot(
                Request::get(first_uri)
                    .header("authorization", format!("Bearer {first_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        response.into_body().collect().await.unwrap().to_bytes()
    };
    let second = async move {
        let response = second_router
            .oneshot(
                Request::get(concurrent_uri)
                    .header("authorization", format!("Bearer {second_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        response.into_body().collect().await.unwrap().to_bytes()
    };
    let (first_bytes, second_bytes) = tokio::join!(first, second);
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(
        std::fs::read_dir(&config.transcode_cache_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(
                |entry| entry.path().extension().and_then(std::ffi::OsStr::to_str) == Some("mp3")
            )
            .count(),
        2,
        "duplicate consumers must create only one file per cache key"
    );

    // A browser audio element opens every resource with `Range: bytes=0-`, so
    // a cold transcode must answer it rather than refuse the range. Refusing
    // made a web client fail on the first play of a track and succeed on the
    // second, once the cache existed.
    let cold_open = router
        .clone()
        .oneshot(
            Request::get(format!("{uri}?format=mp3&bitrate=128"))
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=0-")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cold_open.status(), StatusCode::OK);
    assert_eq!(cold_open.headers()["content-type"], "audio/mpeg");
    assert!(
        cold_open
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len()
            > 100
    );

    // iOS probes a resource with a two-byte range before it plays anything, so
    // a bounded range from zero has to open the stream too. Juliet failed on
    // the first play of every track until it did.
    let cold_probe = router
        .clone()
        .oneshot(
            Request::get(format!("{uri}?format=mp3&bitrate=136"))
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=0-1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cold_probe.status(), StatusCode::OK);
    assert_eq!(cold_probe.headers()["content-type"], "audio/mpeg");
    // Drained and awaited: an unread transcode holds its per-user permit, and
    // the checks below would meet 429 instead of what they are testing.
    cold_probe.into_body().collect().await.unwrap();
    for _ in 0..100 {
        if media.active_transcodes() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        media.active_transcodes(),
        0,
        "a drained transcode must release its permit"
    );

    // A range that actually seeks still has no meaning before the transcode
    // exists, and keeps its refusal.
    let cold_seek = router
        .clone()
        .oneshot(
            Request::get(format!("{uri}?format=mp3&bitrate=144"))
                .header("authorization", format!("Bearer {owner_token}"))
                .header("range", "bytes=64-")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cold_seek.status(), StatusCode::RANGE_NOT_SATISFIABLE);

    let live_seek = router
        .clone()
        .oneshot(
            Request::get(format!("{uri}?format=opus&bitrate=64&offsetMs=25"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(live_seek.status(), StatusCode::OK);
    assert_eq!(live_seek.headers()["accept-ranges"], "none");
    drop(live_seek);
    for _ in 0..100 {
        if media.active_transcodes() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        media.active_transcodes(),
        0,
        "abandoned FFmpeg was not cancelled"
    );
    assert!(!std::fs::read_dir(&config.transcode_cache_dir)
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().contains(".part-")));

    sqlx::query("UPDATE track SET relative_path = '../outside.wav' WHERE id = ?")
        .bind(track.id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let escaped = router
        .oneshot(
            Request::get(&uri)
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(escaped.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn startup_reports_missing_ffmpeg() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = Config::for_data_dir(temp.path().join("data"));
    config.ffmpeg_path = temp.path().join("missing-ffmpeg");
    let error = match waveflow_server::initialize(&config).await {
        Ok(_) => panic!("initialization unexpectedly accepted a missing FFmpeg"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("ffmpeg is required"));
}

#[tokio::test]
async fn subsonic_xml_json_auth_catalog_and_user_data_are_compatible() {
    use md5::{Digest, Md5};

    let (_temp, config, state) = test_app().await;
    let web_password = "correct horse battery staple";
    let subsonic_password = "subsonic-secret-123";
    let api_key = "wfsk_golden-api-key";
    let web_hash = security::hash_password(web_password).unwrap();
    let admin = state
        .db
        .create_account("sub-admin", &web_hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let encrypted = state
        .secret_box
        .encrypt(subsonic_password.as_bytes())
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("subsonic-music");
    std::fs::create_dir_all(&music).unwrap();
    generate_audio_fixture(&music.join("Golden.wav"), "pcm_s16le", "wav");
    std::fs::write(
        music.join("Golden.lrc"),
        "[00:01.25]Golden opening\n[00:02.500]Golden chorus",
    )
    .unwrap();
    write_test_wav(&music.join("NoArtist.wav"));
    write_test_png(&music.join("cover.png"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Subsonic library",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    run_scan(
        &state,
        admin,
        LibraryRecord {
            id: library,
            name: "Subsonic library".into(),
            root_path: root,
        },
    )
    .await;
    let secondary_music = config.data_dir.join("subsonic-secondary");
    std::fs::create_dir_all(&secondary_music).unwrap();
    let secondary_root = std::fs::canonicalize(&secondary_music).unwrap();
    let secondary_library = state
        .db
        .create_library(
            admin,
            "Secondary Subsonic library",
            &secondary_root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let foreign_owner = state
        .db
        .create_account("sub-foreign-owner", &web_hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let foreign_music = config.data_dir.join("subsonic-foreign");
    std::fs::create_dir_all(&foreign_music).unwrap();
    let foreign_library = state
        .db
        .create_library(
            foreign_owner,
            "Foreign Subsonic library",
            &std::fs::canonicalize(&foreign_music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let foreign_scan = state
        .db
        .create_scan_job(foreign_library, Some(foreign_owner), "manual")
        .await
        .unwrap();
    state
        .db
        .start_scan_job(foreign_scan, 1, false)
        .await
        .unwrap();
    let mut foreign_input = browse_input(
        7_000,
        "Foreign track",
        "Foreign album",
        "Foreign artist",
        Some(1),
        Some(1),
    );
    foreign_input.lyrics = vec![waveflow_server::lyrics::LyricsInput {
        source: "embedded",
        lang: "eng".into(),
        synced: false,
        content: "private words".into(),
    }];
    foreign_input.lyrics_hash = blake3::hash(b"private words").to_hex().to_string();
    state
        .db
        .apply_catalog_track(foreign_library, foreign_scan, &foreign_input, None, false)
        .await
        .unwrap();
    state
        .db
        .consolidate_musicbrainz_ids(foreign_library)
        .await
        .unwrap();
    state.db.finish_scan_job(foreign_scan, 0).await.unwrap();
    let foreign_artist = state
        .services
        .catalog_snapshot(foreign_owner, &[])
        .await
        .unwrap()
        .artists
        .first()
        .unwrap()
        .artist
        .id;
    let foreign_song = state
        .services
        .catalog_snapshot(foreign_owner, &[])
        .await
        .unwrap()
        .songs
        .first()
        .unwrap()
        .id;
    let foreign_album = state
        .services
        .catalog_snapshot(foreign_owner, &[])
        .await
        .unwrap()
        .albums
        .first()
        .unwrap()
        .id;
    let snapshot = state.services.catalog_snapshot(admin, &[]).await.unwrap();
    let song = snapshot
        .songs
        .iter()
        .find(|song| song.title == "Matrix wav")
        .expect("the tagged Golden fixture is present")
        .id;
    let no_artist_song = snapshot
        .songs
        .iter()
        .find(|song| song.artist_id.is_none())
        .expect("the untagged fixture has no track_artist row")
        .id;
    let artist = snapshot.artists.first().unwrap().artist.id;
    let album = snapshot.albums.first().unwrap().id;
    let artwork = snapshot
        .songs
        .first()
        .unwrap()
        .artwork_hash
        .clone()
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let plain_auth = format!("u=sub-admin&p={subsonic_password}&v=1.16.1&c=golden");

    let ping = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/ping.view?{plain_auth}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ping.status(), StatusCode::OK);
    let ping_xml = body_text(ping).await;
    assert!(ping_xml.starts_with("<subsonic-response"));
    assert!(ping_xml.contains("status=\"ok\""));
    assert!(!ping_xml.contains("<ping"));

    let symfonium_probe = router
        .clone()
        .oneshot(
            Request::get("/rest/ping.view?u=test&p=test&v=1.13.0&c=Symfonium&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(symfonium_probe.status(), StatusCode::OK);
    assert_eq!(
        json_body(symfonium_probe).await["subsonic-response"]["status"],
        "ok"
    );

    for path in [
        "/rest/getMusicFolders.view?u=test&p=test&v=1.13.0&c=Symfonium&f=json",
        "/rest/ping.view?u=test&p=test&v=1.13.0&c=another-client&f=json",
        "/rest/ping.view?u=test&p=test&apiKey=invalid&v=1.13.0&c=Symfonium&f=json",
        "/rest/ping.view?u=test&u=another&p=test&v=1.13.0&c=Symfonium&f=json",
    ] {
        let rejected = router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        // A refused credential is an HTTP 200 carrying error code 40: the
        // Subsonic contract puts the outcome in the body, and a client that
        // trusted the status line would never read it.
        assert_eq!(rejected.status(), StatusCode::OK, "{path}");
        let body = json_body(rejected).await;
        assert_eq!(body["subsonic-response"]["error"]["code"], 40, "{path}");
        // A failed response still identifies the server. WaveFlow Desktop
        // decides whether to enable the native /api/v2 surface from `type`
        // alone, before it holds any credential, so this must survive a
        // rewrite of the error envelope. getOpenSubsonicExtensions cannot
        // serve that purpose: it needs an authenticated call, and this decision
        // is made before there is a credential.
        assert_eq!(body["subsonic-response"]["type"], "waveflow", "{path}");
        assert_eq!(body["subsonic-response"]["openSubsonic"], true, "{path}");
        assert!(
            body["subsonic-response"]["serverVersion"].is_string(),
            "{path}"
        );
    }

    // Under `formPost` the requested format arrives in the body, not the query
    // string. A refused credential must still be answered in it: a client that
    // asked for JSON and received XML cannot read the error at all.
    let rejected_post = router
        .clone()
        .oneshot(
            Request::post("/rest/getMusicFolders.view")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("apiKey=wfsk_wrong&v=1.16.1&c=golden&f=json"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected_post.status(), StatusCode::OK);
    assert_eq!(
        rejected_post.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    assert_eq!(
        json_body(rejected_post).await["subsonic-response"]["error"]["code"],
        40
    );

    let salt = "golden-salt";
    let mut digest = Md5::new();
    digest.update(subsonic_password.as_bytes());
    digest.update(salt.as_bytes());
    let token_auth = format!(
        "u=sub-admin&t={}&s={salt}&v=1.16.1&c=golden&f=json",
        hex::encode(digest.finalize())
    );
    let token_ping = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/ping?{token_auth}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(token_ping.status(), StatusCode::OK);
    assert_eq!(
        json_body(token_ping).await["subsonic-response"]["status"],
        "ok"
    );

    let post_body = format!("apiKey={api_key}&v=1.16.1&c=golden&f=json");
    let folders = router
        .clone()
        .oneshot(
            Request::post("/rest/getMusicFolders.view")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(post_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(folders.status(), StatusCode::OK);
    let folders = json_body(folders).await;
    let folder_ids = folders["subsonic-response"]["musicFolders"]["musicFolder"]
        .as_array()
        .unwrap()
        .iter()
        .map(|folder| folder["id"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(folder_ids.contains(&library.to_string()));
    assert!(folder_ids.contains(&secondary_library.to_string()));

    // The same request with a conformant spelling of the media type: case is not
    // significant and a charset parameter is allowed, so neither may turn a
    // valid form POST into a protocol error.
    let cased = router
        .clone()
        .oneshot(
            Request::post("/rest/getMusicFolders.view")
                .header(
                    "content-type",
                    "Application/X-WWW-Form-Urlencoded; charset=UTF-8",
                )
                .body(Body::from(format!(
                    "apiKey={api_key}&v=1.16.1&c=golden&f=json"
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cased.status(), StatusCode::OK);
    assert_eq!(json_body(cased).await["subsonic-response"]["status"], "ok");

    // A type that merely starts with the expected one is a different type. The
    // format comes from the query string here because a rejected body is never
    // read, so it cannot carry `f`.
    let lookalike = router
        .clone()
        .oneshot(
            Request::post("/rest/getMusicFolders.view?f=json")
                .header("content-type", "application/x-www-form-urlencodedish")
                .body(Body::from(format!(
                    "apiKey={api_key}&v=1.16.1&c=golden&f=json"
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lookalike.status(), StatusCode::OK);
    assert_eq!(
        json_body(lookalike).await["subsonic-response"]["error"]["code"],
        10
    );

    let cases = [
        ("getLicense", String::new()),
        ("getOpenSubsonicExtensions", String::new()),
        ("tokenInfo", String::new()),
        ("getBookmarks", String::new()),
        ("getIndexes", format!("&musicFolderId={library}")),
        ("getArtists", format!("&musicFolderId={library}")),
        ("getArtist", format!("&id={artist}")),
        ("getArtistInfo", format!("&id={artist}")),
        ("getArtistInfo2", format!("&id={artist}")),
        ("getAlbumInfo", format!("&id={album}")),
        ("getAlbumInfo2", format!("&id={album}")),
        ("getAlbum", format!("&id={album}")),
        ("getSong", format!("&id={song}")),
        ("getLyrics", "&title=Matrix%20wav".into()),
        ("getLyricsBySongId", format!("&id={song}")),
        ("getGenres", String::new()),
        ("getMusicDirectory", format!("&id={album}")),
        ("getAlbumList", "&type=newest&size=10".into()),
        ("getAlbumList2", "&type=alphabeticalByName&size=10".into()),
        ("getRandomSongs", "&size=10".into()),
        ("getSongsByGenre", "&genre=Electronic&count=10".into()),
        ("search3", "&query=Matrix&songCount=10".into()),
    ];
    for (method, extra) in cases {
        let response = subsonic_json(&router, method, api_key, &extra).await;
        assert_eq!(response["subsonic-response"]["status"], "ok", "{method}");
        if method == "getAlbumList" {
            assert!(response["subsonic-response"]["albumList"].is_object());
            assert!(response["subsonic-response"]["albumList"]["album"].is_array());
            assert!(response["subsonic-response"].get("albumList2").is_none());
        }
        if method == "getOpenSubsonicExtensions" {
            let extensions = &response["subsonic-response"]["openSubsonicExtensions"];
            assert!(
                extensions.is_array(),
                "the extension list is an array whether empty or populated, never an object"
            );
            let advertised = extensions
                .as_array()
                .unwrap()
                .iter()
                .map(|extension| {
                    assert!(
                        extension["versions"].is_array(),
                        "versions must be an array of integers"
                    );
                    extension["name"].as_str().unwrap().to_owned()
                })
                .collect::<Vec<_>>();
            // Advertising an extension the server does not honour is worse than
            // advertising none: the client stops probing and starts relying on
            // it. Each of these is exercised elsewhere in this suite — form
            // POST, apiKey authentication and timeOffset on a transcode.
            assert!(advertised.contains(&"formPost".to_owned()));
            assert!(advertised.contains(&"apiKeyAuthentication".to_owned()));
            assert!(advertised.contains(&"transcodeOffset".to_owned()));
            assert!(advertised.contains(&"songLyrics".to_owned()));
            let song_lyrics = extensions
                .as_array()
                .unwrap()
                .iter()
                .find(|extension| extension["name"] == "songLyrics")
                .unwrap();
            assert_eq!(song_lyrics["versions"], serde_json::json!([1]));
        }
        if method == "getBookmarks" {
            assert!(response["subsonic-response"]["bookmarks"].is_object());
        }
        if method == "getSong" {
            assert_eq!(
                response["subsonic-response"]["song"]["artistId"],
                artist.to_string()
            );
        }
        if method == "getLyrics" {
            let lyrics = &response["subsonic-response"]["lyrics"];
            assert_eq!(lyrics["title"], "Matrix wav");
            assert_eq!(lyrics["value"], "Golden opening\nGolden chorus");
        }
        if method == "getLyricsBySongId" {
            let lyrics = &response["subsonic-response"]["lyricsList"]["structuredLyrics"];
            assert!(lyrics.is_array());
            assert_eq!(lyrics[0]["displayTitle"], "Matrix wav");
            assert_eq!(lyrics[0]["lang"], "xxx");
            assert_eq!(lyrics[0]["synced"], true);
            assert_eq!(lyrics[0]["line"][0]["start"], 1_250);
            assert_eq!(lyrics[0]["line"][0]["value"], "Golden opening");
        }
        if method == "getArtistInfo" || method == "getArtistInfo2" {
            let container = if method == "getArtistInfo" {
                "artistInfo"
            } else {
                "artistInfo2"
            };
            assert_eq!(
                response["subsonic-response"][container],
                serde_json::json!({})
            );
        }
        // Feishin and Symfonium open an album with this call. WaveFlow enriches
        // nothing yet, so the honest answer is the standard empty container —
        // not the code 0 that made the client treat the album as broken.
        if method == "getAlbumInfo" || method == "getAlbumInfo2" {
            let container = if method == "getAlbumInfo" {
                "albumInfo"
            } else {
                "albumInfo2"
            };
            assert_eq!(
                response["subsonic-response"][container],
                serde_json::json!({})
            );
        }
        // The second half of apiKeyAuthentication: a key holder can ask which
        // account it speaks for. The extension is advertised, so this must
        // answer.
        if method == "tokenInfo" {
            assert_eq!(
                response["subsonic-response"]["tokenInfo"]["username"],
                "sub-admin"
            );
        }
    }

    let artist_info = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/getArtistInfo.view?{plain_auth}&id={artist}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(artist_info.status(), StatusCode::OK);
    assert!(body_text(artist_info).await.contains("<artistInfo/>"));

    let song_xml = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/getSong.view?{plain_auth}&id={song}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(song_xml.status(), StatusCode::OK);
    assert!(body_text(song_xml)
        .await
        .contains(&format!("artistId=\"{artist}\"")));

    let lyrics_xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getLyricsBySongId.view?{plain_auth}&id={song}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(lyrics_xml.status(), StatusCode::OK);
    let lyrics_xml = body_text(lyrics_xml).await;
    assert!(lyrics_xml.contains("<lyricsList>"));
    assert!(lyrics_xml.contains("<structuredLyrics"));
    assert!(lyrics_xml.contains("<line start=\"1250\">Golden opening</line>"));

    let native_token = login_token(&router, "sub-admin", web_password).await;
    let native_lyrics = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{song}/lyrics"))
                .header("authorization", format!("Bearer {native_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native_lyrics.status(), StatusCode::OK);
    let native_lyrics = json_body(native_lyrics).await;
    assert_eq!(native_lyrics["trackId"], song.to_string());
    assert_eq!(
        native_lyrics["structuredLyrics"][0]["line"][1]["start"],
        2_500
    );

    let hidden_lyrics = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getLyricsBySongId.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={foreign_song}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden_lyrics.status(), StatusCode::OK);
    let hidden_lyrics = json_body(hidden_lyrics).await;
    assert_eq!(hidden_lyrics["subsonic-response"]["status"], "failed");
    assert_eq!(hidden_lyrics["subsonic-response"]["error"]["code"], 70);
    let hidden_native_lyrics = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{foreign_song}/lyrics"))
                .header("authorization", format!("Bearer {native_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(hidden_native_lyrics.status(), StatusCode::NOT_FOUND);

    let empty_lyrics = subsonic_json(
        &router,
        "getLyricsBySongId",
        api_key,
        &format!("&id={no_artist_song}"),
    )
    .await;
    assert_eq!(
        empty_lyrics["subsonic-response"]["lyricsList"]["structuredLyrics"],
        serde_json::json!([])
    );

    let no_artist_json = subsonic_json(
        &router,
        "getSong",
        api_key,
        &format!("&id={no_artist_song}"),
    )
    .await;
    assert!(no_artist_json["subsonic-response"]["song"]
        .get("artistId")
        .is_none());
    let no_artist_xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getSong.view?{plain_auth}&id={no_artist_song}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(no_artist_xml.status(), StatusCode::OK);
    assert!(!body_text(no_artist_xml).await.contains("artistId="));

    let dsub_artist_info = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getArtistInfo2.view?{plain_auth}&id={artist}&includeNotPresent=true"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(dsub_artist_info.status(), StatusCode::OK);
    let dsub_artist_info = body_text(dsub_artist_info).await;
    assert!(dsub_artist_info.contains("<artistInfo2/>"));

    // XML is the default, and an empty container renders as a self-closing tag
    // there rather than as the `{}` the JSON branch produces. Album pages open on
    // this call, so both encodings are asserted.
    for (method, container) in [
        ("getAlbumInfo", "<albumInfo/>"),
        ("getAlbumInfo2", "<albumInfo2/>"),
    ] {
        let empty_album_info = router
            .clone()
            .oneshot(
                Request::get(format!("/rest/{method}.view?{plain_auth}&id={album}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(empty_album_info.status(), StatusCode::OK, "{method}");
        let empty_album_info = body_text(empty_album_info).await;
        assert!(
            empty_album_info.starts_with("<subsonic-response"),
            "{method}"
        );
        assert!(empty_album_info.contains(container), "{method}");
    }

    for (method, id) in [
        ("getArtistInfo", foreign_artist),
        ("getArtistInfo2", foreign_artist),
        ("getArtistInfo2", Uuid::nil()),
        ("getAlbumInfo", foreign_album),
        ("getAlbumInfo", Uuid::nil()),
        ("getAlbumInfo2", foreign_album),
        ("getAlbumInfo2", Uuid::nil()),
    ] {
        let hidden_artist_info = router
            .clone()
            .oneshot(
                Request::get(format!(
                    "/rest/{method}.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={id}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(hidden_artist_info.status(), StatusCode::OK);
        let hidden_artist_info = json_body(hidden_artist_info).await;
        assert_eq!(hidden_artist_info["subsonic-response"]["status"], "failed");
        assert_eq!(hidden_artist_info["subsonic-response"]["error"]["code"], 70);
    }
    let match_all_search = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=%22%22&artistCount=500&albumCount=500&songCount=500",
    )
    .await;
    let match_all = &match_all_search["subsonic-response"]["searchResult3"];
    assert!(!match_all["artist"].as_array().unwrap().is_empty());
    assert!(!match_all["album"].as_array().unwrap().is_empty());
    assert!(!match_all["song"].as_array().unwrap().is_empty());

    let exhausted_search = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Matrix&artistCount=0&albumCount=0&songCount=1&songOffset=1",
    )
    .await;
    assert!(exhausted_search["subsonic-response"]["searchResult3"]
        .get("song")
        .is_none());
    let repeated_folder_search = subsonic_json(
        &router,
        "search3",
        api_key,
        &format!(
            "&query=Matrix&artistCount=0&albumCount=0&songCount=10&musicFolderId={secondary_library}&musicFolderId={library}"
        ),
    )
    .await;
    assert_eq!(
        repeated_folder_search["subsonic-response"]["searchResult3"]["song"][0]["id"],
        song.to_string()
    );
    let secondary_only_search = subsonic_json(
        &router,
        "search3",
        api_key,
        &format!(
            "&query=Matrix&artistCount=0&albumCount=0&songCount=10&musicFolderId={secondary_library}"
        ),
    )
    .await;
    assert!(secondary_only_search["subsonic-response"]["searchResult3"]
        .get("song")
        .is_none());

    let created = subsonic_json(
        &router,
        "createPlaylist",
        api_key,
        &format!("&name=Golden&songId={song}"),
    )
    .await;
    let playlist = created["subsonic-response"]["playlist"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    // Feishin decides whether a playlist is editable from `owner`. Playlist
    // reads are already scoped to their owner, so the empty string this used to
    // emit made every playlist look like someone else's.
    assert_eq!(
        created["subsonic-response"]["playlist"]["owner"],
        "sub-admin"
    );
    // Given a playlistId, songId names every song of the playlist. A client
    // that removes a song sends back what remains, so treating those ids as
    // additions left the removed song in place and the edit looked lost.
    let replaced = subsonic_json(
        &router,
        "createPlaylist",
        api_key,
        &format!("&playlistId={playlist}&songId={no_artist_song}"),
    )
    .await;
    assert_eq!(replaced["subsonic-response"]["playlist"]["songCount"], 1);
    let reread = subsonic_json(&router, "getPlaylist", api_key, &format!("&id={playlist}")).await;
    let entries = reread["subsonic-response"]["playlist"]["entry"]
        .as_array()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], no_artist_song.to_string());
    // Put the original song back, so the checks below see the playlist they
    // were written against.
    subsonic_json(
        &router,
        "createPlaylist",
        api_key,
        &format!("&playlistId={playlist}&songId={song}"),
    )
    .await;
    for (method, extra) in [
        ("getPlaylists", String::new()),
        ("getPlaylist", format!("&id={playlist}")),
        (
            "updatePlaylist",
            format!("&playlistId={playlist}&comment=Updated"),
        ),
        (
            "star",
            format!("&id={song}&albumId={album}&artistId={artist}"),
        ),
        ("getStarred2", String::new()),
        ("setRating", format!("&id={song}&rating=5")),
        ("scrobble", format!("&id={song}&submission=false")),
        ("getNowPlaying", String::new()),
        (
            "savePlayQueue",
            format!("&id={song}&current={song}&position=25"),
        ),
        ("getPlayQueue", String::new()),
    ] {
        let response = subsonic_json(&router, method, api_key, &extra).await;
        assert_eq!(response["subsonic-response"]["status"], "ok", "{method}");
        if method == "getPlaylists" {
            assert_eq!(
                response["subsonic-response"]["playlists"]["playlist"][0]["owner"],
                "sub-admin"
            );
        }
        if method == "getPlaylist" {
            assert_eq!(
                response["subsonic-response"]["playlist"]["owner"],
                "sub-admin"
            );
        }
    }

    let decorated_song = subsonic_json(&router, "getSong", api_key, &format!("&id={song}")).await;
    assert_eq!(decorated_song["subsonic-response"]["song"]["userRating"], 5);
    assert!(decorated_song["subsonic-response"]["song"]["starred"]
        .as_str()
        .is_some());
    let decorated_search = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Matrix&artistCount=0&albumCount=0&songCount=10",
    )
    .await;
    assert_eq!(
        decorated_search["subsonic-response"]["searchResult3"]["song"][0]["userRating"],
        5
    );
    assert!(
        decorated_search["subsonic-response"]["searchResult3"]["song"][0]["starred"]
            .as_str()
            .is_some()
    );

    // DSub 5.5.3 sends albums and artists through the generic `id`
    // parameter instead of the newer albumId/artistId parameters.
    for id in [album, artist] {
        assert_eq!(
            subsonic_json(&router, "unstar", api_key, &format!("&id={id}")).await
                ["subsonic-response"]["status"],
            "ok"
        );
        assert_eq!(
            subsonic_json(&router, "star", api_key, &format!("&id={id}")).await
                ["subsonic-response"]["status"],
            "ok"
        );
    }
    assert_eq!(
        subsonic_json(
            &router,
            "setRating",
            api_key,
            &format!("&id={album}&rating=4")
        )
        .await["subsonic-response"]["status"],
        "ok"
    );
    assert_eq!(
        subsonic_json(
            &router,
            "scrobble",
            api_key,
            &format!("&id={song}&submission=true")
        )
        .await["subsonic-response"]["status"],
        "ok"
    );
    for (kind, extra) in [
        ("highest", String::new()),
        ("frequent", String::new()),
        ("recent", String::new()),
        ("starred", String::new()),
        ("byGenre", "&genre=Electronic".to_owned()),
    ] {
        let response = subsonic_json(
            &router,
            "getAlbumList2",
            api_key,
            &format!("&type={kind}{extra}"),
        )
        .await;
        assert_eq!(
            response["subsonic-response"]["albumList2"]["album"][0]["id"],
            album.to_string(),
            "album list type {kind}"
        );
    }

    let share = subsonic_json(
        &router,
        "createShare",
        api_key,
        &format!("&id={song}&description=Golden"),
    )
    .await;
    let share_id = share["subsonic-response"]["shares"]["share"][0]["id"]
        .as_str()
        .unwrap();
    // Same reasoning as playlist.owner: a share is read by its owner, so the
    // empty username told the client the share belonged to nobody.
    assert_eq!(
        share["subsonic-response"]["shares"]["share"][0]["username"],
        "sub-admin"
    );
    let share_url = share["subsonic-response"]["shares"]["share"][0]["url"]
        .as_str()
        .unwrap();
    assert!(share_url.starts_with("http://waveflow.test/share/"));
    let public = router
        .clone()
        .oneshot(Request::get(share_url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(public.status(), StatusCode::OK);
    assert_eq!(public.headers()["cache-control"], "no-store");
    let public = json_body(public).await;
    assert_eq!(public["tracks"][0]["id"], song.to_string());
    let public_stream_url = public["tracks"][0]["streamUrl"].as_str().unwrap();
    let public_stream = router
        .clone()
        .oneshot(
            Request::get(public_stream_url)
                .header("range", "bytes=0-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_stream.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        public_stream
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        "RIFF"
    );
    let foreign_public_stream = router
        .clone()
        .oneshot(
            Request::get(format!("{share_url}/tracks/{}/stream", Uuid::new_v4(),))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign_public_stream.status(), StatusCode::NOT_FOUND);
    let listed_shares = subsonic_json(&router, "getShares", api_key, "").await;
    assert_eq!(listed_shares["subsonic-response"]["status"], "ok");
    assert!(listed_shares["subsonic-response"]["shares"]["share"][0]
        .get("url")
        .is_none());
    assert_eq!(
        listed_shares["subsonic-response"]["shares"]["share"][0]["username"],
        "sub-admin"
    );
    assert_eq!(
        subsonic_json(
            &router,
            "updateShare",
            api_key,
            &format!("&id={share_id}&description=Changed")
        )
        .await["subsonic-response"]["status"],
        "ok"
    );
    let journal_entities = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT entity_type FROM sync_event WHERE user_id=? ORDER BY entity_type",
    )
    .bind(admin.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert!(journal_entities.contains(&"playlist".to_owned()));
    assert!(journal_entities.contains(&"favorite".to_owned()));
    assert!(journal_entities.contains(&"rating".to_owned()));
    assert!(journal_entities.contains(&"scrobble".to_owned()));
    assert!(journal_entities.contains(&"queue".to_owned()));
    assert!(journal_entities.contains(&"share".to_owned()));

    let default_user = subsonic_json(
        &router,
        "createUser",
        api_key,
        "&username=sub-default&password=default-secret&email=default@example.invalid",
    )
    .await;
    assert!(default_user["subsonic-response"].get("user").is_none());
    let default_user = subsonic_json(&router, "getUser", api_key, "&username=sub-default").await;
    let default_folders = default_user["subsonic-response"]["user"]["folder"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert!(default_folders.contains(&library.to_string()));
    assert!(default_folders.contains(&secondary_library.to_string()));

    let unicode_username = subsonic_json(
        &router,
        "createUser",
        api_key,
        "&username=%C3%A9lodie&password=unicode-user-secret&email=unicode@example.invalid",
    )
    .await;
    assert_eq!(unicode_username["subsonic-response"]["status"], "ok");
    assert!(state
        .db
        .account_by_username("élodie")
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        subsonic_json(&router, "deleteUser", api_key, "&username=%C3%A9lodie").await
            ["subsonic-response"]["status"],
        "ok"
    );
    assert!(state
        .db
        .account_by_username("élodie")
        .await
        .unwrap()
        .is_none());

    assert_eq!(
        subsonic_json(&router, "deleteUser", api_key, "&username=sub-default").await
            ["subsonic-response"]["status"],
        "ok"
    );

    assert_eq!(
        subsonic_json(&router, "getUsers", api_key, "").await["subsonic-response"]["status"],
        "ok"
    );
    let encoded_listener_password = hex::encode("listener-secret");
    let created_user = subsonic_json(
        &router,
        "createUser",
        api_key,
        &format!(
            "&username=sub-listener&password=enc:{encoded_listener_password}&email=listener@example.invalid&adminRole=false&musicFolderId={library}"
        ),
    )
    .await;
    assert!(created_user["subsonic-response"].get("user").is_none());

    let user = subsonic_json(&router, "getUser", api_key, "&username=sub-listener").await;
    assert_eq!(
        user["subsonic-response"]["user"]["folder"],
        serde_json::json!([library.to_string()])
    );
    let listener_before = state
        .db
        .account_by_username("sub-listener")
        .await
        .unwrap()
        .unwrap();
    let listener_folders = router
        .clone()
        .oneshot(
            Request::get(
                "/rest/getMusicFolders?u=sub-listener&p=listener-secret&v=1.16.1&c=golden&f=json",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listener_folders.status(), StatusCode::OK);
    assert_eq!(
        json_body(listener_folders).await["subsonic-response"]["musicFolders"]["musicFolder"][0]
            ["id"],
        library.to_string()
    );
    let listener_now_playing = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/scrobble?u=sub-listener&p=listener-secret&v=1.16.1&c=golden&f=json&id={song}&submission=false"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listener_now_playing.status(), StatusCode::OK);
    let all_now_playing = subsonic_json(&router, "getNowPlaying", api_key, "").await;
    assert!(all_now_playing["subsonic-response"]["nowPlaying"]["song"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["username"] == "sub-listener"));

    let updated_password = hex::encode("updated-secret");
    let updated_user = subsonic_json(
        &router,
        "updateUser",
        api_key,
        &format!(
            "&username=sub-listener&locked=false&password=enc:{updated_password}&musicFolderId={secondary_library}"
        ),
    )
    .await;
    assert!(updated_user["subsonic-response"].get("user").is_none());
    let listener_folders = router
        .clone()
        .oneshot(
            Request::get(
                "/rest/getMusicFolders?u=sub-listener&p=updated-secret&v=1.16.1&c=golden&f=json",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listener_folders.status(), StatusCode::OK);
    assert_eq!(
        json_body(listener_folders).await["subsonic-response"]["musicFolders"]["musicFolder"][0]
            ["id"],
        secondary_library.to_string()
    );

    let denied_admin = router
        .clone()
        .oneshot(
            Request::get("/rest/getUsers?u=sub-listener&p=updated-secret&v=1.16.1&c=golden&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied_admin.status(), StatusCode::OK);
    assert_eq!(
        json_body(denied_admin).await["subsonic-response"]["error"]["code"],
        50
    );

    let changed_password = hex::encode("changed-secret");
    assert_eq!(
        subsonic_json(
            &router,
            "changePassword",
            api_key,
            &format!("&username=sub-listener&password=enc:{changed_password}")
        )
        .await["subsonic-response"]["status"],
        "ok"
    );
    let listener_after = state
        .db
        .account_by_username("sub-listener")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(listener_before.password_hash, listener_after.password_hash);
    assert_eq!(
        subsonic_json(&router, "deleteUser", api_key, "&username=sub-listener").await
            ["subsonic-response"]["status"],
        "ok"
    );

    let cover = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getCoverArt?apiKey={api_key}&id={artwork}&v=1.16.1&c=golden"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cover.status(), StatusCode::OK);
    assert!(cover.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("image/"));

    // The same cover over the native API. Deliberately authenticated with a
    // native session rather than the Subsonic key above: the whole point is
    // that a native client needs no second set of credentials. Without this
    // route a remote catalogue rendered with no covers at all — payloads carry
    // `artwork_hash`, and only the Subsonic facade could resolve it.
    let session = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "sub-admin",
                "password": web_password,
                "device_name": "artwork-probe"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(session.status(), StatusCode::OK);
    let native_token = json_body(session).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let native_cover = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/artwork/{artwork}"))
                .header("authorization", format!("Bearer {native_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native_cover.status(), StatusCode::OK);
    assert!(native_cover.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("image/"));
    // An entity id resolves too, so a client holding only a song need not first
    // read its hash.
    let by_song = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/artwork/{song}"))
                .header("authorization", format!("Bearer {native_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(by_song.status(), StatusCode::OK);
    // No bearer, no cover: the image is not public just because the hash is
    // unguessable.
    let anonymous_cover = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/artwork/{artwork}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous_cover.status(), StatusCode::UNAUTHORIZED);

    let download = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/download?apiKey={api_key}&id={song}&v=1.16.1&c=golden"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(download.status(), StatusCode::OK);
    assert_eq!(download.headers()["content-disposition"], "attachment");
    assert!(
        download
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len()
            > 44
    );

    let source_bitrate = snapshot.songs.first().unwrap().bitrate.unwrap() as u32;
    let unlimited_stream = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={api_key}&id={song}&maxBitRate=0&v=1.16.1&c=golden"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unlimited_stream.status(), StatusCode::OK);
    assert_eq!(unlimited_stream.headers()["content-type"], "audio/wav");
    let ranged_stream = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={api_key}&id={song}&maxBitRate={source_bitrate}&v=1.16.1&c=golden"
            ))
            .header("range", "bytes=0-3")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ranged_stream.status(), StatusCode::PARTIAL_CONTENT);
    assert!(ranged_stream.headers()["content-range"]
        .to_str()
        .unwrap()
        .starts_with("bytes 0-3/"));
    assert_eq!(
        ranged_stream
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes(),
        "RIFF"
    );

    let invalid_range = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/download.view?apiKey={api_key}&id={song}&v=1.16.1&c=golden"
            ))
            .header("range", "bytes=999999-")
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid_range.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert!(invalid_range.headers()["content-range"]
        .to_str()
        .unwrap()
        .starts_with("bytes */"));

    let direct_stream = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={api_key}&id={song}&maxBitRate={source_bitrate}&v=1.16.1&c=DSub"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(direct_stream.status(), StatusCode::OK);
    assert_eq!(direct_stream.headers()["content-type"], "audio/wav");
    assert!(direct_stream
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .starts_with(b"RIFF"));

    let transcoded_stream = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/stream.view?apiKey={api_key}&id={song}&maxBitRate=32&v=1.16.1&c=DSub"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(transcoded_stream.status(), StatusCode::OK);
    assert_eq!(transcoded_stream.headers()["content-type"], "audio/mpeg");
    assert!(
        transcoded_stream
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .len()
            > 32
    );

    for (method, extra) in [
        ("deleteShare", format!("&id={share_id}")),
        ("deletePlaylist", format!("&id={playlist}")),
    ] {
        assert_eq!(
            subsonic_json(&router, method, api_key, &extra).await["subsonic-response"]["status"],
            "ok"
        );
    }

    let wrong = router
        .oneshot(
            Request::get("/rest/ping?u=sub-admin&p=wrong&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::OK);
    assert_eq!(
        json_body(wrong).await["subsonic-response"]["error"]["code"],
        40
    );
}

#[tokio::test]
async fn sqlite_and_instance_key_backup_restore_as_one_consistent_bundle() {
    let (_temp, config, state) = test_app().await;
    let password = "backup-web-password";
    let user = state
        .db
        .create_account(
            "backup-admin",
            &security::hash_password(password).unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state.secret_box.encrypt(b"backup-subsonic-secret").unwrap();
    state
        .db
        .set_subsonic_credential(
            user,
            user,
            &encrypted,
            &security::token_hash("wfsk_backup"),
            now_ms(),
        )
        .await
        .unwrap();
    let bundle = config.data_dir.with_file_name("backup-bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    state
        .db
        .backup_to(&bundle.join("waveflow.db"))
        .await
        .unwrap();
    std::fs::copy(&config.instance_key_path, bundle.join("instance.key")).unwrap();
    assert!(
        waveflow_server::database::Database::check_file(&bundle.join("waveflow.db"))
            .await
            .unwrap()
    );
    state.db.pool().close().await;
    drop(state);

    let mismatched_bundle = config.data_dir.with_file_name("mismatched-backup-bundle");
    std::fs::create_dir_all(&mismatched_bundle).unwrap();
    std::fs::copy(
        bundle.join("waveflow.db"),
        mismatched_bundle.join("waveflow.db"),
    )
    .unwrap();
    std::fs::write(mismatched_bundle.join("instance.key"), [9u8; 32]).unwrap();
    let database_before = std::fs::read(&config.database_path).unwrap();
    let key_before = std::fs::read(&config.instance_key_path).unwrap();
    let error = waveflow_server::cli::restore(
        &config,
        waveflow_server::cli::RestoreArgs {
            input: mismatched_bundle,
        },
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("do not match"));
    assert_eq!(
        std::fs::read(&config.database_path).unwrap(),
        database_before
    );
    assert_eq!(
        std::fs::read(&config.instance_key_path).unwrap(),
        key_before
    );

    waveflow_server::cli::restore(
        &config,
        waveflow_server::cli::RestoreArgs {
            input: bundle.clone(),
        },
    )
    .await
    .unwrap();
    let restored = waveflow_server::initialize(&config).await.unwrap();
    let credential = restored
        .services
        .credential_by_username("backup-admin")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        restored
            .services
            .decrypt_subsonic_password(&credential)
            .unwrap(),
        b"backup-subsonic-secret"
    );
    assert!(restored.db.integrity_check().await.unwrap());
}

#[tokio::test]
async fn subsonic_blurs_foreign_catalog_and_rate_limits_failed_authentication() {
    let (_temp, config, state) = test_app().await;
    let web_hash = security::hash_password("web-password-for-test").unwrap();
    let owner = state
        .db
        .create_account("sub-owner", &web_hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("sub-outsider", &web_hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    // A dedicated account for the throttling assertion: the rate window is a
    // process-wide map keyed by the supplied username, so reusing one of the
    // accounts asserted on elsewhere would leak between tests.
    let throttled = state
        .db
        .create_account("sub-throttled", &web_hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    for (actor, user, password, key) in [
        (owner, owner, "owner-sub-password", "wfsk_owner"),
        (owner, outsider, "outsider-sub-password", "wfsk_outsider"),
        (owner, throttled, "throttled-sub-password", "wfsk_throttled"),
    ] {
        state
            .db
            .set_subsonic_credential(
                actor,
                user,
                &state.secret_box.encrypt(password.as_bytes()).unwrap(),
                &security::token_hash(key),
                now_ms(),
            )
            .await
            .unwrap();
    }
    let music = config.data_dir.join("isolated-subsonic");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Private.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Private Subsonic",
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
            name: "Private Subsonic".into(),
            root_path: root,
        },
    )
    .await;
    let song = state.db.list_tracks_for_user(owner, library).await.unwrap()[0].id;
    let router = waveflow_server::app(&config, state);
    let foreign = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getSong?apiKey=wfsk_outsider&id={song}&f=json"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::OK);
    assert_eq!(
        json_body(foreign).await["subsonic-response"]["error"]["code"],
        70
    );

    let foreign_star = router
        .clone()
        .oneshot(
            Request::get(format!("/rest/star?apiKey=wfsk_outsider&id={song}&f=json"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign_star.status(), StatusCode::OK);
    assert_eq!(
        json_body(foreign_star).await["subsonic-response"]["error"]["code"],
        70
    );

    // Repeated failures throttle the credential. That refusal is no longer
    // visible in the status line — every Subsonic answer is HTTP 200 — so the
    // limiter is asserted where it actually bites: once the window is full,
    // even the correct password is refused.
    for _ in 0..=20 {
        let refused = router
            .clone()
            .oneshot(
                Request::get("/rest/ping?u=sub-throttled&p=wrong&f=json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(refused.status(), StatusCode::OK);
        assert_eq!(
            json_body(refused).await["subsonic-response"]["error"]["code"],
            40
        );
    }
    let throttled_but_correct = router
        .clone()
        .oneshot(
            Request::get("/rest/ping?u=sub-throttled&p=throttled-sub-password&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(throttled_but_correct.status(), StatusCode::OK);
    assert_eq!(
        json_body(throttled_but_correct).await["subsonic-response"]["error"]["code"],
        40
    );

    let unknown = router
        .clone()
        .oneshot(
            Request::get("/rest/ping?u=unknown-enum&p=wrong&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let wrong = router
        .oneshot(
            Request::get("/rest/ping?u=sub-outsider&p=wrong&f=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unknown.status(), wrong.status());
    assert_eq!(body_text(unknown).await, body_text(wrong).await);
}

fn catalog_input(index: usize, artist: &str) -> CatalogTrackInput {
    CatalogTrackInput {
        relative_path: format!("track-{index}.flac"),
        file_size: 1024 + index as i64,
        modified_at: 1_700_000_000_000 + index as i64,
        quick_hash: format!("{:064x}", index + 1),
        full_hash: format!("{:064x}", index + 101),
        title: format!("Compilation track {index}"),
        artist: Some(artist.into()),
        artists: Vec::new(),
        album_artists: Vec::new(),
        roles: Vec::new(),
        performer_pairs: Vec::new(),
        album: Some("Shared compilation".into()),
        album_artist: None,
        is_compilation: true,
        genre: Some("Rock; Pop".into()),
        year: Some(2026),
        track_number: Some(index as i64 + 1),
        disc_number: Some(1),
        duration_ms: 180_000,
        bitrate: Some(1_000),
        sample_rate: Some(48_000),
        channels: Some(2),
        bit_depth: Some(24),
        codec: Some("FLAC".into()),
        musical_key: None,
        tag_rating: None,
        musicbrainz_recording_id: None,
        musicbrainz_release_id: None,
        musicbrainz_artist_id: None,
        replay_gain_track_gain: None,
        replay_gain_track_peak: None,
        replay_gain_album_gain: None,
        replay_gain_album_peak: None,
        bpm: None,
        sort_title: None,
        sort_album: None,
        sort_album_artist: None,
        sort_artist: None,
        comment: None,
        isrc: None,
        moods: None,
        explicit_status: None,
        artwork: None,
        lyrics_hash: blake3::hash(b"").to_hex().to_string(),
        lyrics: Vec::new(),
    }
}

async fn run_scan(state: &waveflow_server::AppState, owner: uuid::Uuid, library: LibraryRecord) {
    let id = state
        .scanner
        .trigger(library, Some(owner), "manual")
        .await
        .unwrap();
    for _ in 0..200 {
        let job = state
            .db
            .scan_job_for_user(owner, id)
            .await
            .unwrap()
            .unwrap();
        if job.status == "completed" {
            return;
        }
        if job.status == "failed" {
            panic!("scan failed: {:?}", job.message);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("scan timed out");
}

fn write_test_wav(path: &std::path::Path) {
    let sample_rate = 8_000u32;
    let samples = vec![0i16; 800];
    let data_len = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&sample_rate.to_le_bytes());
    bytes.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

fn write_test_dsf(path: &std::path::Path) {
    let samples_per_channel = 32_768u64;
    let channels = 2u32;
    let rate = 2_822_400u32;
    let payload_bytes = (samples_per_channel / 8) * channels as u64;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"DSD ");
    bytes.extend_from_slice(&28u64.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(&0u64.to_le_bytes());
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&52u64.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&channels.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&samples_per_channel.to_le_bytes());
    bytes.extend_from_slice(&4096u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(payload_bytes + 12).to_le_bytes());
    bytes.extend(std::iter::repeat_n(0xAA, payload_bytes as usize));
    std::fs::write(path, bytes).unwrap();
}

fn write_test_png(path: &std::path::Path) {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z4m8AAAAASUVORK5CYII=")
        .unwrap();
    std::fs::write(path, bytes).unwrap();
}

async fn wait_for_cache_file(dir: &std::path::Path, extension: &str) -> std::path::PathBuf {
    for _ in 0..100 {
        if let Some(path) = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| path.extension().and_then(std::ffi::OsStr::to_str) == Some(extension))
        {
            return path;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("transcode cache file was not committed")
}

fn generate_audio_fixture(path: &std::path::Path, codec: &str, extension: &str) {
    let output = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.15",
            "-metadata",
            &format!("title=Matrix {extension}"),
            "-metadata",
            "artist=Alpha; Beta",
            "-metadata",
            "album=WaveFlow format matrix",
            "-metadata",
            "album_artist=Matrix Artist",
            "-metadata",
            "genre=Electronic; Test",
            "-c:a",
            codec,
        ])
        .arg(path)
        .output()
        .unwrap_or_else(|error| panic!("FFmpeg is required for the format matrix: {error}"));
    assert!(
        output.status.success(),
        "FFmpeg failed for {extension}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

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

fn json_request(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::post(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

async fn body_text(response: axum::response::Response) -> String {
    String::from_utf8(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

async fn subsonic_json(
    router: &axum::Router,
    method: &str,
    api_key: &str,
    extra: &str,
) -> serde_json::Value {
    let response = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/{method}.view?apiKey={api_key}&v=1.16.1&c=golden&f=json{extra}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = json_body(response).await;
    assert!(status.is_success(), "{method} returned {status}: {body}");
    body
}

async fn login_token(router: &axum::Router, username: &str, password: &str) -> String {
    let response = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": username,
                "password": password,
                "device_name": "Route test"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await["access_token"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The ten Subsonic album-list modes and their native equivalents resolve
/// through one SQL implementation, so both surfaces agree by construction and
/// neither loads the catalogue to sort it.
#[tokio::test]
async fn album_discovery_orders_and_filters_in_sql_for_both_surfaces() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let api_key = "wfsk_discovery-key";
    let owner = state
        .db
        .create_account("discovery-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"discovery-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("discovery-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Discovery",
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
    state.db.start_scan_job(scan, 5, false).await.unwrap();
    // "delta moon" is lowercase on purpose: a byte-wise sort would file it after
    // "Gamma Sun", and album order is documented as case-insensitive.
    for (index, (title, album, artist, genre, year)) in [
        ("Tidewater", "Alpha Sea", "Zed Waves", "Rock", 1999),
        ("Undertow", "Alpha Sea", "Zed Waves", "Rock", 1999),
        ("Cirrus", "Beta Sky", "Aria Lux", "Jazz; Rock", 2010),
        ("Corona", "Gamma Sun", "Mono Field", "Jazz", 2024),
        ("Waning", "delta moon", "Beta Person", "Hip-Hop", 2005),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(index + 40, title, album, artist, Some(1), Some(1));
        input.genre = Some(genre.into());
        input.year = Some(year);
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    // Albums created inside one scan share a millisecond, and the tie-break is a
    // random UUID. Pinning created_at is what makes `newest` assertable at all.
    for (title, created_at) in [
        ("Alpha Sea", 1_000_i64),
        ("Beta Sky", 2_000),
        ("Gamma Sun", 3_000),
        ("delta moon", 4_000),
    ] {
        sqlx::query("UPDATE album SET created_at = ? WHERE title = ?")
            .bind(created_at)
            .bind(title)
            .execute(state.db.pool())
            .await
            .unwrap();
    }

    let albums = state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap();
    let album_id = |title: &str| {
        albums
            .iter()
            .find(|album| album.title == title)
            .unwrap_or_else(|| panic!("{title} was indexed"))
            .id
    };

    state
        .services
        .set_rating(owner, "album", album_id("Gamma Sun"), 5)
        .await
        .unwrap();
    state
        .services
        .set_rating(owner, "album", album_id("Alpha Sea"), 3)
        .await
        .unwrap();
    state
        .services
        .set_star(owner, "album", album_id("Beta Sky"), true)
        .await
        .unwrap();
    let gamma_track = state
        .services
        .album(owner, album_id("Gamma Sun"))
        .await
        .unwrap()
        .songs[0]
        .id;
    let alpha_track = state
        .services
        .album(owner, album_id("Alpha Sea"))
        .await
        .unwrap()
        .songs[0]
        .id;
    for time in [1_000_i64, 2_000, 3_000] {
        state
            .services
            .scrobble(owner, gamma_track, true, Some(time))
            .await
            .unwrap();
    }
    // Played once but most recently: `frequent` and `recent` must not agree.
    state
        .services
        .scrobble(owner, alpha_track, true, Some(9_000))
        .await
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let token = login_token(&router, "discovery-owner", password).await;

    let subsonic_titles = |kind: String| {
        let router = router.clone();
        async move {
            let response = subsonic_json(&router, "getAlbumList2", api_key, &kind).await;
            response["subsonic-response"]["albumList2"]["album"]
                .as_array()
                .unwrap()
                .iter()
                .map(|album| album["name"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };
    let native_titles = |query: String| {
        let router = router.clone();
        let token = token.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(format!("/api/v2/albums?{query}"))
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{query}");
            json_body(response)
                .await
                .as_array()
                .unwrap()
                .iter()
                .map(|album| album["title"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };

    for (kind, native, expected) in [
        (
            "&type=alphabeticalByName&size=500",
            "sort=alphabeticalByName&limit=500",
            vec!["Alpha Sea", "Beta Sky", "delta moon", "Gamma Sun"],
        ),
        (
            "&type=alphabeticalByArtist&size=500",
            "sort=alphabeticalByArtist&limit=500",
            vec!["Beta Sky", "delta moon", "Gamma Sun", "Alpha Sea"],
        ),
        (
            "&type=newest&size=500",
            "sort=newest&limit=500",
            vec!["delta moon", "Gamma Sun", "Beta Sky", "Alpha Sea"],
        ),
        (
            "&type=highest&size=500",
            "sort=highest&limit=500",
            vec!["Gamma Sun", "Alpha Sea"],
        ),
        (
            "&type=frequent&size=500",
            "sort=frequent&limit=500",
            vec!["Gamma Sun", "Alpha Sea"],
        ),
        (
            "&type=recent&size=500",
            "sort=recent&limit=500",
            vec!["Alpha Sea", "Gamma Sun"],
        ),
        (
            "&type=starred&size=500",
            "sort=starred&limit=500",
            vec!["Beta Sky"],
        ),
        (
            "&type=byYear&fromYear=2000&toYear=2020&size=500",
            "sort=byYear&from_year=2000&to_year=2020&limit=500",
            vec!["delta moon", "Beta Sky"],
        ),
        (
            // A reversed range is how Subsonic asks for descending years.
            "&type=byYear&fromYear=2020&toYear=2000&size=500",
            "sort=byYear&from_year=2020&to_year=2000&limit=500",
            vec!["Beta Sky", "delta moon"],
        ),
        (
            "&type=byGenre&genre=Rock&size=500",
            "sort=byGenre&genre=Rock&limit=500",
            vec!["Alpha Sea", "Beta Sky"],
        ),
        (
            // Genre matching is on the canonical form, so punctuation and case
            // no longer split one genre in two.
            "&type=byGenre&genre=hip%20hop&size=500",
            "sort=byGenre&genre=hip+hop&limit=500",
            vec!["delta moon"],
        ),
    ] {
        assert_eq!(subsonic_titles(kind.into()).await, expected, "{kind}");
        assert_eq!(native_titles(native.into()).await, expected, "{native}");
    }

    // `random` draws from the same set as every other ordering. Its page
    // contents cannot be asserted: SQLite reshuffles per statement, so two
    // requests are two independent draws and a title may repeat or be missed
    // across them. What must hold is membership — no ordering may surface an
    // album the account cannot see.
    let catalogue = vec!["Alpha Sea", "Beta Sky", "Gamma Sun", "delta moon"];
    let mut shuffled = subsonic_titles("&type=random&size=500".into()).await;
    shuffled.sort();
    assert_eq!(shuffled, catalogue);
    for offset in [0, 2] {
        let page = subsonic_titles(format!("&type=random&size=2&offset={offset}")).await;
        assert!(page.len() <= 2, "offset {offset} returned {page:?}");
        for title in &page {
            assert!(
                catalogue.contains(&title.as_str()),
                "offset {offset} returned {title}"
            );
        }
    }

    // Paging happens in SQL now; the second page of an ordered list is exact.
    // Both surfaces are asserted because they reach `page` by different routes:
    // Subsonic clamps `size` before building it, the native handler maps
    // `offset`/`limit` straight onto `BrowsePage::new`.
    assert_eq!(
        subsonic_titles("&type=alphabeticalByName&size=2&offset=2".into()).await,
        vec!["delta moon", "Gamma Sun"]
    );
    assert_eq!(
        native_titles("sort=alphabeticalByName&limit=2&offset=2".into()).await,
        vec!["delta moon", "Gamma Sun"]
    );
    assert_eq!(
        native_titles("sort=newest&limit=1&offset=1".into()).await,
        vec!["Gamma Sun"]
    );

    // An empty page is where the two surfaces deliberately diverge. Subsonic
    // answered `size=0` with an empty container long before this change and
    // still does, while the native contract is `1 <= limit <= 500` and rejects
    // the bound like it rejects 501.
    let empty_page = subsonic_json(&router, "getAlbumList2", api_key, "&type=newest&size=0").await;
    let empty_page = &empty_page["subsonic-response"]["albumList2"];
    assert!(empty_page.is_object());
    assert!(empty_page.get("album").is_none());
    for query in ["sort=newest&limit=0", "sort=newest&limit=501"] {
        let out_of_bounds = router
            .clone()
            .oneshot(
                Request::get(format!("/api/v2/albums?{query}"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            out_of_bounds.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "{query}"
        );
    }

    // An unknown ordering is refused on both surfaces rather than silently
    // falling back to the default.
    let rejected = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbumList2.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&type=nope"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(rejected).await["subsonic-response"]["error"]["code"],
        10
    );
    let rejected_native = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums?sort=nope")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected_native.status(), StatusCode::UNPROCESSABLE_ENTITY);
    // byGenre without a genre would silently drop the filter if it were not
    // refused, so both surfaces reject it.
    let missing_genre = router
        .clone()
        .oneshot(
            Request::get("/api/v2/albums?sort=byGenre")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_genre.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let missing_genre_subsonic = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbumList2.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&type=byGenre"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_genre_subsonic.status(), StatusCode::OK);
    assert_eq!(
        json_body(missing_genre_subsonic).await["subsonic-response"]["error"]["code"],
        10
    );

    // songCount and duration describe the album, not the tracks the caller
    // happened to load. "Tidewater" matches one of Alpha Sea's two tracks.
    let hit = subsonic_json(&router, "search3", api_key, "&query=Tidewater").await;
    let matched = &hit["subsonic-response"]["searchResult3"]["album"][0];
    assert_eq!(matched["name"], "Alpha Sea");
    assert_eq!(matched["songCount"], 2);
    assert_eq!(matched["duration"], 240);

    // Genres are counted once per canonical name across every visible library,
    // on both surfaces.
    let genres = subsonic_json(&router, "getGenres", api_key, "").await;
    let genres = genres["subsonic-response"]["genres"]["genre"]
        .as_array()
        .unwrap()
        .iter()
        .map(|genre| {
            (
                genre["value"].as_str().unwrap().to_owned(),
                genre["songCount"].as_i64().unwrap(),
                genre["albumCount"].as_i64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        genres,
        vec![
            ("Hip-Hop".to_owned(), 1, 1),
            ("Jazz".to_owned(), 2, 2),
            ("Rock".to_owned(), 3, 2),
        ]
    );
    let native_genres = router
        .clone()
        .oneshot(
            Request::get("/api/v2/genres")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native_genres.status(), StatusCode::OK);
    let native_genres = json_body(native_genres).await;
    assert_eq!(
        native_genres
            .as_array()
            .unwrap()
            .iter()
            .map(|genre| (
                genre["name"].as_str().unwrap().to_owned(),
                genre["song_count"].as_i64().unwrap(),
                genre["album_count"].as_i64().unwrap()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("Hip-Hop".to_owned(), 1, 1),
            ("Jazz".to_owned(), 2, 2),
            ("Rock".to_owned(), 3, 2),
        ]
    );
}

/// OpenSubsonic reports support for a field by emitting it even when the value
/// is unknown, so a track with nothing tagged is as much a test of the contract
/// as a fully tagged one — in both encodings, since XML and JSON build the
/// arrays through different code paths.
#[tokio::test]
async fn media_items_carry_the_modern_opensubsonic_fields_in_both_encodings() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_fields-key";
    let subsonic_password = "fields-secret";
    let owner = state
        .db
        .create_account(
            "fields-owner",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state
                .secret_box
                .encrypt(subsonic_password.as_bytes())
                .unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("fields-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Fields",
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
    state.db.start_scan_job(scan, 2, false).await.unwrap();

    // Two credited artists and two genres, so tag order and the split are both
    // observable. The album is a compilation, which is stored and was never
    // surfaced before.
    let mut tagged = browse_input(80, "Tagged", "Sun Bloom", "Aria Lux", Some(1), Some(1));
    tagged.artist = Some("Aria Lux; Mono Field".into());
    tagged.album_artist = Some("Aria Lux".into());
    tagged.genre = Some("Rock; Jazz".into());
    tagged.is_compilation = true;
    tagged.musicbrainz_recording_id = Some("9f4c1d2e-recording".into());
    tagged.musicbrainz_release_id = Some("3a8b7c6d-release".into());
    tagged.musicbrainz_artist_id = Some("1b2c3d4e-artist".into());
    tagged.replay_gain_track_gain = Some(-7.32);
    tagged.replay_gain_track_peak = Some(0.988_525);
    tagged.replay_gain_album_gain = Some(-6.5);
    tagged.replay_gain_album_peak = Some(1.0);
    tagged.bpm = Some(128);
    tagged.sort_title = Some("Tagged, The".into());
    tagged.comment = Some("ripped from vinyl".into());
    tagged.isrc = Some("FRZ039800212; GBAYE0601498".into());
    tagged.moods = Some("Melancholic; Warm".into());
    tagged.explicit_status = Some("clean".into());
    state
        .db
        .apply_catalog_track(library, scan, &tagged, None, false)
        .await
        .unwrap();

    // Nothing tagged and nothing decoded: the case where every added field has
    // to be present with its default rather than omitted.
    let mut bare = browse_input(81, "Bare", "Sun Bloom", "Aria Lux", Some(2), Some(1));
    bare.artist = None;
    bare.album_artist = Some("Aria Lux".into());
    bare.genre = None;
    bare.is_compilation = true;
    bare.sample_rate = None;
    bare.channels = None;
    bare.bit_depth = None;
    state
        .db
        .apply_catalog_track(library, scan, &bare, None, false)
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let songs = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .songs;
    let song_id = |title: &str| {
        songs
            .iter()
            .find(|song| song.title == title)
            .unwrap_or_else(|| panic!("{title} was indexed"))
            .id
    };
    let tagged_id = song_id("Tagged");
    let bare_id = song_id("Bare");
    state
        .services
        .scrobble(owner, tagged_id, true, Some(1_700_000_000_000))
        .await
        .unwrap();

    let router = waveflow_server::app(&config, state.clone());

    let tagged_json = subsonic_json(&router, "getSong", api_key, &format!("&id={tagged_id}")).await;
    let tagged_json = &tagged_json["subsonic-response"]["song"];
    assert_eq!(tagged_json["samplingRate"], 44_100);
    assert_eq!(tagged_json["channelCount"], 2);
    assert_eq!(tagged_json["bitDepth"], 16);
    assert_eq!(tagged_json["mediaType"], "song");
    assert_eq!(tagged_json["isVideo"], false);
    assert_eq!(tagged_json["playCount"], 1);
    assert!(tagged_json["played"].is_string());
    assert_eq!(tagged_json["displayArtist"], "Aria Lux; Mono Field");
    // Tag order, not alphabetical: "Mono Field" is credited second.
    let artists = tagged_json["artists"].as_array().unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0]["name"], "Aria Lux");
    assert_eq!(artists[1]["name"], "Mono Field");
    assert!(artists[0]["id"].is_string());
    // The primary credit still matches the frozen artistId.
    assert_eq!(tagged_json["artistId"], artists[0]["id"]);
    assert_eq!(tagged_json["musicBrainzId"], "9f4c1d2e-recording");
    assert_eq!(tagged_json["bpm"], 128);
    assert_eq!(tagged_json["sortName"], "Tagged, The");
    assert_eq!(tagged_json["comment"], "ripped from vinyl");
    // Multi-valued like artists and genres, and split the same way.
    assert_eq!(
        tagged_json["isrc"],
        serde_json::json!(["FRZ039800212", "GBAYE0601498"])
    );
    assert_eq!(
        tagged_json["replayGain"],
        serde_json::json!({
            "trackGain": -7.32,
            "trackPeak": 0.988_525,
            "albumGain": -6.5,
            "albumPeak": 1.0
        })
    );
    assert_eq!(
        tagged_json["moods"],
        serde_json::json!(["Melancholic", "Warm"])
    );
    assert_eq!(tagged_json["explicitStatus"], "clean");
    // Genres are ordered by name so two identical catalogues answer identically.
    let genres = tagged_json["genres"]
        .as_array()
        .unwrap()
        .iter()
        .map(|genre| genre["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(genres, vec!["Jazz", "Rock"]);

    let bare_json = subsonic_json(&router, "getSong", api_key, &format!("&id={bare_id}")).await;
    let bare_json = &bare_json["subsonic-response"]["song"];
    // Present with their defaults. Omitting them would tell the client this
    // server does not implement the fields at all.
    assert_eq!(bare_json["samplingRate"], 0);
    assert_eq!(bare_json["channelCount"], 0);
    assert_eq!(bare_json["bitDepth"], 0);
    assert_eq!(bare_json["playCount"], 0);
    assert_eq!(bare_json["displayArtist"], "");
    assert_eq!(bare_json["artists"], serde_json::json!([]));
    assert_eq!(bare_json["genres"], serde_json::json!([]));
    assert_eq!(bare_json["musicBrainzId"], "");
    assert_eq!(bare_json["bpm"], 0);
    assert_eq!(bare_json["sortName"], "");
    assert_eq!(bare_json["comment"], "");
    assert_eq!(bare_json["isrc"], serde_json::json!([]));
    assert_eq!(bare_json["moods"], serde_json::json!([]));
    assert_eq!(bare_json["explicitStatus"], "");
    // replayGain is the one addition whose members the specification says to
    // omit when unknown. The container still has to be there: it is what says
    // the server reads gain tags at all.
    assert_eq!(bare_json["replayGain"], serde_json::json!({}));
    // The documented exception: an empty string is not a timestamp, and
    // playCount already signals that play statistics are supported.
    assert!(bare_json.get("played").is_none());

    // XML builds the arrays as repeated child elements rather than as a JSON
    // array, so it is asserted separately rather than assumed.
    let plain_auth = format!("u=fields-owner&p={subsonic_password}&v=1.16.1&c=golden");
    let xml_song = |id: uuid::Uuid| {
        let router = router.clone();
        let plain_auth = plain_auth.clone();
        async move {
            let response = router
                .oneshot(
                    Request::get(format!("/rest/getSong.view?{plain_auth}&id={id}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            body_text(response).await
        }
    };
    let tagged_xml = xml_song(tagged_id).await;
    assert!(tagged_xml.contains("samplingRate=\"44100\""));
    assert!(tagged_xml.contains("channelCount=\"2\""));
    assert!(tagged_xml.contains("bitDepth=\"16\""));
    assert!(tagged_xml.contains("mediaType=\"song\""));
    assert!(tagged_xml.contains("isVideo=\"false\""));
    assert!(tagged_xml.contains("<artists "));
    assert!(tagged_xml.contains("name=\"Mono Field\""));
    assert!(tagged_xml.contains("<genres name=\"Jazz\"/>"));
    assert!(tagged_xml.contains("<genres name=\"Rock\"/>"));
    assert!(tagged_xml.contains("played="));
    assert!(tagged_xml.contains("musicBrainzId=\"9f4c1d2e-recording\""));
    assert!(tagged_xml.contains("bpm=\"128\""));
    assert!(tagged_xml.contains("<isrc>FRZ039800212</isrc>"));
    assert!(tagged_xml.contains("<isrc>GBAYE0601498</isrc>"));
    assert!(tagged_xml.contains("trackGain=\"-7.32\""));
    assert!(tagged_xml.contains("<moods>Melancholic</moods>"));
    assert!(tagged_xml.contains("explicitStatus=\"clean\""));

    let bare_xml = xml_song(bare_id).await;
    assert!(bare_xml.contains("samplingRate=\"0\""));
    assert!(bare_xml.contains("bitDepth=\"0\""));
    assert!(bare_xml.contains("displayArtist=\"\""));
    // An empty array has no repeated element to render, which is exactly why
    // the JSON branch needs its own rule and gets its own assertion above.
    assert!(!bare_xml.contains("<artists "));
    assert!(!bare_xml.contains("<genres "));
    assert!(!bare_xml.contains("played="));
    assert!(bare_xml.contains("musicBrainzId=\"\""));
    assert!(bare_xml.contains("bpm=\"0\""));
    assert!(!bare_xml.contains("<isrc>"));
    // Present but empty, in both encodings.
    assert!(bare_xml.contains("<replayGain/>"));

    // Albums carry their own additions.
    let album_id = state
        .services
        .catalog_snapshot(owner, &[])
        .await
        .unwrap()
        .albums[0]
        .id;
    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={album_id}")).await;
    let album = &album["subsonic-response"]["album"];
    assert_eq!(album["isCompilation"], true);
    assert_eq!(album["playCount"], 1);
    assert_eq!(album["displayArtist"], "Aria Lux");
    assert!(album["played"].is_string());

    // The native surface reads the same projection, so the structured relations
    // reach it without a second implementation.
    let token = login_token(&router, "fields-owner", "correct horse battery staple").await;
    let native = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/tracks/{tagged_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    let native = json_body(native).await;
    assert_eq!(native["sample_rate"], 44_100);
    assert_eq!(native["play_count"], 1);
    assert_eq!(
        native["artists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|artist| artist["name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>(),
        vec!["Aria Lux", "Mono Field"]
    );
    assert_eq!(native["genres"], serde_json::json!(["Jazz", "Rock"]));
    assert_eq!(native["musicbrainz_id"], "9f4c1d2e-recording");
    assert_eq!(native["bpm"], 128);
    assert_eq!(
        native["isrc"],
        serde_json::json!(["FRZ039800212", "GBAYE0601498"])
    );
}

/// The facade could not trigger a rescan the native API has always been able to
/// trigger, and answered a not-implemented error for surfaces clients open by
/// default. Both gaps are closed without inventing data.
#[tokio::test]
async fn facade_controls_scans_and_answers_its_remaining_methods() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_asymmetry-key";
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("asym-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"asym-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("asym-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();

    let seed = |account: Uuid, name: &'static str, titles: Vec<&'static str>| {
        let state = state.clone();
        let root = config.data_dir.join(name);
        async move {
            std::fs::create_dir_all(&root).unwrap();
            let library = state
                .db
                .create_library(
                    account,
                    name,
                    &std::fs::canonicalize(&root).unwrap(),
                    LibraryVisibility::Private,
                    now_ms(),
                )
                .await
                .unwrap();
            let scan = state
                .db
                .create_scan_job(library, Some(account), "manual")
                .await
                .unwrap();
            state
                .db
                .start_scan_job(scan, titles.len() as i64, false)
                .await
                .unwrap();
            for (index, title) in titles.into_iter().enumerate() {
                let mut input = browse_input(
                    index + 120,
                    title,
                    "Asym Album",
                    "Asym Artist",
                    Some(1),
                    Some(1),
                );
                input.relative_path = format!("{name}-{index}.flac");
                input.quick_hash = format!("{:064x}", index + 7_000 + name.len() * 100);
                input.full_hash = format!("{:064x}", index + 8_000 + name.len() * 100);
                state
                    .db
                    .apply_catalog_track(library, scan, &input, None, false)
                    .await
                    .unwrap();
            }
            state.db.consolidate_musicbrainz_ids(library).await.unwrap();
            state.db.finish_scan_job(scan, 0).await.unwrap();
            library
        }
    };
    let library = seed(owner, "asym-own", vec!["One", "Two", "Three"]).await;
    seed(outsider, "asym-foreign", vec!["Hidden"]).await;

    let router = waveflow_server::app(&config, state.clone());

    // Idle, and counting only what this account can reach: the outsider's
    // fourth track must not appear in the owner's total.
    let status = subsonic_json(&router, "getScanStatus", api_key, "").await;
    let status = &status["subsonic-response"]["scanStatus"];
    assert_eq!(status["scanning"], false);
    assert_eq!(status["count"], 3);

    // A membership revoked between the lookup and the queuing must not leave
    // a job behind. The window cannot be interleaved deterministically, so
    // what is asserted is the property that makes it safe: the insert itself
    // requires a library_member row, and a non-member is exactly the state a
    // revocation leaves behind.
    assert!(state
        .db
        .create_scan_job_for_user(outsider, library, "manual")
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        state.services.start_library_scan(outsider, library).await,
        Err(ServiceError::NotFound)
    ));
    // Scoped to the owner's library: the outsider legitimately has jobs against
    // their own.
    let intruder_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scan_job WHERE requested_by = ? AND library_id = ?",
    )
    .bind(outsider.to_string())
    .bind(library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(intruder_jobs, 0, "a non-member queued a scan");

    // Membership is not authority. A listener reads the catalogue; a scan
    // walks the owner's files and takes the writer gate, so it is refused --
    // and refused the way everything unentitled is, indistinguishably from a
    // library that does not exist.
    let listener = state
        .db
        .create_account("asym-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(owner, library, listener, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    // The membership is real: the listener sees the library and its tracks.
    assert_eq!(
        state.db.libraries_for_user(listener).await.unwrap().len(),
        1
    );
    assert!(state
        .db
        .create_scan_job_for_user(listener, library, "manual")
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        state.services.start_library_scan(listener, library).await,
        Err(ServiceError::NotFound)
    ));
    // startScan names no library, so it skips the read-only ones instead of
    // failing: an account whose every library is read-only queues nothing and
    // succeeds, exactly like one that reaches no library at all.
    assert!(state
        .services
        .start_visible_scans(listener)
        .await
        .unwrap()
        .is_empty());
    let listener_jobs: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM scan_job WHERE requested_by = ?")
            .bind(listener.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(listener_jobs, 0, "a listener queued a scan");
    // Promotion is all it takes, and it is read from the row rather than
    // cached anywhere.
    state
        .db
        .add_library_member(owner, library, listener, LibraryRole::Manager, now_ms())
        .await
        .unwrap();
    assert!(state
        .db
        .create_scan_job_for_user(listener, library, "manual")
        .await
        .unwrap()
        .is_some());
    state
        .db
        .remove_library_member(owner, library, listener, now_ms())
        .await
        .unwrap();

    // An account that can reach no library has nothing to scan. That is an
    // empty result, not an error: there is no missing resource to report, and
    // every other catalogue-wide method answers such an account the same way.
    let stranger = state
        .db
        .create_account("asym-stranger", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    assert!(state
        .services
        .start_visible_scans(stranger)
        .await
        .unwrap()
        .is_empty());

    // Container-only aliases for browse-by-folder clients. The payload is the
    // ID3 one; only the wrapper name differs, as for getAlbumList.
    let search2 = subsonic_json(&router, "search2", api_key, "&query=One&songCount=10").await;
    assert!(search2["subsonic-response"]["searchResult2"]["song"].is_array());
    assert!(search2["subsonic-response"].get("searchResult3").is_none());
    // Nothing is starred yet, so the container is present but carries no list.
    let starred = subsonic_json(&router, "getStarred", api_key, "").await;
    assert!(starred["subsonic-response"]["starred"].is_object());
    assert!(starred["subsonic-response"]["starred"]
        .get("song")
        .is_none());
    assert!(starred["subsonic-response"].get("starred2").is_none());

    // One favorite of each kind, so the renamed container is exercised with
    // content: an alias that only ever answers empty proves nothing about the
    // JSON array rules its new name needs.
    let snapshot = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    for (entity, id) in [
        ("artist", snapshot.artists[0].artist.id),
        ("album", snapshot.albums[0].id),
        ("track", snapshot.songs[0].id),
    ] {
        state
            .services
            .set_star(owner, entity, id, true)
            .await
            .unwrap();
    }
    let starred = subsonic_json(&router, "getStarred", api_key, "").await;
    let starred = &starred["subsonic-response"]["starred"];
    for field in ["artist", "album", "song"] {
        let entries = starred[field]
            .as_array()
            .unwrap_or_else(|| panic!("starred.{field} is not an array: {starred}"));
        assert_eq!(entries.len(), 1, "{field}");
    }
    // The ID3 method answers the same payload under its own container.
    let starred2 = subsonic_json(&router, "getStarred2", api_key, "").await;
    let starred2 = &starred2["subsonic-response"]["starred2"];
    for field in ["artist", "album", "song"] {
        assert_eq!(starred2[field], starred[field], "{field}");
    }

    // Surfaces WaveFlow does not compute answer the standard empty container
    // rather than a not-implemented error.
    for (method, container) in [
        ("getTopSongs", "topSongs"),
        ("getSimilarSongs", "similarSongs"),
        ("getSimilarSongs2", "similarSongs2"),
        ("getInternetRadioStations", "internetRadioStations"),
    ] {
        let response = subsonic_json(&router, method, api_key, "&id=whatever&count=5").await;
        assert_eq!(response["subsonic-response"]["status"], "ok", "{method}");
        assert_eq!(
            response["subsonic-response"][container],
            serde_json::json!({}),
            "{method}"
        );
    }

    // No avatars are stored, so the data is missing rather than the method.
    let avatar = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAvatar.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&username=asym-owner"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(avatar.status(), StatusCode::OK);
    assert_eq!(
        json_body(avatar).await["subsonic-response"]["error"]["code"],
        70
    );

    // A method that really is unimplemented still says so.
    let unknown = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getPodcasts.view?apiKey={api_key}&v=1.16.1&c=golden&f=json"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(unknown).await["subsonic-response"]["error"]["code"],
        0
    );

    // Starting a scan comes last: it runs a real scan over a library root that
    // holds no files, which marks the fabricated tracks unavailable. Every
    // assertion above reads the catalogue and would race it.
    //
    // The response carries the same shape as getScanStatus, so a client that
    // only calls startScan still learns the state.
    let started = subsonic_json(&router, "startScan", api_key, "").await;
    let started = &started["subsonic-response"]["scanStatus"];
    assert!(started["scanning"].is_boolean());
    assert!(started["count"].is_number());
    // The work is real: a job now exists for the owner's library beyond the one
    // the fixture created.
    let queued: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scan_job WHERE library_id = ?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(queued >= 2, "startScan queued nothing: {queued} jobs");
    let foreign_jobs: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM scan_job sj JOIN library l ON l.id = sj.library_id \
         WHERE l.name = 'asym-foreign'",
    )
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(
        foreign_jobs, 1,
        "startScan reached a library the account cannot see"
    );
}

/// Bookmarks are the last Subsonic mutation that had nowhere to write. They go
/// through the domain services like every other piece of user data, which is
/// what puts them in the sync journal and the bootstrap snapshot rather than
/// only in one client's view.
#[tokio::test]
async fn bookmarks_round_trip_sync_and_isolate_tenants() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_bookmark-key";
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("bookmark-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"bookmark-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("bookmark-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();

    let seed = |account: Uuid, name: &'static str, offset: usize| {
        let state = state.clone();
        let root = config.data_dir.join(name);
        async move {
            std::fs::create_dir_all(&root).unwrap();
            let library = state
                .db
                .create_library(
                    account,
                    name,
                    &std::fs::canonicalize(&root).unwrap(),
                    LibraryVisibility::Private,
                    now_ms(),
                )
                .await
                .unwrap();
            let scan = state
                .db
                .create_scan_job(library, Some(account), "manual")
                .await
                .unwrap();
            state.db.start_scan_job(scan, 1, false).await.unwrap();
            let mut input = browse_input(
                offset,
                "Long Form",
                "Bookmark Album",
                "Bookmark Artist",
                Some(1),
                Some(1),
            );
            input.relative_path = format!("{name}.flac");
            input.quick_hash = format!("{:064x}", offset + 20_000);
            input.full_hash = format!("{:064x}", offset + 21_000);
            state
                .db
                .apply_catalog_track(library, scan, &input, None, false)
                .await
                .unwrap();
            state.db.finish_scan_job(scan, 0).await.unwrap();
            state
                .services
                .catalog_snapshot(account, &[])
                .await
                .unwrap()
                .songs[0]
                .id
        }
    };
    let track = seed(owner, "bookmark-own", 200).await;
    let foreign_track = seed(outsider, "bookmark-foreign", 201).await;

    let router = waveflow_server::app(&config, state.clone());

    // Nothing set yet: the container is present and empty, as it always was.
    let empty = subsonic_json(&router, "getBookmarks", api_key, "").await;
    assert!(empty["subsonic-response"]["bookmarks"].is_object());
    assert!(empty["subsonic-response"]["bookmarks"]
        .get("bookmark")
        .is_none());

    let created = subsonic_json(
        &router,
        "createBookmark",
        api_key,
        &format!("&id={track}&position=125000&comment=where%20I%20stopped"),
    )
    .await;
    assert_eq!(created["subsonic-response"]["status"], "ok");
    // A mutation with no result answers the bare envelope.
    assert!(created["subsonic-response"].get("createBookmark").is_none());

    let listed = subsonic_json(&router, "getBookmarks", api_key, "").await;
    let bookmark = &listed["subsonic-response"]["bookmarks"]["bookmark"][0];
    assert_eq!(bookmark["position"], 125_000);
    assert_eq!(bookmark["username"], "bookmark-owner");
    assert_eq!(bookmark["comment"], "where I stopped");
    assert!(bookmark["created"].is_string());
    assert!(bookmark["changed"].is_string());
    // The entry is a full media item, carrying the position it is bookmarked at.
    assert_eq!(bookmark["entry"]["id"], track.to_string());
    assert_eq!(bookmark["entry"]["title"], "Long Form");
    assert_eq!(bookmark["entry"]["bookmarkPosition"], 125_000);
    // It goes through the shared projection, so the modern fields are there too.
    assert_eq!(bookmark["entry"]["samplingRate"], 44_100);
    assert!(bookmark["entry"]["artists"].is_array());

    // A bookmark answers "where did I stop in this file", so setting it again
    // moves it rather than adding a second one.
    subsonic_json(
        &router,
        "createBookmark",
        api_key,
        &format!("&id={track}&position=250000"),
    )
    .await;
    let moved = subsonic_json(&router, "getBookmarks", api_key, "").await;
    let moved = &moved["subsonic-response"]["bookmarks"]["bookmark"];
    assert_eq!(moved.as_array().unwrap().len(), 1);
    assert_eq!(moved[0]["position"], 250_000);
    // Omitting the comment clears it rather than keeping the old one.
    assert!(moved[0].get("comment").is_none());

    // It reaches the sync surfaces because it is a domain mutation, not a
    // facade-local one: a desktop client sees it without a second contract.
    let snapshot = state.services.sync_snapshot(owner, 50).await.unwrap();
    assert_eq!(snapshot.bookmarks.len(), 1);
    assert_eq!(snapshot.bookmarks[0].position_ms, 250_000);
    assert_eq!(snapshot.bookmarks[0].song.id, track);
    let changes = state.sync.changes(owner, 0, 100).await.unwrap();
    assert!(
        changes
            .changes
            .iter()
            .any(|change| change.entity_type == "bookmark" && change.entity_id == track),
        "no bookmark event in the journal: {:?}",
        changes
            .changes
            .iter()
            .map(|change| change.entity_type.clone())
            .collect::<Vec<_>>()
    );

    // And the native bootstrap carries them, so a desktop client that only
    // ever calls /sync/snapshot receives bookmarks without a second contract.
    let token = login_token(&router, "bookmark-owner", "correct horse battery staple").await;
    let native = router
        .clone()
        .oneshot(
            Request::get("/api/v2/sync/snapshot")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(native.status(), StatusCode::OK);
    let native = json_body(native).await;
    assert_eq!(native["bookmarks"].as_array().unwrap().len(), 1);
    assert_eq!(native["bookmarks"][0]["position_ms"], 250_000);
    assert_eq!(native["bookmarks"][0]["song"]["id"], track.to_string());

    // A position before the start of the file is not a position. The service
    // refuses it, and the facade reports the parameter error rather than an
    // internal one.
    assert!(matches!(
        state.services.set_bookmark(owner, track, -1, None).await,
        Err(ServiceError::Invalid)
    ));
    let negative = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={track}&position=-1"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(negative).await["subsonic-response"]["error"]["code"],
        10
    );

    // XML is the default encoding and nests the entry as an element rather
    // than as a JSON object, so it gets its own assertion.
    let xml = router
        .clone()
        .oneshot(
            Request::get(
                "/rest/getBookmarks.view?u=bookmark-owner&p=bookmark-secret&v=1.16.1&c=golden",
            )
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(xml.status(), StatusCode::OK);
    let xml = body_text(xml).await;
    assert!(xml.contains("<bookmark "));
    assert!(xml.contains("username=\"bookmark-owner\""));
    assert!(xml.contains("position=\"250000\""));
    assert!(xml.contains("<entry "));
    assert!(xml.contains("bookmarkPosition=\"250000\""));

    // Another account's track is not bookmarkable, and the refusal does not
    // confirm that the track exists.
    let foreign = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={foreign_track}&position=1000"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::OK);
    assert_eq!(
        json_body(foreign).await["subsonic-response"]["error"]["code"],
        70
    );
    let unknown = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/createBookmark.view?apiKey={api_key}&v=1.16.1&c=golden&f=json&id={}&position=1000",
                Uuid::nil()
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json_body(unknown).await["subsonic-response"]["error"]["code"],
        70
    );
    assert!(state.services.bookmarks(outsider).await.unwrap().is_empty());

    subsonic_json(&router, "deleteBookmark", api_key, &format!("&id={track}")).await;
    assert!(state.services.bookmarks(owner).await.unwrap().is_empty());
    // Deleting one that is not there is not an error: the caller asked for the
    // track to carry no bookmark, and it does not.
    subsonic_json(&router, "deleteBookmark", api_key, &format!("&id={track}")).await;
}

/// An album artist tag holding two credits is two artists, and the album now
/// hangs off both.
///
/// Feeding the joined string to the artist table minted an entity named after
/// it, gave that entity the album, and left both real artists with nothing:
/// DSub browsed to either and found no album. Splitting the credit fixed the
/// entity but not the browse — the album still pointed at one artist through a
/// single column, so the second credit found nothing. The album's participants
/// are what answer now, and both credits reach it.
#[tokio::test]
async fn an_album_hangs_off_every_artist_it_is_credited_to() {
    let (_temp, _config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("credits", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = _config.data_dir.join("credit-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Credits",
            &root,
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
    state.db.start_scan_job(scan, 4, false).await.unwrap();

    // Four albums whose credits differ only in where the boundaries fall.
    for (index, album, credit) in [
        (0usize, "Live", "Nova Kern; Lior Sand"),
        (1, "Live", "Nova Kern; Ada Vale"),
        (2, "Split", "A; B C"),
        (3, "Split", "A; B; C"),
    ] {
        let mut input = catalog_input(index, credit);
        input.title = format!("Track {index}");
        input.album = Some(album.into());
        input.album_artist = Some(credit.into());
        input.is_compilation = false;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.consolidate_musicbrainz_ids(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    // No entity is named after a joined string.
    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .iter()
        .map(|summary| summary.artist.name.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "A".to_owned(),
            "Ada Vale".to_owned(),
            "B".to_owned(),
            "B C".to_owned(),
            "C".to_owned(),
            "Lior Sand".to_owned(),
            "Nova Kern".to_owned(),
        ],
        "each credit is its own artist and the joined string is nobody"
    );

    // Sharing a title and a lead credit is not sharing an album.
    let albums = state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap();
    assert_eq!(
        albums.iter().filter(|album| album.title == "Live").count(),
        2,
        "`A; B` and `A; C` are two records, not one"
    );
    assert_eq!(
        albums.iter().filter(|album| album.title == "Split").count(),
        2,
        "where the boundary falls is part of the identity"
    );

    // And the second credit reaches the album, which is what the single
    // `album_artist_id` column could never answer.
    let by_name = |name: &str| {
        artists
            .iter()
            .find(|summary| summary.artist.name == name)
            .unwrap_or_else(|| panic!("{name} was indexed"))
            .artist
            .id
    };
    for (name, credited_on) in [("Nova Kern", 2), ("Lior Sand", 1), ("Ada Vale", 1)] {
        let detail = state.services.artist(owner, by_name(name)).await.unwrap();
        assert_eq!(
            detail.albums.len(),
            credited_on,
            "browsing to {name} finds every album it is credited to"
        );
        assert!(detail.albums.iter().all(|album| album.title == "Live"));
        assert_eq!(
            detail.album_count, credited_on as i64,
            "{name}'s album count counts the credits, not a single column"
        );
    }

    // The display string stays what the file wrote, joins and all.
    let live = albums
        .iter()
        .find(|album| album.title == "Live")
        .expect("an album titled Live");
    assert!(
        live.artist
            .as_deref()
            .is_some_and(|credit| credit.contains("; ")),
        "the album still renders the credit as written: {:?}",
        live.artist
    );
}

/// A release identifier belongs to the release, not to whichever file was
/// scanned last.
///
/// Under the default identity spec the release identifier *is* the album's
/// identity, so files naming different releases are different albums and the
/// majority vote has nothing left to settle there. It still runs, because a
/// spec that does not name `musicbrainz_albumid` puts the disagreement back —
/// and because the artist's identifier is voted on the same way, where no spec
/// can make the question go away.
#[tokio::test]
async fn entity_musicbrainz_ids_are_a_majority_vote_over_the_tracks() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_mbid-key";
    let owner = state
        .db
        .create_account(
            "mbid-owner",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"mbid-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("mbid-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Mbid",
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
    state.db.start_scan_job(scan, 6, false).await.unwrap();

    struct TaggedFile {
        album: &'static str,
        title: &'static str,
        track: i64,
        disc: i64,
        release: Option<&'static str>,
        artist: Option<&'static str>,
    }
    let file = |album, title, track, disc, release, artist| TaggedFile {
        album,
        title,
        track,
        disc,
        release,
        artist,
    };
    let fixture = [
        // Two files agree on the reissue, one still carries the original
        // pressing. The majority wins, and the odd file does not.
        file(
            "Split Sky",
            "Dawn",
            1,
            1,
            Some("release-reissue"),
            Some("artist-vale"),
        ),
        file(
            "Split Sky",
            "Noon",
            2,
            1,
            Some("release-original"),
            Some("artist-vale"),
        ),
        file("Split Sky", "Dusk", 3, 1, Some("release-reissue"), None),
        // A genuine tie, one file each. It is broken by the earliest disc and
        // track, so the answer is stable across scans rather than arbitrary.
        // The earlier one sorts last as a string, so a lexical fallback alone
        // would answer the other.
        file(
            "Even Halves",
            "Side A",
            1,
            1,
            Some("release-zulu"),
            Some("artist-vale"),
        ),
        file(
            "Even Halves",
            "Side B",
            1,
            2,
            Some("release-alpha"),
            Some("artist-other"),
        ),
        // Nothing tagged at all.
        file("No Tags", "Silence", 1, 1, None, None),
    ];
    for (index, entry) in fixture.into_iter().enumerate() {
        let mut input = browse_input(
            300 + index,
            entry.title,
            entry.album,
            "Vale",
            Some(entry.track),
            Some(entry.disc),
        );
        input.relative_path = format!("mbid-{index}.flac");
        input.quick_hash = format!("{:064x}", index + 31_000);
        input.full_hash = format!("{:064x}", index + 32_000);
        input.musicbrainz_release_id = entry.release.map(str::to_owned);
        input.musicbrainz_artist_id = entry.artist.map(str::to_owned);
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let albums_by_title = || {
        let state = state.clone();
        async move {
            state
                .services
                .catalog_snapshot(owner, &[])
                .await
                .unwrap()
                .albums
                .into_iter()
                .map(|album| (album.title.clone(), album))
                .collect::<std::collections::BTreeMap<_, _>>()
        }
    };

    // Nothing is derived until the pass that derives it: the tracks carry the
    // identifiers from the moment they are indexed, the albums do not.
    assert!(albums_by_title()
        .await
        .values()
        .all(|album| album.musicbrainz_id.is_none()));

    state.db.consolidate_musicbrainz_ids(library).await.unwrap();

    // The majority vote no longer has anything to resolve on an album, and
    // that is the point: under the default identity spec a release identifier
    // *is* the album's identity, so two files carrying different ones are two
    // albums rather than one album with a disagreement to settle. "Split Sky"
    // holds three files naming two releases, and answers as two records.
    let all_albums = state
        .services
        .list_albums(owner, &Default::default())
        .await
        .unwrap();
    let split: Vec<&str> = all_albums
        .iter()
        .filter(|album| album.title == "Split Sky")
        .filter_map(|album| album.musicbrainz_id.as_deref())
        .collect();
    let mut split = split;
    split.sort_unstable();
    assert_eq!(
        split,
        vec!["release-original", "release-reissue"],
        "two release identifiers are two albums, each reporting its own"
    );
    // The vote still runs, and still clears: an album whose files name no
    // release has nothing to report.
    assert!(all_albums
        .iter()
        .filter(|album| album.title == "No Tags")
        .all(|album| album.musicbrainz_id.is_none()));

    // The artist takes the identifier from the tracks it is the primary credit
    // of. Every track here credits Vale first, and `artist-vale` is what most
    // of them say.
    let snapshot = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    let artist = snapshot
        .artists
        .iter()
        .find(|artist| artist.artist.name == "Vale")
        .expect("the artist was indexed");
    assert_eq!(artist.artist.musicbrainz_id.as_deref(), Some("artist-vale"));

    let router = waveflow_server::app(&config, state.clone());
    let albums = albums_by_title().await;
    // Either of the two "Split Sky" records answers; both carry the release
    // they were identified by.
    let reissue = all_albums
        .iter()
        .find(|album| album.musicbrainz_id.as_deref() == Some("release-reissue"))
        .expect("the reissue is one of the two records");
    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={}", reissue.id)).await;
    assert_eq!(
        album["subsonic-response"]["album"]["musicBrainzId"],
        "release-reissue"
    );
    // Presence, not omission: an album with no release id still carries the
    // field, because that is the only way a client tells "untagged" from "this
    // server does not read the tag".
    let untagged = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", albums["No Tags"].id),
    )
    .await;
    assert_eq!(untagged["subsonic-response"]["album"]["musicBrainzId"], "");
    let artist_response = subsonic_json(
        &router,
        "getArtist",
        api_key,
        &format!("&id={}", artist.artist.id),
    )
    .await;
    assert_eq!(
        artist_response["subsonic-response"]["artist"]["musicBrainzId"],
        "artist-vale"
    );

    // getAlbumInfo predates the presence rule and its members are elements, so
    // the untagged album omits it rather than sending an empty one.
    let info = subsonic_json(
        &router,
        "getAlbumInfo2",
        api_key,
        &format!("&id={}", reissue.id),
    )
    .await;
    assert_eq!(
        info["subsonic-response"]["albumInfo2"]["musicBrainzId"],
        "release-reissue"
    );
    let info = subsonic_json(
        &router,
        "getAlbumInfo2",
        api_key,
        &format!("&id={}", albums["No Tags"].id),
    )
    .await;
    assert!(info["subsonic-response"]["albumInfo2"]
        .get("musicBrainzId")
        .is_none());

    // On a directory child the specification defines musicBrainzId as the
    // recording, and a folder standing for a release has no recording, so the
    // browse view drops it rather than putting a release id under that name.
    let directory = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={}", artist.artist.id),
    )
    .await;
    let children = directory["subsonic-response"]["directory"]["child"]
        .as_array()
        .expect("the artist directory lists its albums");
    assert!(!children.is_empty());
    for child in children {
        assert!(
            child.get("musicBrainzId").is_none(),
            "a browsing entry claimed a recording id: {child}"
        );
    }

    // A tag removed from the files has to disappear from the catalogue: the
    // derivation runs after every scan, so it clears as readily as it sets.
    sqlx::query("UPDATE track SET musicbrainz_release_id = NULL WHERE library_id = ?")
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    state.db.consolidate_musicbrainz_ids(library).await.unwrap();
    assert!(albums_by_title()
        .await
        .values()
        .all(|album| album.musicbrainz_id.is_none()));
}

/// A genre is one thing or it is nothing. `getGenres` folds spelling variants
/// into one row, so the method that lists a genre's songs has to fold them the
/// same way — otherwise a client displays a genre it was just handed and finds
/// it empty.
#[tokio::test]
async fn genre_matching_is_canonical_on_every_surface() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_genre-key";
    let owner = state
        .db
        .create_account(
            "genre-owner",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"genre-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("genre-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Genres",
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
    state.db.start_scan_job(scan, 3, false).await.unwrap();

    // The same genre, spelled three ways across three files. Canonicalisation
    // folds case, punctuation and spacing, so all three are one genre.
    for (index, (title, genre)) in [
        ("Boom", "Hip-Hop"),
        ("Bap", "hip hop"),
        ("Clap", "HIP  HOP"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(
            400 + index,
            title,
            "Cipher",
            "Nine Mics",
            Some(index as i64 + 1),
            Some(1),
        );
        input.relative_path = format!("genre-{index}.flac");
        input.quick_hash = format!("{:064x}", index + 41_000);
        input.full_hash = format!("{:064x}", index + 42_000);
        input.genre = Some(genre.into());
        input.year = Some(2001);
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    // The album's own credits are derived at the end of a scan, like the
    // identifiers and the sort names: a test driving the catalogue directly
    // runs the same pass the scanner runs.
    state.db.consolidate_musicbrainz_ids(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());

    // One row, holding all three tracks.
    let genres = subsonic_json(&router, "getGenres", api_key, "").await;
    let listed = genres["subsonic-response"]["genres"]["genre"]
        .as_array()
        .expect("genres is an array");
    assert_eq!(listed.len(), 1, "one genre, three spellings: {listed:?}");
    assert_eq!(listed[0]["songCount"], 3);
    let name = listed[0]["value"]
        .as_str()
        .expect("the genre carries its name")
        .to_owned();

    // Asking for the name the server just gave must return all three, and so
    // must each of the other spellings: they are the same genre.
    for spelling in [name.as_str(), "Hip-Hop", "hip hop", "HIP  HOP"] {
        let encoded = spelling.replace(' ', "%20");
        let songs = subsonic_json(
            &router,
            "getSongsByGenre",
            api_key,
            &format!("&genre={encoded}&count=50"),
        )
        .await;
        let entries = songs["subsonic-response"]["songsByGenre"]["song"]
            .as_array()
            .unwrap_or_else(|| panic!("no songs for {spelling}: {songs}"));
        assert_eq!(entries.len(), 3, "{spelling}");
    }

    // getRandomSongs applies the same rule, and its year filter still narrows.
    let random = subsonic_json(&router, "getRandomSongs", api_key, "&genre=Hip-Hop&size=50").await;
    assert_eq!(
        random["subsonic-response"]["randomSongs"]["song"]
            .as_array()
            .expect("randomSongs is an array")
            .len(),
        3
    );
    let out_of_range = subsonic_json(
        &router,
        "getRandomSongs",
        api_key,
        "&genre=Hip-Hop&size=50&fromYear=2010&toYear=2020",
    )
    .await;
    assert!(out_of_range["subsonic-response"]["randomSongs"]
        .get("song")
        .is_none());
    // A reversed range is how Subsonic asks for one, not an empty request.
    let reversed = subsonic_json(
        &router,
        "getRandomSongs",
        api_key,
        "&genre=Hip-Hop&size=50&fromYear=2005&toYear=1999",
    )
    .await;
    assert_eq!(
        reversed["subsonic-response"]["randomSongs"]["song"]
            .as_array()
            .expect("randomSongs is an array")
            .len(),
        3
    );

    // A genre nobody uses is an empty list, not an error.
    let unknown = subsonic_json(&router, "getSongsByGenre", api_key, "&genre=Polka").await;
    assert_eq!(unknown["subsonic-response"]["status"], "ok");
    assert!(unknown["subsonic-response"]["songsByGenre"]
        .get("song")
        .is_none());

    // The album filter already matched canonically, and still agrees.
    let by_genre = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=byGenre&genre=hip%20hop&size=10",
    )
    .await;
    assert_eq!(
        by_genre["subsonic-response"]["albumList2"]["album"]
            .as_array()
            .expect("albumList2 is an array")
            .len(),
        1
    );

    // Paging getSongsByGenre no longer slices a full catalogue read, and the
    // page boundaries still line up.
    let first = subsonic_json(
        &router,
        "getSongsByGenre",
        api_key,
        "&genre=Hip-Hop&count=2&offset=0",
    )
    .await;
    let second = subsonic_json(
        &router,
        "getSongsByGenre",
        api_key,
        "&genre=Hip-Hop&count=2&offset=2",
    )
    .await;
    assert_eq!(
        first["subsonic-response"]["songsByGenre"]["song"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        second["subsonic-response"]["songsByGenre"]["song"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // The native surface answers the same two questions the facade does, on
    // the same services: an asymmetry the audit left open, where the query
    // existed and only the HTTP adapter was missing.
    let login = router
        .clone()
        .oneshot(json_request(
            "/api/v2/auth/login",
            serde_json::json!({
                "username": "genre-owner",
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
    let native = |path: String| {
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
    // Any spelling reaches the same three tracks, natively too.
    let by_genre = json_body(native("/api/v2/songs?genre=hip%20hop&limit=50".into()).await).await;
    assert_eq!(by_genre.as_array().expect("a list").len(), 3);
    let random =
        json_body(native("/api/v2/songs/random?genre=HIP%20%20HOP&limit=50".into()).await).await;
    assert_eq!(random.as_array().expect("a list").len(), 3);
    // The year filter narrows, and a genre nobody uses is empty rather than
    // an error.
    let narrowed = json_body(
        native("/api/v2/songs/random?genre=Hip-Hop&limit=50&from_year=2010&to_year=2020".into())
            .await,
    )
    .await;
    assert!(narrowed.as_array().expect("a list").is_empty());
    let unused = json_body(native("/api/v2/songs?genre=Polka".into()).await).await;
    assert!(unused.as_array().expect("a list").is_empty());
    // The genre is what the request is about, so its absence is a malformed
    // request and not an unfiltered catalogue.
    assert_eq!(
        native("/api/v2/songs".into()).await.status(),
        StatusCode::BAD_REQUEST
    );
    // Search pages each kind on its own offset.
    let paged =
        json_body(native("/api/v2/search?q=Boom&limit=10&song_offset=5".into()).await).await;
    assert!(paged["songs"].as_array().expect("songs").is_empty());
    assert_eq!(paged["albums"].as_array().expect("albums").len(), 1);

    // The album carries the credits and genres of its tracks, folded the same
    // way, which is what AlbumID3 asks for.
    let albums = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    let album = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", albums.albums[0].id),
    )
    .await;
    let album = &album["subsonic-response"]["album"];
    assert_eq!(
        album["genres"]
            .as_array()
            .expect("album genres is an array")
            .len(),
        1,
        "three spellings should be one album genre: {album}"
    );
    assert_eq!(album["artists"][0]["name"], "Nine Mics");
    // And a track names the album's credit beside its own.
    assert_eq!(album["song"][0]["displayAlbumArtist"], "Nine Mics");
    assert_eq!(album["song"][0]["albumArtists"][0]["name"], "Nine Mics");
}

/// The browse methods used to resolve through a snapshot of every visible
/// track. They now ask for what they render, which is only observable as
/// behaviour at the edges: a foreign id is still not found, an album still
/// comes back in sleeve order, and the match-all search still pages.
#[tokio::test]
async fn browse_methods_read_only_what_they_render() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_browse-key";
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("browse-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            owner,
            owner,
            &state.secret_box.encrypt(b"browse-secret").unwrap(),
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let outsider = state
        .db
        .create_account("browse-outsider", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();

    let seed = |account: Uuid, name: &'static str, artist: &'static str| {
        let state = state.clone();
        let root = config.data_dir.join(name);
        async move {
            std::fs::create_dir_all(&root).unwrap();
            let library = state
                .db
                .create_library(
                    account,
                    name,
                    &std::fs::canonicalize(&root).unwrap(),
                    LibraryVisibility::Private,
                    now_ms(),
                )
                .await
                .unwrap();
            let scan = state
                .db
                .create_scan_job(library, Some(account), "manual")
                .await
                .unwrap();
            state.db.start_scan_job(scan, 3, false).await.unwrap();
            // Deliberately out of sleeve order, and titled so that ordering by
            // title would give a different answer from ordering by track.
            for (index, (title, track)) in [("Zephyr", 1), ("Anvil", 2), ("Marrow", 3)]
                .into_iter()
                .enumerate()
            {
                let mut input = browse_input(
                    500 + index + name.len() * 10,
                    title,
                    "Ordered",
                    artist,
                    Some(track),
                    Some(1),
                );
                input.relative_path = format!("{name}-{index}.flac");
                input.quick_hash = format!("{:064x}", index + 51_000 + name.len() * 100);
                input.full_hash = format!("{:064x}", index + 52_000 + name.len() * 100);
                state
                    .db
                    .apply_catalog_track(library, scan, &input, None, false)
                    .await
                    .unwrap();
            }
            state.db.consolidate_musicbrainz_ids(library).await.unwrap();
            state.db.finish_scan_job(scan, 0).await.unwrap();
            library
        }
    };
    let library = seed(owner, "browse-own", "Own Artist").await;
    seed(outsider, "browse-foreign", "Foreign Artist").await;

    let router = waveflow_server::app(&config, state.clone());
    let mine = state.services.catalog_snapshot(owner, &[]).await.unwrap();
    let theirs = state
        .services
        .catalog_snapshot(outsider, &[])
        .await
        .unwrap();

    // getAlbum asks for one album, and returns it in sleeve order rather than
    // alphabetically.
    let album = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", mine.albums[0].id),
    )
    .await;
    let titles = album["subsonic-response"]["album"]["song"]
        .as_array()
        .expect("the album lists its songs")
        .iter()
        .map(|song| song["title"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(titles, vec!["Zephyr", "Anvil", "Marrow"]);

    // A foreign album is not found, indistinguishably from one that does not
    // exist. This is what the snapshot used to enforce by simply not holding
    // the row.
    for (method, id) in [
        ("getAlbum", theirs.albums[0].id),
        ("getArtist", theirs.artists[0].artist.id),
        ("getMusicDirectory", theirs.albums[0].id),
    ] {
        let response = subsonic_json(&router, method, api_key, &format!("&id={id}")).await;
        assert_eq!(
            response["subsonic-response"]["error"]["code"], 70,
            "{method} reached another account: {response}"
        );
    }

    // getMusicDirectory answers at all three levels, and the album level is the
    // only one that loads tracks.
    let folder = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={library}"),
    )
    .await;
    assert_eq!(
        folder["subsonic-response"]["directory"]["child"][0]["name"],
        "Own Artist"
    );
    let artist_dir = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={}", mine.artists[0].artist.id),
    )
    .await;
    assert_eq!(
        artist_dir["subsonic-response"]["directory"]["child"][0]["title"],
        "Ordered"
    );
    let album_dir = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={}", mine.albums[0].id),
    )
    .await;
    assert_eq!(
        album_dir["subsonic-response"]["directory"]["child"]
            .as_array()
            .expect("the album directory lists its tracks")
            .len(),
        3
    );

    // getStarred reads the star join rather than the catalogue, and reports one
    // of each kind.
    for (entity, id) in [
        ("artist", mine.artists[0].artist.id),
        ("album", mine.albums[0].id),
        ("track", mine.songs[0].id),
    ] {
        state
            .services
            .set_star(owner, entity, id, true)
            .await
            .unwrap();
    }
    let starred = subsonic_json(&router, "getStarred2", api_key, "").await;
    let starred = &starred["subsonic-response"]["starred2"];
    for field in ["artist", "album", "song"] {
        assert_eq!(
            starred[field].as_array().expect(field).len(),
            1,
            "{field}: {starred}"
        );
    }
    assert!(starred["song"][0]["starred"].is_string());

    // search3's match-all pages in SQL. Two pages of two cover three songs and
    // stop, and the third page is empty rather than an error.
    let page = |offset: usize| {
        let router = router.clone();
        async move {
            let response = subsonic_json(
                &router,
                "search3",
                api_key,
                &format!(
                    "&query=%22%22&songCount=2&songOffset={offset}&artistCount=0&albumCount=0"
                ),
            )
            .await;
            response["subsonic-response"]["searchResult3"]["song"]
                .as_array()
                .map(|songs| {
                    songs
                        .iter()
                        .map(|song| song["title"].as_str().unwrap_or_default().to_owned())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }
    };
    let first = page(0).await;
    let second = page(2).await;
    let third = page(4).await;
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 1);
    assert!(third.is_empty());
    // The pages do not overlap, and between them they are the whole library —
    // the outsider's tracks are in neither.
    let mut seen = first;
    seen.extend(second);
    seen.sort();
    assert_eq!(seen, vec!["Anvil", "Marrow", "Zephyr"]);
}

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
            .create_api_token(listener, "token-admin", "stolen", &[])
            .await,
        Err(ServiceError::Forbidden)
    ));
}

/// Catalogue fixture for the native browse endpoints. Unlike [`catalog_input`]
/// it is not a compilation, so `album_artist_id` is populated and the artist
/// drill-down has something to resolve.
#[allow(clippy::too_many_arguments)]
fn browse_input(
    index: usize,
    title: &str,
    album: &str,
    artist: &str,
    track_number: Option<i64>,
    disc_number: Option<i64>,
) -> CatalogTrackInput {
    CatalogTrackInput {
        relative_path: format!("browse-{index}.flac"),
        file_size: 2048 + index as i64,
        modified_at: 1_700_000_000_000 + index as i64,
        quick_hash: format!("{:064x}", index + 500),
        full_hash: format!("{:064x}", index + 900),
        title: title.into(),
        artist: Some(artist.into()),
        artists: Vec::new(),
        album_artists: Vec::new(),
        roles: Vec::new(),
        performer_pairs: Vec::new(),
        album: Some(album.into()),
        album_artist: Some(artist.into()),
        is_compilation: false,
        genre: Some("Ambient".into()),
        year: Some(2024),
        track_number,
        disc_number,
        duration_ms: 120_000,
        bitrate: Some(900),
        sample_rate: Some(44_100),
        channels: Some(2),
        bit_depth: Some(16),
        codec: Some("FLAC".into()),
        musical_key: None,
        tag_rating: None,
        musicbrainz_recording_id: None,
        musicbrainz_release_id: None,
        musicbrainz_artist_id: None,
        replay_gain_track_gain: None,
        replay_gain_track_peak: None,
        replay_gain_album_gain: None,
        replay_gain_album_peak: None,
        bpm: None,
        sort_title: None,
        sort_album: None,
        sort_album_artist: None,
        sort_artist: None,
        comment: None,
        isrc: None,
        moods: None,
        explicit_status: None,
        artwork: None,
        lyrics_hash: blake3::hash(b"").to_hex().to_string(),
        lyrics: Vec::new(),
    }
}

#[tokio::test]
async fn native_browse_endpoints_page_search_and_isolate_tenants() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("browse-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("browse-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("browse-music");
    std::fs::create_dir_all(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Browse",
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
    state.db.start_scan_job(scan_id, 5, false).await.unwrap();
    // Tracks are applied out of sleeve order on purpose: the album drill-down
    // must sort them, not echo insertion order.
    for (index, (title, album, artist, track, disc)) in [
        (
            "Slow Tide",
            "Aurora Fields",
            "Lumen Drift",
            Some(2),
            Some(1),
        ),
        (
            "First Light",
            "Aurora Fields",
            "Lumen Drift",
            Some(1),
            Some(1),
        ),
        // Incomplete tags are common in real libraries and must not jump ahead.
        ("Hidden Track", "Aurora Fields", "Lumen Drift", None, None),
        (
            "Rivière Noire",
            "Nocturne Bleue",
            "Écho Solaire",
            Some(2),
            Some(1),
        ),
        (
            "Prélude",
            "Nocturne Bleue",
            "Écho Solaire",
            Some(1),
            Some(1),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = browse_input(index, title, album, artist, track, disc);
        match title {
            // The album still has an artist, but this track has no credited
            // artist and therefore no track_artist row.
            "Hidden Track" => {
                input.artist = None;
                input.album_artist = Some("Lumen Drift".into());
            }
            // Materializes positions 0 and 1 while preserving the existing
            // album identity. The public primary must remain Écho Solaire.
            "Prélude" => {
                input.artist = Some("Écho Solaire; Lumen Drift".into());
                input.album_artist = Some("Écho Solaire".into());
            }
            _ => {}
        }
        state
            .db
            .apply_catalog_track(library_id, scan_id, &input, None, false)
            .await
            .unwrap();
    }
    state
        .db
        .consolidate_musicbrainz_ids(library_id)
        .await
        .unwrap();
    state.db.finish_scan_job(scan_id, 0).await.unwrap();

    let router = waveflow_server::app(&config, state);
    let owner_token = login_token(&router, "browse-owner", password).await;
    let intruder_token = login_token(&router, "browse-intruder", password).await;

    let get = |uri: String, token: String| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::get(uri)
                        .header("authorization", format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
        }
    };

    // Albums are ordered by title, so Aurora Fields precedes Nocturne Bleue.
    let albums = get("/api/v2/albums".into(), owner_token.clone()).await;
    assert_eq!(albums.status(), StatusCode::OK);
    let albums = json_body(albums).await;
    let albums = albums.as_array().unwrap();
    assert_eq!(albums.len(), 2);
    assert_eq!(albums[0]["title"], "Aurora Fields");
    assert_eq!(albums[1]["title"], "Nocturne Bleue");

    // Paging is applied in SQL, not after the fact.
    let page = get(
        "/api/v2/albums?limit=1&offset=1".into(),
        owner_token.clone(),
    )
    .await;
    let page = json_body(page).await;
    assert_eq!(page.as_array().unwrap().len(), 1);
    assert_eq!(page[0]["title"], "Nocturne Bleue");

    // The paging ceiling matches the Subsonic contract's 500-item cap.
    let rejected = get("/api/v2/albums?limit=501".into(), owner_token.clone()).await;
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let album_id = albums[0]["id"].as_str().unwrap().to_owned();
    let detail = get(format!("/api/v2/albums/{album_id}"), owner_token.clone()).await;
    assert_eq!(detail.status(), StatusCode::OK);
    let detail = json_body(detail).await;
    assert_eq!(detail["title"], "Aurora Fields");
    let album_artist_id = detail["artist_id"]
        .as_str()
        .expect("the album artist has a public id")
        .to_owned();
    let songs = detail["songs"].as_array().unwrap();
    assert_eq!(songs.len(), 3);
    assert_eq!(songs[0]["artist_id"], album_artist_id);
    assert_eq!(songs[1]["artist_id"], album_artist_id);
    assert!(songs[2]["artist_id"].is_null());
    assert_eq!(songs[0]["title"], "First Light", "sleeve order wins");
    assert_eq!(songs[1]["title"], "Slow Tide");
    assert_eq!(
        songs[2]["title"], "Hidden Track",
        "an untagged track sorts last, not ahead of track 1"
    );

    let artists = get("/api/v2/artists".into(), owner_token.clone()).await;
    let artists = json_body(artists).await;
    let artists = artists.as_array().unwrap();
    assert_eq!(artists.len(), 2);
    let echo = artists
        .iter()
        .find(|artist| artist["name"] == "Écho Solaire")
        .expect("accented artist is listed");
    assert_eq!(echo["album_count"], 1);

    let artist_id = echo["id"].as_str().unwrap().to_owned();
    let detail = get(format!("/api/v2/artists/{artist_id}"), owner_token.clone()).await;
    let detail = json_body(detail).await;
    assert_eq!(detail["name"], "Écho Solaire");
    assert_eq!(detail["albums"].as_array().unwrap().len(), 1);
    assert_eq!(detail["albums"][0]["title"], "Nocturne Bleue");

    // FTS5 folds diacritics, so an unaccented query still reaches "Écho Solaire".
    let found = get("/api/v2/search?q=echo".into(), owner_token.clone()).await;
    assert_eq!(found.status(), StatusCode::OK);
    let found = json_body(found).await;
    assert_eq!(found["artists"].as_array().unwrap().len(), 2);
    assert_eq!(found["albums"].as_array().unwrap().len(), 1);
    assert_eq!(found["albums"][0]["title"], "Nocturne Bleue");
    assert_eq!(found["songs"].as_array().unwrap().len(), 2);
    let found_artist_id = found["artists"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artist| artist["name"] == "Écho Solaire")
        .expect("the primary artist is included in search results")["id"]
        .as_str()
        .unwrap();
    for title in ["Prélude", "Rivière Noire"] {
        let song = found["songs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|song| song["title"] == title)
            .unwrap_or_else(|| panic!("missing search result {title}"));
        assert_eq!(song["artist_id"], found_artist_id, "{title}");
    }

    // The library projection uses TrackRecord rather than SongItem, but keeps
    // the same artist link so a client can open an artist from every track list.
    let tracks = get(
        format!("/api/v2/libraries/{library_id}/tracks"),
        owner_token.clone(),
    )
    .await;
    let tracks = json_body(tracks).await;
    assert_eq!(tracks.as_array().unwrap().len(), 5);
    let track = |title: &str| {
        tracks
            .as_array()
            .unwrap()
            .iter()
            .find(|track| track["title"] == title)
            .unwrap_or_else(|| panic!("missing library track {title}"))
    };
    assert_eq!(track("First Light")["artist_id"], album_artist_id);
    assert_eq!(track("Slow Tide")["artist_id"], album_artist_id);
    assert!(track("Hidden Track")["artist_id"].is_null());
    assert_eq!(track("Prélude")["artist_id"], artist_id);
    assert_eq!(track("Rivière Noire")["artist_id"], artist_id);

    // Search-as-you-type: the trailing term matches as a prefix. This exact
    // case was reported from the Android client — "echo" returned songs,
    // albums and artists while "ech" returned nothing, because the native
    // surface still required whole tokens after the Subsonic one moved on.
    let partial = get("/api/v2/search?q=ech".into(), owner_token.clone()).await;
    assert_eq!(partial.status(), StatusCode::OK);
    let partial = json_body(partial).await;
    assert_eq!(partial["songs"].as_array().unwrap().len(), 2);
    assert_eq!(partial["albums"].as_array().unwrap().len(), 1);
    assert_eq!(partial["artists"].as_array().unwrap().len(), 2);

    // Extra terms still narrow rather than widen.
    let narrowed = get(
        "/api/v2/search?q=echo%20nonexistent".into(),
        owner_token.clone(),
    )
    .await;
    assert!(json_body(narrowed).await["songs"]
        .as_array()
        .unwrap()
        .is_empty());

    // A search with no usable term is an empty result, never a SQL error.
    let blank = get("/api/v2/search?q=%20".into(), owner_token.clone()).await;
    assert_eq!(blank.status(), StatusCode::OK);
    let blank = json_body(blank).await;
    assert!(blank["songs"].as_array().unwrap().is_empty());

    // A foreign tenant sees an empty catalogue and cannot probe ids.
    let foreign = get("/api/v2/albums".into(), intruder_token.clone()).await;
    assert_eq!(foreign.status(), StatusCode::OK);
    assert!(json_body(foreign).await.as_array().unwrap().is_empty());
    for uri in [
        format!("/api/v2/albums/{album_id}"),
        format!("/api/v2/artists/{artist_id}"),
    ] {
        let response = get(uri, intruder_token.clone()).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "foreign ids must not be distinguishable from missing ones"
        );
    }
    let foreign_search = get("/api/v2/search?q=echo".into(), intruder_token).await;
    let foreign_search = json_body(foreign_search).await;
    assert!(foreign_search["artists"].as_array().unwrap().is_empty());
    assert!(foreign_search["songs"].as_array().unwrap().is_empty());

    // Anonymous access is rejected before any catalogue work happens.
    let anonymous = router
        .clone()
        .oneshot(Request::get("/api/v2/albums").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

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
    let lost_response = share_request().await;
    assert_eq!(lost_response.status(), StatusCode::CREATED);
    drop(lost_response);

    let replayed_share = share_request().await;
    assert_eq!(replayed_share.status(), StatusCode::CREATED);
    let replayed_share = json_body(replayed_share).await;
    let share_id = replayed_share["id"].as_str().unwrap();
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
        let notice = tokio::time::timeout(std::time::Duration::from_secs(1), notices.recv())
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
    let notice = tokio::time::timeout(std::time::Duration::from_secs(1), socket.next())
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
    assert!(state
        .services
        .queue(owner)
        .await
        .unwrap()
        .unwrap()
        .songs
        .is_empty());
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
    state.services.sync_snapshot(owner, 100).await.unwrap();
}

#[tokio::test]
async fn embedded_web_client_serves_shell_without_shadowing_the_api() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state);

    let get = |uri: &'static str| {
        let router = router.clone();
        async move {
            router
                .oneshot(Request::get(uri).body(Body::empty()).unwrap())
                .await
                .unwrap()
        }
    };

    // The shell is served at the root.
    let root = get("/").await;
    assert_eq!(root.status(), StatusCode::OK);
    assert!(root.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    // The shell must never be cached: it is what points at the hashed assets.
    assert_eq!(root.headers()["cache-control"], "no-cache");

    // Client-side routes resolve to the same shell rather than 404.
    let deep = get("/albums/some-client-route").await;
    assert_eq!(deep.status(), StatusCode::OK);
    assert!(deep.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));

    // An unknown API path stays a JSON 404 instead of silently returning HTML.
    let missing_api = get("/api/v2/does-not-exist").await;
    assert_eq!(missing_api.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        missing_api.headers()["content-type"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap(),
        "application/json"
    );

    // The other server namespaces answer as themselves rather than falling
    // through. /rest keeps its Subsonic contract — HTTP 200 with the failure in
    // the body — and /share is a plain 404. Neither may return the client shell.
    let unknown_method = get("/rest/nope").await;
    assert_eq!(unknown_method.status(), StatusCode::OK);
    assert!(!unknown_method.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));
    let unknown_method = body_text(unknown_method).await;
    assert!(unknown_method.starts_with("<subsonic-response"));
    assert!(unknown_method.contains("status=\"failed\""));

    let unknown_share = get("/share/nope").await;
    assert_eq!(unknown_share.status(), StatusCode::NOT_FOUND);
    assert!(!unknown_share
        .headers()
        .get("content-type")
        .map(|value| value.to_str().unwrap().to_owned())
        .unwrap_or_default()
        .starts_with("text/html"));

    // A client route that merely starts like a reserved endpoint is not one.
    let lookalike = get("/reference-guide").await;
    assert_eq!(lookalike.status(), StatusCode::OK);
    assert!(lookalike.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/html"));

    // Real routes are untouched by the fallback.
    let health = get("/health").await;
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(json_body(health).await["status"], "ok");
}

#[tokio::test]
async fn stream_tickets_authorise_browser_playback_without_a_bearer() {
    let (_temp, config, state) = test_app().await;
    let password = "correct horse battery staple";
    let hash = security::hash_password(password).unwrap();
    let owner = state
        .db
        .create_account("ticket-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    state
        .db
        .create_account("ticket-intruder", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("ticket-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Ticket.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Tickets",
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
            id: library_id,
            name: "Tickets".into(),
            root_path: root,
        },
    )
    .await;

    // Kept before `state` moves into the router, to mint an expired ticket below.
    let secret_box = std::sync::Arc::clone(&state.secret_box);
    let router = waveflow_server::app(&config, state.clone());
    let owner_token = login_token(&router, "ticket-owner", password).await;
    let intruder_token = login_token(&router, "ticket-intruder", password).await;

    let tracks = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/libraries/{library_id}/tracks"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let tracks = json_body(tracks).await;
    let track_id = tracks[0]["id"].as_str().unwrap().to_owned();

    let mint = |token: Option<String>| {
        let router = router.clone();
        let track_id = track_id.clone();
        async move {
            let request = Request::post(format!("/api/v2/tracks/{track_id}/stream-ticket"))
                .body(Body::empty());
            let request = match token {
                Some(token) => Request::post(format!("/api/v2/tracks/{track_id}/stream-ticket"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
                None => request.unwrap(),
            };
            router.oneshot(request).await.unwrap()
        }
    };

    // Minting requires a bearer; redeeming must not.
    assert_eq!(mint(None).await.status(), StatusCode::UNAUTHORIZED);

    let issued = mint(Some(owner_token.clone())).await;
    assert_eq!(issued.status(), StatusCode::OK);
    let issued = json_body(issued).await;
    let url = issued["url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/api/v2/stream/"));
    assert!(issued["expires_at"].as_i64().unwrap() > now_ms());

    // The ticket URL stays relative even when a public URL is configured, and
    // that asymmetry with createShare is deliberate: a share link is made to
    // leave the application, a ticket is not. An absolute ticket would let the
    // server point playback at a host the user never authenticated against, so
    // clients are right to reject absolute or protocol-relative values — this
    // pins the guarantee they rely on.
    let mut public_config = config.clone();
    public_config.public_url = Some("https://waveflow.example".to_owned());
    let public_router = waveflow_server::app(&public_config, state.clone());
    let public_ticket = public_router
        .oneshot(
            Request::post(format!("/api/v2/tracks/{track_id}/stream-ticket"))
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(public_ticket.status(), StatusCode::OK);
    let public_url = json_body(public_ticket).await["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        public_url.starts_with("/api/v2/stream/"),
        "ticket URL must stay relative, got {public_url}"
    );

    // The ticket URL plays with no Authorization header at all.
    let played = router
        .clone()
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(played.status(), StatusCode::OK);
    assert_eq!(played.headers()["accept-ranges"], "bytes");

    // Range requests work, which is what a browser seek relies on.
    let ranged = router
        .clone()
        .oneshot(
            Request::get(&url)
                .header("range", "bytes=0-15")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);

    // An expired ticket is refused even though it is otherwise well formed.
    let expired = waveflow_server::stream_ticket::mint(
        &secret_box,
        owner,
        uuid::Uuid::parse_str(&track_id).unwrap(),
        now_ms() - 1,
    )
    .unwrap();
    let expired = router
        .clone()
        .oneshot(
            Request::get(format!("/api/v2/stream/{expired}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::NOT_FOUND);

    // A tampered ticket is indistinguishable from an unknown track. The flipped
    // character sits mid-ticket: trailing base64url characters carry spare bits,
    // so changing one there can decode to the same bytes.
    let prefix_len = "/api/v2/stream/".len();
    let mut tampered: Vec<char> = url.chars().collect();
    let middle = prefix_len + (tampered.len() - prefix_len) / 2;
    tampered[middle] = if tampered[middle] == 'A' { 'B' } else { 'A' };
    let tampered: String = tampered.into_iter().collect();
    let forged = router
        .clone()
        .oneshot(Request::get(&tampered).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::NOT_FOUND);

    // A tenant without access cannot mint a ticket in the first place.
    assert_eq!(
        mint(Some(intruder_token)).await.status(),
        StatusCode::NOT_FOUND
    );
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

/// `search3` runs on the FTS5 index rather than filtering a fully materialised
/// catalogue in memory. That is not a pure refactor — it changes which queries
/// match — so the trade is pinned here rather than left to a client to discover.
#[tokio::test]
async fn subsonic_search_matches_through_the_fts_index() {
    let (_temp, config, state) = test_app().await;
    let subsonic_password = "subsonic-secret-123";
    let api_key = "wfsk_search-key";
    let admin = state
        .db
        .create_account(
            "search-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state
        .secret_box
        .encrypt(subsonic_password.as_bytes())
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("search-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(admin, "Search", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 3, false).await.unwrap();
    // One album of three tracks, one of them accented.
    for (index, title) in ["Echo Chamber", "Écho lointain", "Silent Partner"]
        .into_iter()
        .enumerate()
    {
        let mut input = catalog_input(index, "Nocturne");
        input.title = title.to_owned();
        input.album = Some("Night Sessions".into());
        input.is_compilation = false;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let titles = |result: &serde_json::Value| -> Vec<String> {
        result["subsonic-response"]["searchResult3"]["song"]
            .as_array()
            .map(|songs| {
                songs
                    .iter()
                    .filter_map(|song| song["title"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };

    // Whole-word match, as before.
    let whole = subsonic_json(&router, "search3", api_key, "&query=Echo&songCount=10").await;
    assert!(titles(&whole).contains(&"Echo Chamber".to_owned()));

    // Search-as-you-type: the trailing term matches as a prefix, so a client
    // querying on every keystroke keeps getting results.
    let prefix = subsonic_json(&router, "search3", api_key, "&query=Ech&songCount=10").await;
    assert!(titles(&prefix).contains(&"Echo Chamber".to_owned()));

    // Gained: the tokenizer folds diacritics, so "echo" now reaches "Écho".
    // The previous lowercase substring test did not.
    assert!(titles(&prefix).contains(&"Écho lointain".to_owned()));

    // Given up: matching inside a word. "cho" used to find "Echo Chamber"
    // through a substring test and no longer does. Documented, not accidental.
    let infix = subsonic_json(&router, "search3", api_key, "&query=cho&songCount=10").await;
    assert!(!titles(&infix).contains(&"Echo Chamber".to_owned()));

    // Terms narrow rather than widen.
    let narrowed = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Echo%20Silent&songCount=10",
    )
    .await;
    assert!(titles(&narrowed).is_empty());

    // An album reports its own size, not how much of it the query hit: two of
    // the three tracks match "echo", and songCount must still read 3.
    let album = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=Echo&albumCount=10&songCount=0",
    )
    .await;
    let albums = album["subsonic-response"]["searchResult3"]["album"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let night = albums
        .iter()
        .find(|album| album["name"] == "Night Sessions")
        .expect("matching album should be returned");
    assert_eq!(
        night["songCount"], 3,
        "songCount must describe the album, not the query"
    );

    // The documented match-all query still returns the whole catalogue.
    let all = subsonic_json(
        &router,
        "search3",
        api_key,
        "&query=%22%22&songCount=500&albumCount=500&artistCount=500",
    )
    .await;
    assert_eq!(titles(&all).len(), 3);
}

/// The three OpenSubsonic gaps the fifth audit named, pinned together because
/// they are one statement: what the catalogue can answer, it now says.
///
/// `sortName` moves from absent to present-and-possibly-empty — the presence
/// rule's difference between "not supported" and "not tagged". `song.parent`
/// stops naming a directory that would not list the song. And the native
/// search documents the 400 its required parameter already produced.
#[tokio::test]
async fn the_catalogue_answers_for_sort_names_and_for_songs_without_an_album() {
    let (_temp, config, state) = test_app().await;
    let subsonic_password = "subsonic-secret-123";
    let api_key = "wfsk_sortname-key";
    let admin = state
        .db
        .create_account(
            "sort-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state
        .secret_box
        .encrypt(subsonic_password.as_bytes())
        .unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("sort-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(admin, "Sorted", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 4, false).await.unwrap();

    // A tagged album, whose sort forms differ from the display forms — the
    // only case where the field carries information.
    let mut tagged = catalog_input(0, "The Nocturnes");
    tagged.title = "Opening".into();
    tagged.album = Some("The Night Sessions".into());
    tagged.album_artist = Some("The Nocturnes".into());
    tagged.is_compilation = false;
    tagged.sort_album = Some("Night Sessions, The".into());
    tagged.sort_album_artist = Some("Nocturnes, The".into());
    tagged.sort_artist = Some("Nocturnes, The".into());
    state
        .db
        .apply_catalog_track(library, scan, &tagged, None, false)
        .await
        .unwrap();

    // An untagged album by another artist: supported and unknown, which is not
    // the same statement as unsupported.
    let mut untagged = catalog_input(1, "Plain Ensemble");
    untagged.title = "Untitled".into();
    untagged.album = Some("Plain Record".into());
    untagged.album_artist = Some("Plain Ensemble".into());
    untagged.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &untagged, None, false)
        .await
        .unwrap();

    // And a track belonging to no album at all: the one that names its library
    // as its parent for want of an album id.
    let mut orphan = catalog_input(2, "Lone Voice");
    orphan.title = "Single Only".into();
    orphan.album = None;
    orphan.album_artist = None;
    orphan.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &orphan, None, false)
        .await
        .unwrap();
    // A second one, so the folder's ceiling has something to cut.
    let mut second_orphan = catalog_input(3, "Lone Voice");
    second_orphan.title = "Also Alone".into();
    second_orphan.album = None;
    second_orphan.album_artist = None;
    second_orphan.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &second_orphan, None, false)
        .await
        .unwrap();
    // Sort names are derived at the end of a scan, like the identifiers: the
    // scanner runs both passes here, so a test driving the catalogue directly
    // runs them too.
    state.db.consolidate_sort_names(library).await.unwrap();
    state.db.consolidate_musicbrainz_ids(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    // --- sortName on AlbumID3 -------------------------------------------
    let albums = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let albums = albums["subsonic-response"]["albumList2"]["album"]
        .as_array()
        .expect("the album list")
        .clone();
    let sort_of = |name: &str| -> String {
        albums
            .iter()
            .find(|album| album["name"] == name)
            .unwrap_or_else(|| panic!("{name} is listed"))["sortName"]
            .as_str()
            .expect("sortName is emitted for every album")
            .to_owned()
    };
    assert_eq!(sort_of("The Night Sessions"), "Night Sessions, The");
    // Emitted empty rather than omitted: the difference between a server that
    // cannot answer and an album no file supplied a sort tag for.
    assert_eq!(sort_of("Plain Record"), "");

    // --- sortName on ArtistID3 ------------------------------------------
    let artists = subsonic_json(&router, "getArtists", api_key, "").await;
    let mut seen = std::collections::BTreeMap::new();
    for index in artists["subsonic-response"]["artists"]["index"]
        .as_array()
        .expect("the artist index")
    {
        for artist in index["artist"].as_array().expect("an index holds artists") {
            seen.insert(
                artist["name"].as_str().unwrap().to_owned(),
                artist["sortName"]
                    .as_str()
                    .expect("sortName is emitted for every artist")
                    .to_owned(),
            );
        }
    }
    assert_eq!(
        seen.get("The Nocturnes").map(String::as_str),
        Some("Nocturnes, The")
    );
    assert_eq!(seen.get("Plain Ensemble").map(String::as_str), Some(""));

    // --- the parent of a song without an album --------------------------
    let directory = subsonic_json(
        &router,
        "getMusicDirectory",
        api_key,
        &format!("&id={library}"),
    )
    .await;
    let children = directory["subsonic-response"]["directory"]["child"]
        .as_array()
        .expect("the folder lists children")
        .clone();
    let orphan_child = children
        .iter()
        .find(|child| child["title"] == "Single Only")
        .expect("a song with no album is reachable by browsing its library");
    // The claim is coherence, not a new identifier: the song already said this
    // was its parent, and browsing there now finds it.
    assert_eq!(
        orphan_child["parent"].as_str(),
        Some(library.to_string()).as_deref()
    );
    assert_eq!(orphan_child["isDir"], serde_json::json!(false));
    // The artists of the library are still listed alongside it.
    assert!(
        children
            .iter()
            .any(|child| child["title"] == "The Nocturnes" || child["name"] == "The Nocturnes"),
        "the folder still lists its artists: {children:?}"
    );
    // An album's own track is not duplicated into the folder level.
    assert!(
        !children.iter().any(|child| child["title"] == "Opening"),
        "only album-less tracks belong at the folder level"
    );
    // Both album-less tracks are there: the ceiling the facade passes is a
    // bound on the answer, not a page the client has to ask again for.
    assert!(
        children.iter().any(|child| child["title"] == "Also Alone"),
        "every album-less track under the ceiling is listed: {children:?}"
    );

    // --- the folder listing is bounded ----------------------------------
    // `getMusicDirectory` takes no offset, so the query has to stop on its
    // own or a library of loose files answers with all of them at once.
    let capped = state
        .services
        .songs_without_album(admin, library, 1)
        .await
        .unwrap();
    assert_eq!(
        capped
            .iter()
            .map(|song| song.title.as_str())
            .collect::<Vec<_>>(),
        vec!["Also Alone"],
        "the ceiling cuts the listing in its own order, not at random"
    );
    let whole = state
        .services
        .songs_without_album(admin, library, 2_000)
        .await
        .unwrap();
    assert_eq!(whole.len(), 2, "a ceiling above the library cuts nothing");
    assert!(
        matches!(
            state.services.songs_without_album(admin, library, 0).await,
            Err(ServiceError::Invalid)
        ),
        "a ceiling of nothing is a caller error, not an empty folder"
    );

    // --- the same artist projection, wherever it is read ----------------
    // `getArtists`, the folder listing and the search each used to spell the
    // artist columns out for themselves, and one copy fell behind the day the
    // list gained `sortName`. They read one projection now, so the value that
    // reaches the index has to reach the search too.
    let searched = subsonic_json(&router, "search3", api_key, "&query=Nocturnes").await;
    let searched_artist = searched["subsonic-response"]["searchResult3"]["artist"]
        .as_array()
        .expect("the search answers with artists")
        .iter()
        .find(|artist| artist["name"] == "The Nocturnes")
        .expect("the searched artist is listed")
        .clone();
    assert_eq!(searched_artist["sortName"], "Nocturnes, The");

    // --- the native search's required parameter -------------------------
    let token = login_token(&router, "sort-admin", "correct horse battery staple").await;
    let missing_q = router
        .clone()
        .oneshot(
            Request::get("/api/v2/search")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        missing_q.status(),
        StatusCode::BAD_REQUEST,
        "the OpenAPI document now says 400, so the route must mean it"
    );

    // --- a sort tag removed from the files leaves the catalogue ---------
    // Writing the value during the per-track upsert could not do this: the
    // artist row is rewritten once per track, so a file with no tag had to be
    // stopped from erasing what a sibling supplied — and that preservation
    // outlived the tag. Deriving at the end of the scan is what makes removal
    // mean removal, exactly as it already does for the MusicBrainz ids.
    let rescan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(rescan, 1, false).await.unwrap();
    let mut untagged_now = tagged.clone();
    untagged_now.sort_album = None;
    untagged_now.sort_album_artist = None;
    untagged_now.sort_artist = None;
    // The same file, retagged: re-applied onto its own row rather than added
    // beside it, which is what a rescan of an edited file does.
    let existing = state
        .services
        .catalog_snapshot(admin, &[])
        .await
        .unwrap()
        .songs
        .into_iter()
        .find(|song| song.title == "Opening")
        .expect("the tagged track is in the catalogue")
        .id;
    state
        .db
        .apply_catalog_track(library, rescan, &untagged_now, Some(existing), false)
        .await
        .unwrap();
    state.db.consolidate_sort_names(library).await.unwrap();
    state.db.finish_scan_job(rescan, 0).await.unwrap();

    let after = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let after_album = after["subsonic-response"]["albumList2"]["album"]
        .as_array()
        .expect("the album list")
        .iter()
        .find(|album| album["name"] == "The Night Sessions")
        .expect("the album is still listed")
        .clone();
    assert_eq!(
        after_album["sortName"].as_str(),
        Some(""),
        "a sort tag removed from the files must leave the catalogue with it"
    );

    let after_artists = subsonic_json(&router, "getArtists", api_key, "").await;
    let mut after_seen = std::collections::BTreeMap::new();
    for index in after_artists["subsonic-response"]["artists"]["index"]
        .as_array()
        .expect("the artist index")
    {
        for artist in index["artist"].as_array().expect("an index holds artists") {
            after_seen.insert(
                artist["name"].as_str().unwrap().to_owned(),
                artist["sortName"].as_str().unwrap().to_owned(),
            );
        }
    }
    assert_eq!(
        after_seen.get("The Nocturnes").map(String::as_str),
        Some("")
    );
}

/// Two artist projections that are deliberately not the same shape, pinned in
/// both encodings so neither drifts into the other.
///
/// `artists[]` and `albumArtists[]` are *references*: an identifier and a
/// display name, and nothing else. The entries `getMusicDirectory` renders as
/// `child` are the artist and album nodes themselves, minus `musicBrainzId`,
/// so they do carry `sortName` and the rest. A field added to the node reaches
/// the second and must not leak into the first.
#[tokio::test]
async fn an_artist_reference_is_not_an_artist_record() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_reference-key";
    let admin = state
        .db
        .create_account(
            "reference-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state.secret_box.encrypt(b"subsonic-secret-123").unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("reference-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(admin, "Refs", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let scan = state
        .db
        .create_scan_job(library, Some(admin), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    let mut input = catalog_input(0, "The Nocturnes");
    input.title = "Opening".into();
    input.album = Some("The Night Sessions".into());
    input.album_artist = Some("The Nocturnes".into());
    input.is_compilation = false;
    input.sort_album = Some("Night Sessions, The".into());
    input.sort_album_artist = Some("Nocturnes, The".into());
    input.sort_artist = Some("Nocturnes, The".into());
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    state.db.consolidate_musicbrainz_ids(library).await.unwrap();
    state.db.consolidate_sort_names(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    // JSON: the album carries its own sortName; its `artists[]` entry carries
    // an identifier and a name, and no third key.
    let albums = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let album = albums["subsonic-response"]["albumList2"]["album"][0].clone();
    assert_eq!(album["sortName"], serde_json::json!("Night Sessions, The"));
    let reference_keys = |value: &serde_json::Value, what: &str| {
        let entry = value
            .as_object()
            .unwrap_or_else(|| panic!("{what} is an object: {value}"))
            .clone();
        let mut keys: Vec<String> = entry.keys().cloned().collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["id".to_owned(), "name".to_owned()],
            "{what} is a reference, not an ArtistID3: {entry:?}"
        );
    };
    reference_keys(&album["artists"][0], "an album's artists[] entry");

    // `albumArtists[]` is a reference too, and it is emitted on media items
    // rather than on the album — the album's own credit is `artists[]`. So it
    // is pinned on a song of the album.
    let album_json = subsonic_json(
        &router,
        "getAlbum",
        api_key,
        &format!("&id={}", album["id"].as_str().expect("the album id")),
    )
    .await;
    let song = album_json["subsonic-response"]["album"]["song"][0].clone();
    reference_keys(&song["artists"][0], "a song's artists[] entry");
    reference_keys(&song["albumArtists"][0], "a song's albumArtists[] entry");
    // A contributor's artist is a reference on the same terms, and the
    // contributor itself carries only what names the credit.
    if let Some(credit) = song["contributors"].as_array().and_then(|all| all.first()) {
        reference_keys(&credit["artist"], "a contributor's artist");
        let mut keys: Vec<String> = credit
            .as_object()
            .expect("a contributor is an object")
            .keys()
            .cloned()
            .collect();
        keys.sort_unstable();
        assert!(
            keys == vec!["artist".to_owned(), "role".to_owned()]
                || keys == vec!["artist".to_owned(), "role".to_owned(), "subRole".to_owned()],
            "a contributor names the credit and nothing more: {credit:?}"
        );
    }

    // XML: the same statement, in the encoding where an absent attribute is
    // absent rather than a missing key.
    let directory = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getMusicDirectory.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={library}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(directory.status(), StatusCode::OK);
    let directory = body_text(directory).await;
    // The browsing child is the artist node minus musicBrainzId, so the sort
    // name reaches it, carrying the tagged value.
    assert!(
        directory.contains(r#"sortName="Nocturnes, The""#),
        "a getMusicDirectory child carries the artist's sortName: {directory}"
    );
    // ...while the references carry neither that field nor any other one
    // belonging to the record. They live on the album and on its songs, so
    // this reads the album document rather than the folder listing — where a
    // library holding no album-less track has no song child at all.
    let album_xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbum.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={}",
                album["id"].as_str().expect("the album id")
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(album_xml.status(), StatusCode::OK);
    let album_xml = body_text(album_xml).await;
    for name in ["<artists ", "<albumArtists "] {
        let mut references = 0;
        for element in album_xml.split(name).skip(1) {
            let element = element.split("/>").next().unwrap_or_default();
            references += 1;
            // The whitelist, rather than a list of fields to reject: a field
            // added to the artist node has to fail here even if nobody thought
            // to name it.
            // Split on the quote rather than on whitespace: an attribute
            // value holds spaces, and `name="The Nocturnes"` would otherwise
            // read as two attributes.
            let mut attributes: Vec<String> = element
                .split('"')
                .step_by(2)
                .map(|key| key.trim().trim_end_matches('=').trim().to_owned())
                .filter(|key| !key.is_empty())
                .collect();
            // Sorted before comparing, like the JSON side: which attributes
            // are present is the contract, the order they are written in is
            // not, and pinning it would fail a reordering that changes
            // nothing observable.
            attributes.sort_unstable();
            assert_eq!(
                attributes,
                vec!["id".to_owned(), "name".to_owned()],
                "{name} is a reference, not an ArtistID3: {element}"
            );
        }
        assert!(
            references > 0,
            "the fixture must exercise {name}: {album_xml}"
        );
    }
}

/// A scan can be told to ignore what it already knows.
///
/// The skip is unconditional today, and it is right: it is why rescanning a
/// large library costs seconds. But nothing could ask for the work to be done
/// again, and a change to how the catalogue derives its identifiers needs
/// exactly that — the files have not moved, only the meaning of the rows has.
#[tokio::test]
async fn a_full_scan_reads_what_an_ordinary_one_would_skip() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("full-scan", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("full-scan-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Only Track.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Full", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Full".into(),
        root_path: root.clone(),
    };

    let job_of = |scan_id: uuid::Uuid| {
        let state = state.clone();
        async move {
            state
                .db
                .scan_job_for_user(owner, scan_id)
                .await
                .unwrap()
                .unwrap()
        }
    };
    let scan_once = |library: LibraryRecord| {
        let state = state.clone();
        async move {
            let id = state
                .scanner
                .trigger(library, Some(owner), "manual")
                .await
                .unwrap();
            for _ in 0..200 {
                let job = state
                    .db
                    .scan_job_for_user(owner, id)
                    .await
                    .unwrap()
                    .unwrap();
                if job.status == "completed" {
                    return id;
                }
                if job.status == "failed" {
                    panic!("scan failed: {:?}", job.message);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("scan timed out");
        }
    };

    let first = scan_once(library.clone()).await;
    assert_eq!(job_of(first).await.added, 1);

    // The second run recognises the file and does nothing, which is the
    // behaviour worth keeping.
    let second = scan_once(library.clone()).await;
    let second = job_of(second).await;
    assert_eq!(second.skipped, 1);
    assert_eq!(second.updated, 0);

    // Asking changes that, on a file that has not moved by a single byte.
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
    state.db.request_full_scan_everywhere().await.unwrap();
    assert!(state.db.full_scan_requested(library_id).await.unwrap());
    let third = scan_once(library.clone()).await;
    let third = job_of(third).await;
    assert_eq!(third.skipped, 0, "a full scan skips nothing");
    assert_eq!(third.updated, 1);

    // And the request is spent, so the next run is ordinary again.
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
    let fourth = scan_once(library).await;
    assert_eq!(job_of(fourth).await.skipped, 1);
}

/// The request outlives a run that does not finish.
///
/// This is the whole reason it is a stored state rather than an argument. A
/// migration scan interrupted halfway has rewritten some rows under the new
/// scheme and left the rest under the old one; if the request died with the
/// run, the next scan would skip every remaining file — on the grounds that
/// their bytes had not changed — and freeze the catalogue in two halves.
#[tokio::test]
async fn a_full_scan_request_survives_a_scan_that_never_completes() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("interrupted", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("interrupted-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(
            owner,
            "Interrupted",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    state.db.request_full_scan_everywhere().await.unwrap();
    let failed = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(failed, 1, true).await.unwrap();
    state.db.fail_scan_job(failed, "interrupted").await.unwrap();
    assert!(
        state.db.full_scan_requested(library_id).await.unwrap(),
        "a failed run leaves the request standing"
    );

    // Only a completed run spends it.
    let completed = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(completed, 1, true).await.unwrap();
    state.db.finish_scan_job(completed, 0).await.unwrap();
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
}

/// The instance remembers its own settings between boots.
#[tokio::test]
async fn server_properties_round_trip_and_overwrite() {
    let (_temp, _config, state) = test_app().await;
    assert_eq!(state.db.server_property("pid.album").await.unwrap(), None);
    state
        .db
        .set_server_property("pid.album", "albumartistid,album")
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .server_property("pid.album")
            .await
            .unwrap()
            .as_deref(),
        Some("albumartistid,album")
    );
    state
        .db
        .set_server_property("pid.album", "musicbrainz_albumid")
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .server_property("pid.album")
            .await
            .unwrap()
            .as_deref(),
        Some("musicbrainz_albumid")
    );
}

/// A catalogue keyed under a rule the server no longer follows.
///
/// Nothing about the files reveals this: their bytes and timestamps are
/// unchanged, only the rule reading them moved. Comparing what the last scan
/// recorded against what this instance is configured with is the only way the
/// difference is visible at all.
#[tokio::test]
async fn a_changed_identity_rule_schedules_a_full_rescan_everywhere() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("identity", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    for name in ["First", "Second"] {
        let root = config.data_dir.join(name.to_lowercase());
        std::fs::create_dir_all(&root).unwrap();
        state
            .db
            .create_library(
                owner,
                name,
                &std::fs::canonicalize(&root).unwrap(),
                LibraryVisibility::Private,
                now_ms(),
            )
            .await
            .unwrap();
    }

    // A catalogue built before the property existed is an older server, not a
    // different rule: nothing is scheduled.
    assert_eq!(
        state
            .db
            .reconcile_catalog_identity(&config.pid)
            .await
            .unwrap(),
        0
    );

    // Once a scan has recorded what it used, agreeing costs nothing either.
    state
        .db
        .set_server_property("pid.album", config.pid.album.source())
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .reconcile_catalog_identity(&config.pid)
            .await
            .unwrap(),
        0
    );

    // Disagreeing marks every library, because the rule is instance-wide.
    state
        .db
        .set_server_property("pid.album", "folder")
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .reconcile_catalog_identity(&config.pid)
            .await
            .unwrap(),
        2
    );
    let libraries = state.db.libraries_for_user(owner).await.unwrap();
    assert_eq!(libraries.len(), 2);
    for library in libraries {
        assert!(
            state.db.full_scan_requested(library.id).await.unwrap(),
            "{} was asked to rescan in full",
            library.name
        );
    }
}

/// The separators the reference cuts on, reaching the catalogue.
///
/// The old rule cut on `;` and nothing else, so a file crediting
/// "Nova Kern / Lior Sand" held one artist named after the whole string. The
/// new one cuts on a padded slash — padded so that `AC/DC`, which is one band,
/// survives it.
#[tokio::test]
async fn the_catalogue_cuts_a_credit_where_the_reference_cuts_it() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("separators", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("separator-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Separators",
            &root,
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
    state.db.start_scan_job(scan, 3, false).await.unwrap();

    for (index, credit, album) in [
        (0usize, "Nova Kern / Lior Sand", "Split By Slash"),
        (1, "AC/DC", "Kept Whole"),
        (2, "Ada Vale feat. Nova Kern", "Split By Feat"),
    ] {
        let mut input = catalog_input(index, credit);
        input.title = format!("Track {index}");
        input.album = Some(album.into());
        input.album_artist = Some(credit.into());
        input.is_compilation = false;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .into_iter()
        .map(|summary| summary.artist.name)
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "AC/DC".to_owned(),
            "Ada Vale".to_owned(),
            "Lior Sand".to_owned(),
            "Nova Kern".to_owned(),
        ],
        "a padded slash cuts, a bare one inside a name does not, and `feat.` cuts"
    );
}

#[tokio::test]
async fn the_participant_schema_replaced_the_single_artist_relation() {
    let (_temp, _config, state) = test_app().await;
    let table_exists = |name: &'static str| {
        let pool = state.db.pool().clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
            )
            .bind(name)
            .fetch_one(&pool)
            .await
            .unwrap()
                == 1
        }
    };
    assert!(table_exists("track_participant").await);
    assert!(table_exists("album_participant").await);
    assert!(table_exists("artist_role_stats").await);
    assert!(
        !table_exists("track_artist").await,
        "the single-artist relation is gone, not shadowed"
    );
}

/// A producer is a credit, not an artist of the track.
///
/// This is the failure the participants model makes possible and no existing
/// test could have anticipated: widening `track_participant` to hold every
/// role, and leaving one projection without its role predicate, does not turn
/// the suite red — it leaves it green while every song reports its producer
/// among its artists, and every album reports them among its own.
#[tokio::test]
async fn a_contributor_is_not_one_of_the_track_artists() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("contributors", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("contributor-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Credits",
            &root,
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

    let mut input = catalog_input(0, "Nova Kern");
    input.title = "Only Track".into();
    input.album = Some("Only Album".into());
    input.album_artist = Some("Nova Kern".into());
    input.is_compilation = false;
    input.roles = vec![
        (
            waveflow_server::tags::Role::Producer,
            vec!["Rita Sound".into()],
        ),
        (
            waveflow_server::tags::Role::Composer,
            vec!["Otto Pen".into()],
        ),
    ];
    input.performer_pairs = vec![("guitar".into(), "Jimmy Page".into())];
    state
        .db
        .apply_catalog_track(library, scan, &input, None, false)
        .await
        .unwrap();
    state.db.consolidate_musicbrainz_ids(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();

    // Every credit reached the catalogue...
    let artists = state
        .services
        .list_artists(owner, None, Default::default())
        .await
        .unwrap();
    let mut names: Vec<String> = artists
        .iter()
        .map(|summary| summary.artist.name.clone())
        .collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "Jimmy Page".to_owned(),
            "Nova Kern".to_owned(),
            "Otto Pen".to_owned(),
            "Rita Sound".to_owned(),
        ]
    );

    // ...and none of them is one of the track's artists.
    let songs = state
        .services
        .songs_without_album(owner, library, 100)
        .await
        .unwrap();
    let song = state
        .services
        .album(owner, {
            let albums = state
                .services
                .list_albums(owner, &Default::default())
                .await
                .unwrap();
            albums[0].id
        })
        .await
        .unwrap();
    assert!(songs.is_empty(), "the track belongs to an album");
    let track = &song.songs[0];
    assert_eq!(
        track
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Nova Kern"],
        "artists[] holds the track's own credit and nothing else"
    );
    assert_eq!(
        song.album
            .artists
            .iter()
            .map(|artist| artist.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Nova Kern"],
        "an album's artists[] holds its album artists, not every contributor"
    );
    let credited = state
        .services
        .artist(
            owner,
            artists
                .iter()
                .find(|summary| summary.artist.name == "Rita Sound")
                .expect("the producer was indexed")
                .artist
                .id,
        )
        .await
        .unwrap();
    assert_eq!(
        credited.album_count, 0,
        "a producer holds no album of their own"
    );
    assert!(credited.albums.is_empty());
}

/// The credits OpenSubsonic asks for, and the presence rule they follow.
///
/// These three fields were absent because the columns they need did not
/// exist — and under the presence rule absent is a statement: it says the
/// server does not read them. Now that it does, they are emitted with their
/// default on a track that names nobody, which is the difference between
/// "unsupported" and "this file credits no composer".
#[tokio::test]
async fn credits_reach_the_wire_in_both_encodings() {
    let (_temp, config, state) = test_app().await;
    let api_key = "wfsk_credits-key";
    let admin = state
        .db
        .create_account(
            "credit-admin",
            &security::hash_password("correct horse battery staple").unwrap(),
            AccountRole::Admin,
            now_ms(),
        )
        .await
        .unwrap();
    let encrypted = state.secret_box.encrypt(b"subsonic-secret-123").unwrap();
    state
        .db
        .set_subsonic_credential(
            admin,
            admin,
            &encrypted,
            &security::token_hash(api_key),
            now_ms(),
        )
        .await
        .unwrap();
    let music = config.data_dir.join("credit-wire");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Credits",
            &root,
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
    state.db.start_scan_job(scan, 2, false).await.unwrap();

    let mut credited = catalog_input(0, "Nova Kern");
    credited.title = "Credited".into();
    credited.album = Some("The Record".into());
    credited.album_artist = Some("Nova Kern; Lior Sand".into());
    credited.is_compilation = false;
    credited.roles = vec![
        (
            waveflow_server::tags::Role::Composer,
            vec!["Otto Pen; Ada Vale".into()],
        ),
        (
            waveflow_server::tags::Role::Producer,
            vec!["Rita Sound".into()],
        ),
    ];
    credited.performer_pairs = vec![("guitar".into(), "Jimmy Page".into())];
    state
        .db
        .apply_catalog_track(library, scan, &credited, None, false)
        .await
        .unwrap();

    let mut bare = catalog_input(1, "Nova Kern");
    bare.title = "Bare".into();
    bare.album = Some("The Record".into());
    bare.album_artist = Some("Nova Kern; Lior Sand".into());
    bare.is_compilation = false;
    state
        .db
        .apply_catalog_track(library, scan, &bare, None, false)
        .await
        .unwrap();
    state.db.consolidate_musicbrainz_ids(library).await.unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let albums = subsonic_json(
        &router,
        "getAlbumList2",
        api_key,
        "&type=alphabeticalByName",
    )
    .await;
    let album_id = albums["subsonic-response"]["albumList2"]["album"][0]["id"]
        .as_str()
        .expect("the album is listed")
        .to_owned();
    let album = subsonic_json(&router, "getAlbum", api_key, &format!("&id={album_id}")).await;
    let songs = album["subsonic-response"]["album"]["song"]
        .as_array()
        .expect("the album lists its songs")
        .clone();
    let song = |title: &str| {
        songs
            .iter()
            .find(|song| song["title"] == title)
            .unwrap_or_else(|| panic!("missing {title}"))
            .clone()
    };

    let credited = song("Credited");
    let contributors = credited["contributors"]
        .as_array()
        .expect("contributors is an array");
    let mut named: Vec<(String, String)> = contributors
        .iter()
        .map(|credit| {
            (
                credit["role"].as_str().unwrap().to_owned(),
                credit["artist"]["name"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    named.sort();
    assert_eq!(
        named,
        vec![
            ("composer".to_owned(), "Ada Vale".to_owned()),
            ("composer".to_owned(), "Otto Pen".to_owned()),
            ("performer".to_owned(), "Jimmy Page".to_owned()),
            ("producer".to_owned(), "Rita Sound".to_owned()),
        ],
        "every role but artist and albumartist is a contributor"
    );
    let performer = contributors
        .iter()
        .find(|credit| credit["role"] == "performer")
        .expect("the performer is credited");
    assert_eq!(
        performer["subRole"], "Guitar",
        "a performer carries the instrument, title-cased"
    );
    assert_eq!(
        credited["displayComposer"], "Otto Pen \u{2022} Ada Vale",
        "the composers, in tag order, joined the way the reference joins them"
    );
    let album_artists: Vec<&str> = credited["albumArtists"]
        .as_array()
        .expect("albumArtists is an array")
        .iter()
        .map(|artist| artist["name"].as_str().unwrap())
        .collect();
    assert_eq!(album_artists, vec!["Nova Kern", "Lior Sand"]);

    let bare = song("Bare");
    assert_eq!(bare["contributors"], serde_json::json!([]));
    assert_eq!(bare["displayComposer"], "");

    let artists = subsonic_json(&router, "getArtists", api_key, "").await;
    let indexed: Vec<serde_json::Value> = artists["subsonic-response"]["artists"]["index"]
        .as_array()
        .expect("the artist index")
        .iter()
        .flat_map(|index| index["artist"].as_array().unwrap().clone())
        .collect();
    let named = |name: &str| {
        indexed
            .iter()
            .find(|artist| artist["name"] == name)
            .unwrap_or_else(|| panic!("{name} is indexed"))
            .clone()
    };
    assert_eq!(
        named("Nova Kern")["roles"],
        serde_json::json!(["albumartist", "artist"]),
        "an album artist who also performs says both, in a stable order"
    );
    // A composer holds no album, so the index does not list them — but the
    // artist is still in the catalogue, and still says what it is.
    assert!(
        !indexed.iter().any(|artist| artist["name"] == "Otto Pen"),
        "an artist credited on no album is not one of the library's artists"
    );
    let catalogue = state
        .services
        .list_artists(admin, None, Default::default())
        .await
        .unwrap();
    let composer = catalogue
        .iter()
        .find(|summary| summary.artist.name == "Otto Pen")
        .expect("the composer is in the catalogue");
    assert_eq!(composer.artist.roles, vec!["composer".to_owned()]);

    let xml = router
        .clone()
        .oneshot(
            Request::get(format!(
                "/rest/getAlbum.view?apiKey={api_key}&v=1.16.1&c=fixtures&id={album_id}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let xml = body_text(xml).await;
    assert!(
        xml.contains("<contributors role=\"performer\" subRole=\"Guitar\">"),
        "the performer credit is an element carrying its instrument: {xml}"
    );
    assert!(
        xml.contains("displayComposer=\"Otto Pen \u{2022} Ada Vale\""),
        "the composer display string reaches XML too: {xml}"
    );
}
