//! The backup bundle, and a refused boot that gives the database back.

use super::*;

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
        waveflow_server::storage::database::Database::check_file(&bundle.join("waveflow.db"))
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

/// A boot that is refused closes the database before it says so.
///
/// Dropping a pool is not closing it: sqlx closes each connection on its own
/// thread after the drop has returned, and the last of them checkpoints the
/// WAL under an exclusive lock. A refusal that only dropped left whoever opens
/// the file next — the operator retrying, or a test right after the refusal —
/// racing that close, and losing it as `database is locked`. A zero
/// `busy_timeout` makes the next open fail rather than wait, so a lock still
/// held is seen however briefly it lasts.
#[tokio::test]
async fn a_refused_boot_gives_the_database_back_before_it_returns() {
    let (_temp, config, state) = test_app().await;
    state.db.pool().close().await;
    drop(state);
    std::fs::write(&config.instance_key_path, [7u8; 32]).unwrap();
    let mut impatient = config.clone();
    impatient.sqlite_busy_timeout = std::time::Duration::ZERO;

    for attempt in 0..30 {
        let refusal = match waveflow_server::initialize(&config).await {
            Ok(_) => panic!("a key that is not the database's must stop the boot"),
            Err(error) => format!("{error:#}"),
        };
        assert!(
            refusal.contains("instance.key does not match"),
            "attempt {attempt}: refused for another reason: {refusal}"
        );
        let next = waveflow_server::storage::database::Database::open(&impatient)
            .await
            .unwrap_or_else(|error| {
                panic!("attempt {attempt}: the refused boot still held the database: {error}")
            });
        next.pool().close().await;
    }
}
