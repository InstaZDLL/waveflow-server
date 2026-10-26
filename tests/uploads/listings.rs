//! What the listings say about uploads: a received file, and a library that
//! takes files.

use super::*;

/// A client can tell its own upload from a discovery.
///
/// Without the origin device the feed says only "this track changed", so the
/// client that sent the file reads it back as one it has just found and treats
/// it as a discovery. `sync_event` has carried the same fact since its first
/// day, for the same reason.
#[tokio::test]
async fn a_client_can_tell_its_own_upload_from_a_discovery() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let password = security::generate_token("test-password-");
    let hashed = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("attributed", &hashed, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("attributed");
    std::fs::create_dir_all(&music).unwrap();
    // One track the scan finds by itself, so the feed holds both kinds.
    write_test_wav(&music.join("Found by the scan.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Attributed",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    state
        .db
        .set_library_accepts_uploads(owner, library, true, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state.clone());
    let (token, device) = login_session(&router, "attributed", &password).await;
    run_scan(
        &state,
        owner,
        waveflow_server::catalog::LibraryRecord {
            id: library,
            name: "Attributed".into(),
            root_path: root,
        },
    )
    .await;

    let fixture = Fixture {
        owner,
        library,
        token: token.clone(),
    };
    let source = config.data_dir.join("attributed-source.wav");
    write_test_wav_of_len(&source, 900);
    let bytes = std::fs::read(&source).unwrap();
    std::fs::remove_file(&source).unwrap();
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (sent, _) = put_chunk(&config, &state, &token, &session, 0, &bytes).await;
    assert_eq!(sent, StatusCode::OK);

    // A device belonging to somebody else is refused outright: an unchecked
    // header would let one account attribute its writes to another account's
    // device, and every client filtering its own changes out of the feed would
    // then drop somebody else's.
    let stranger_password = security::generate_token("test-password-");
    let stranger_hash = security::hash_password(&stranger_password).unwrap();
    state
        .db
        .create_account(
            "attributed-stranger",
            &stranger_hash,
            AccountRole::User,
            now_ms(),
        )
        .await
        .unwrap();
    let (_, stranger_device) =
        login_session(&router, "attributed-stranger", &stranger_password).await;
    let borrowed = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v2/uploads/{session}/commit"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-waveflow-device-id", &stranger_device)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(borrowed.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let committed = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/v2/uploads/{session}/commit"))
                .header("authorization", format!("Bearer {token}"))
                .header("x-waveflow-device-id", &device)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(committed.status(), StatusCode::CREATED);
    let uploaded = json_body(committed).await["track_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let feed = router
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/v2/libraries/{library}/events?after=0&limit=100"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(feed.status(), StatusCode::OK);
    let events = json_body(feed).await;
    let events = events["events"].as_array().unwrap();

    let mine = events
        .iter()
        .find(|event| event["entity_id"] == uploaded.as_str())
        .expect("the upload is on the feed");
    assert_eq!(
        mine["origin_device_id"], device,
        "the client that sent the file is named"
    );
    let scanned: Vec<_> = events
        .iter()
        .filter(|event| event["entity_id"] != uploaded.as_str())
        .collect();
    assert!(!scanned.is_empty(), "the scan wrote events too");
    for event in scanned {
        assert!(
            event["origin_device_id"].is_null(),
            "a scan is nobody's request in particular: {event}"
        );
    }
}

/// Whether a library takes files is on the list of libraries, and it follows
/// the operator's switch.
///
/// A client that could not read it would offer an upload in every library and
/// learn `library_closed` only after hashing the whole file.
#[tokio::test]
async fn the_library_list_says_whether_a_library_takes_files() {
    let (_temp, config, state) = upload_app(|_| {}).await;
    let fixture = fixture(&config, &state, "flag-reader").await;
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

    assert_eq!(
        listed().await[0]["accepts_uploads"],
        false,
        "a library is closed until the operator opens it"
    );
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    assert_eq!(listed().await[0]["accepts_uploads"], true);
}
