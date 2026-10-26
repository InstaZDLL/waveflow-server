//! First-run setup and its token, and the native administration it opens onto.

use super::*;

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

    // RFC-019: the boot drew a setup token and left it in the data directory.
    let setup_token = setup_token_on_disk(&config);
    let mut request = json_request(
        "/api/v2/setup",
        serde_json::json!({
            "username": "first-admin",
            "password": &admin_password,
            "setup_token": setup_token,
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

/// The token a boot drew for first-run setup, read where the operator reads it.
fn setup_token_on_disk(config: &waveflow_server::Config) -> String {
    std::fs::read_to_string(
        config
            .data_dir
            .join(waveflow_server::setup_token::FILE_NAME),
    )
    .expect("a fresh instance writes its setup token")
    .trim()
    .to_owned()
}

/// A setup request the origin check accepts, so only the token is in question.
fn setup_request(body: serde_json::Value) -> Request<Body> {
    let mut request = json_request("/api/v2/setup", body);
    request
        .headers_mut()
        .insert("origin", "http://waveflow.test".parse().unwrap());
    request
        .headers_mut()
        .insert("host", "waveflow.test".parse().unwrap());
    request
}

/// RFC-019 decision 1: on a fresh instance, a request that passes the origin
/// check but lacks the token creates nothing — whatever else it gets right.
#[tokio::test]
async fn setup_is_refused_without_the_token_and_forgets_it_once_used() {
    let (_temp, config, state) = test_app().await;
    let router = waveflow_server::app(&config, state.clone());
    let token_file = config
        .data_dir
        .join(waveflow_server::setup_token::FILE_NAME);
    let token = setup_token_on_disk(&config);
    let password = Uuid::new_v4().to_string();

    for presented in [
        None,
        Some(String::new()),
        Some(Uuid::new_v4().to_string()),
        // One character short, and one too many: a prefix is not the token.
        Some(token[..token.len() - 1].to_owned()),
        Some(format!("{token}x")),
    ] {
        let mut body = serde_json::json!({ "username": "intruder", "password": &password });
        if let Some(presented) = &presented {
            body["setup_token"] = serde_json::Value::String(presented.clone());
        }
        let refused = router.clone().oneshot(setup_request(body)).await.unwrap();
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{presented:?}");
        assert_eq!(
            json_body(refused).await["code"],
            "setup_token_rejected",
            "{presented:?}"
        );
    }
    assert!(
        state.db.setup_required().await.unwrap(),
        "nothing was created"
    );

    // A refused password leaves setup open, token and all.
    let short = router
        .clone()
        .oneshot(setup_request(serde_json::json!({
            "username": "first-admin",
            "password": &password[..8],
            "setup_token": &token,
        })))
        .await
        .unwrap();
    assert_eq!(short.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(token_file.exists());

    let created = router
        .clone()
        .oneshot(setup_request(serde_json::json!({
            "username": "first-admin",
            "password": &password,
            "setup_token": &token,
        })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    assert!(!token_file.exists(), "a used token stays on disk");

    // The same token, again: setup is closed, not merely refused.
    let again = router
        .clone()
        .oneshot(setup_request(serde_json::json!({
            "username": "second-admin",
            "password": &password,
            "setup_token": &token,
        })))
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::CONFLICT);

    // And the next boot, finding an account, draws no new one.
    waveflow_server::initialize(&config).await.unwrap();
    assert!(!token_file.exists(), "a boot with an account drew a token");
}

/// An operator who chose the token in `WAVEFLOW_SETUP_TOKEN` gets that one,
/// and no file: the drawn token is not a second way in.
#[tokio::test]
async fn an_operator_chosen_setup_token_replaces_the_drawn_one() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    let chosen = Uuid::new_v4().to_string();
    config.setup_token = Some(chosen.clone());
    let state = waveflow_server::initialize(&config).await.unwrap();
    let router = waveflow_server::app(&config, state);
    assert!(!config
        .data_dir
        .join(waveflow_server::setup_token::FILE_NAME)
        .exists());

    let password = Uuid::new_v4().to_string();
    let created = router
        .oneshot(setup_request(serde_json::json!({
            "username": "first-admin",
            "password": &password,
            "setup_token": &chosen,
        })))
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
}
