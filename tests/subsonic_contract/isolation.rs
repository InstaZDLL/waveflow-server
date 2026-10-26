//! What a foreign catalogue may not see, and the limit on failed
//! authentication.

use super::*;

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
