//! The first library, made at setup from `WAVEFLOW_MUSIC_DIR`.
//!
//! A fresh Docker install mounts the music at `/music` and then opened on
//! empty pages: no library existed until an administrator found the form and
//! typed the path the compose file had already named, and until then uploads
//! had nowhere to go and folders made under the mount showed nowhere.

use super::*;

/// A fresh instance whose first library would be `music_dir`.
async fn app_with_music_dir(
    music_dir: Option<std::path::PathBuf>,
) -> (
    tempfile::TempDir,
    waveflow_server::Config,
    waveflow_server::AppState,
) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.music_dir = music_dir.map(|dir| temp.path().join(dir));
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

/// Creates the first administrator through the setup screen's route.
async fn run_setup(
    config: &waveflow_server::Config,
    router: &axum::Router,
    password: &str,
) -> serde_json::Value {
    let token = std::fs::read_to_string(
        config
            .data_dir
            .join(waveflow_server::auth::setup_token::FILE_NAME),
    )
    .unwrap();
    let mut request = json_request(
        "/api/v2/setup",
        serde_json::json!({
            "username": "first-admin",
            "password": password,
            "setup_token": token.trim(),
        }),
    );
    request
        .headers_mut()
        .insert("origin", "http://waveflow.test".parse().unwrap());
    request
        .headers_mut()
        .insert("host", "waveflow.test".parse().unwrap());
    let response = router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await
}

#[tokio::test]
async fn setup_registers_the_music_folder_as_the_first_library() {
    let (temp, config, state) = app_with_music_dir(Some("music".into())).await;
    let music = temp.path().join("music");
    std::fs::create_dir_all(music.join("Made by hand")).unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let password = Uuid::new_v4().to_string();

    let created = run_setup(&config, &router, &password).await;
    let library_id: Uuid = created["library"]["library_id"]
        .as_str()
        .expect("setup answers the library it made")
        .parse()
        .unwrap();
    assert!(
        created["library"]["scan_id"].is_string(),
        "and the scan it queued"
    );

    let (owner, name, root, visibility): (String, String, String, String) = sqlx::query_as(
        "SELECT owner_user_id, name, root_path, visibility FROM library WHERE id = ?",
    )
    .bind(library_id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(owner, created["user_id"].as_str().unwrap());
    assert_eq!(name, "Music");
    assert_eq!(
        std::path::PathBuf::from(root),
        std::fs::canonicalize(&music).unwrap()
    );
    assert_eq!(visibility, "private", "shared only when the owner says so");

    // The administrator sees it at once, as the web player asks.
    let token = login_token(&router, "first-admin", &password).await;
    let listed = router
        .oneshot(
            Request::get("/api/v2/libraries")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let listed = json_body(listed).await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["id"], library_id.to_string());
}

#[tokio::test]
async fn a_missing_music_folder_leaves_setup_to_succeed_without_a_library() {
    // Named, never created: a server run outside the image, with the setting
    // left pointing at a folder this host does not have.
    let (_temp, config, state) = app_with_music_dir(Some("not-mounted".into())).await;
    let router = waveflow_server::app(&config, state.clone());

    let created = run_setup(&config, &router, &Uuid::new_v4().to_string()).await;
    assert!(created["library"].is_null());
    assert!(
        created["user_id"].is_string(),
        "the account is made all the same"
    );
    assert!(state.db.all_libraries().await.unwrap().is_empty());
}

#[tokio::test]
async fn without_a_music_folder_setup_makes_no_library() {
    let (_temp, config, state) = app_with_music_dir(None).await;
    let router = waveflow_server::app(&config, state.clone());

    let created = run_setup(&config, &router, &Uuid::new_v4().to_string()).await;
    assert!(created["library"].is_null());
    assert!(state.db.all_libraries().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_music_folder_that_is_a_file_is_passed_over() {
    let (temp, config, state) = app_with_music_dir(Some("music".into())).await;
    std::fs::write(temp.path().join("music"), b"not a folder").unwrap();
    let router = waveflow_server::app(&config, state.clone());

    let created = run_setup(&config, &router, &Uuid::new_v4().to_string()).await;
    assert!(created["library"].is_null());
    assert!(state.db.all_libraries().await.unwrap().is_empty());
}
