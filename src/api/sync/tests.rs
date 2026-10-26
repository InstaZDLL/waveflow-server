use super::{sync_notice_action, SyncNoticeAction};

/// A lagged socket recovers the cursor from the journal, not from a default.
///
/// The distinction needs a non-zero cursor to be visible at all: a user with
/// no events answers 0, which is also what returning a constant would give,
/// so the empty case alone proves only that the branch is wired to
/// something. The event below is written through the real journal path, so
/// the expected value is one the test never chose.
#[tokio::test]
async fn lagged_sync_socket_recovers_from_the_durable_cursor() {
    let temp = tempfile::tempdir().unwrap();
    let config = crate::Config::for_data_dir(temp.path().join("data"));
    let db = crate::storage::database::Database::open(&config)
        .await
        .unwrap();
    db.migrate().await.unwrap();
    let user_id = db
        .create_account(
            "lagged",
            "hash",
            crate::storage::database::AccountRole::User,
            crate::auth::authentication::now_ms(),
        )
        .await
        .unwrap();
    let sync = crate::storage::sync::SyncService::new(db.clone());

    let context = crate::storage::sync::MutationContext::server_generated();
    let writer = db.writer_guard().await;
    let mut tx = db.begin_write(&writer).await.unwrap();
    sync.claim_operation(
        &writer,
        &mut tx,
        user_id,
        context,
        crate::storage::sync::MutationIntent::new(
            "set-rating",
            "track:seed",
            &serde_json::json!({}),
        ),
    )
    .await
    .unwrap();
    let receipt = sync
        .complete_operation(
            &mut tx,
            user_id,
            context,
            "rating",
            uuid::Uuid::new_v4(),
            "upsert",
            &serde_json::json!({}),
            None,
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    drop(writer);
    assert_ne!(receipt.cursor, 0);

    let action = sync_notice_action(
        &sync,
        user_id,
        Err(tokio::sync::broadcast::error::RecvError::Lagged(3)),
    )
    .await
    .unwrap();
    assert_eq!(action, SyncNoticeAction::Send(receipt.cursor));

    // An account with nothing in the journal still falls back to the base
    // cursor rather than failing.
    let action = sync_notice_action(
        &sync,
        uuid::Uuid::new_v4(),
        Err(tokio::sync::broadcast::error::RecvError::Lagged(3)),
    )
    .await
    .unwrap();
    assert_eq!(action, SyncNoticeAction::Send(0));
}
