use std::time::Duration;

use uuid::Uuid;

/// A share holding one catalogue row, written straight into the tables:
/// what this test needs is a share the services can read, not a scan.
async fn a_share(state: &crate::AppState) -> (String, Uuid, Uuid) {
    let pool = state.db.pool();
    let (account, library, track, share) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    // A placeholder hash, never verified: putting a value through Argon2
    // in a test is what CodeQL reports as a hard-coded credential.
    for (statement, binds) in [
        (
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
             VALUES (?, 'gate-owner', 'placeholder', 'user', 1, 1)",
            vec![account.to_string()],
        ),
        (
            "INSERT INTO library (id, owner_user_id, name, root_path, visibility, \
                                  created_at, updated_at) \
             VALUES (?, ?, 'Gate', '/music/gate', 'private', 1, 1)",
            vec![library.to_string(), account.to_string()],
        ),
        (
            "INSERT INTO library_member (library_id, user_id, role, created_at) \
             VALUES (?, ?, 'owner', 1)",
            vec![library.to_string(), account.to_string()],
        ),
        (
            "INSERT INTO track (id, library_id, relative_path, file_size, file_modified_at, \
                                quick_hash, full_hash, title, duration_ms, is_available, \
                                created_at, updated_at) \
             VALUES (?, ?, 'gate.flac', 1, 1, ?, ?, 'Gate', 1000, 1, 1, 1)",
            vec![
                track.to_string(),
                library.to_string(),
                "3".repeat(64),
                "4".repeat(64),
            ],
        ),
    ] {
        let mut query = sqlx::query(statement);
        for bind in binds {
            query = query.bind(bind);
        }
        query.execute(pool).await.expect(statement);
    }
    let token = state.secret_box.derive_share_token(share);
    sqlx::query(
        "INSERT INTO share (id, owner_user_id, token_hash, created_at, updated_at) \
         VALUES (?, ?, ?, 1, 1)",
    )
    .bind(share.to_string())
    .bind(account.to_string())
    .bind(crate::security::token_hash(&token).as_slice())
    .execute(pool)
    .await
    .expect("share");
    sqlx::query("INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, 0)")
        .bind(share.to_string())
        .bind(track.to_string())
        .execute(pool)
        .await
        .expect("share track");
    (token, share, track)
}

/// A stream request on a share does not wait for the writer gate
/// (RFC-020 decision 4).
///
/// The gate is held here the way a scan holds it. The opening of the
/// share, which counts a visit, is asserted to wait for it — which is
/// what proves this test can see the gate at all — and the stream lookup
/// is asserted not to.
#[tokio::test]
async fn a_stream_request_on_a_share_does_not_wait_for_the_writer() {
    let temp = tempfile::tempdir().expect("temporary directory");
    let config = crate::Config::for_data_dir(temp.path().join("data"));
    let state = crate::initialize(&config).await.expect("initialize");
    let (token, share, track) = a_share(&state).await;
    let owner = state
        .services
        .public_share(&token)
        .await
        .expect("the share opens")
        .owner_id;

    let held = state.db.writer_guard().await;
    let wait = Duration::from_millis(500);
    assert!(
        tokio::time::timeout(wait, state.services.public_share(&token))
            .await
            .is_err(),
        "opening a share counts a visit, so it must wait for the writer"
    );
    let streamed = tokio::time::timeout(
        wait,
        state.services.shared_track_owner(&token, share, track),
    )
    .await
    .expect("a stream request must not wait for the writer gate")
    .expect("the shared track resolves");
    assert_eq!(streamed, owner);
    drop(held);
}

/// The opening of a share is bounded by the request timeout, whoever holds
/// the writer gate.
///
/// It counts a visit, so it waits for the gate, and a scan holds that gate
/// for as long as it writes. The route was merged beside the media routes,
/// outside every timeout, so an anonymous opening during a long scan kept
/// its connection for the length of the scan. Held here as a scan holds
/// it: the opening must answer 408 once the configured timeout passes, not
/// wait for the gate to come back.
#[tokio::test]
async fn an_opening_waiting_on_the_writer_gate_times_out() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    let temp = tempfile::tempdir().expect("temporary directory");
    let mut config = crate::Config::for_data_dir(temp.path().join("data"));
    config.request_timeout = Duration::from_millis(200);
    let state = crate::initialize(&config).await.expect("initialize");
    let (token, ..) = a_share(&state).await;
    let router = crate::app(&config, state.clone());

    let held = state.db.writer_guard().await;
    let opened = tokio::time::timeout(
        Duration::from_secs(10),
        router.oneshot(
            Request::get(format!("/share/{token}"))
                .body(Body::empty())
                .unwrap(),
        ),
    )
    .await
    .expect("the opening must not wait for the writer gate past the timeout")
    .expect("a response");
    assert_eq!(opened.status(), axum::http::StatusCode::REQUEST_TIMEOUT);
    drop(held);
}
