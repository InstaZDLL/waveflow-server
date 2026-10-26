//! The library commands: a full rescan, and renaming, moving, purging and
//! deleting a library.

use super::*;

/// RFC-018: an incremental scan skips a file whose size and modification time
/// are unchanged, so the operator needs a way to have every file read again.
/// The request is stored on every library and spent by its next scan.
#[tokio::test]
async fn database_rescan_full_asks_every_library_to_read_everything() {
    let (_temp, config, state) = test_app().await;
    let owner = inserted_account(&state, "rescan-owner", "admin").await;
    let music = config.data_dir.join("rescan-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Rescan",
            &std::fs::canonicalize(&music).unwrap(),
            waveflow_server::storage::database::LibraryVisibility::Private,
            waveflow_server::auth::authentication::now_ms(),
        )
        .await
        .unwrap();
    assert!(!state.db.full_scan_requested(library).await.unwrap());

    run_cli(&state, &["database", "rescan-full"]).await.unwrap();

    assert!(state.db.full_scan_requested(library).await.unwrap());
}

/// RFC-021 decision 5 from the command line: rename, move with its scan,
/// purge the missing, and a deletion that refuses until the name is typed.
#[tokio::test]
async fn the_cli_renames_moves_purges_and_deletes_a_library() {
    let (temp, config, state) = test_app().await;
    let admin = inserted_account(&state, "lifecycle-admin", "admin").await;
    let music = config.data_dir.join("lifecycle-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Kept.wav"));
    write_test_wav_of_len(&music.join("Gone.wav"), 900);
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            admin,
            "Lifecycle",
            &root,
            waveflow_server::storage::database::LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let id = library.to_string();
    scan_once(
        &state,
        admin,
        waveflow_server::storage::catalog::LibraryRecord {
            id: library,
            name: "Lifecycle".into(),
            root_path: root.clone(),
        },
    )
    .await;

    run_cli(
        &state,
        &[
            "library",
            "rename",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--name",
            "Renamed",
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM library WHERE name = 'Renamed'"
        )
        .await,
        1
    );

    // Moved with one file fewer: the scan the move queues finds it missing.
    let moved = temp.path().join("lifecycle-moved");
    std::fs::rename(&music, &moved).unwrap();
    std::fs::remove_file(moved.join("Gone.wav")).unwrap();
    let moved_text = moved.to_string_lossy().into_owned();
    run_cli(
        &state,
        &[
            "library",
            "set-path",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--path",
            &moved_text,
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 0").await,
        1
    );

    let unnamed = run_cli(
        &state,
        &[
            "library",
            "purge",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
        ],
    )
    .await;
    assert!(unnamed.is_err(), "a purge naming nothing must refuse");
    run_cli(
        &state,
        &[
            "library",
            "purge",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--all",
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE purged_at IS NULL").await,
        1
    );
    // Within the grace window, a purge is taken back.
    run_cli(
        &state,
        &[
            "library",
            "restore",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--all",
        ],
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE purged_at IS NOT NULL"
        )
        .await,
        0
    );
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 0").await,
        1
    );

    let mistyped = run_cli(
        &state,
        &[
            "library",
            "delete",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--confirm",
            "Lifecycle",
        ],
    )
    .await;
    assert!(
        mistyped.is_err(),
        "the old name must not confirm a deletion"
    );
    assert_eq!(count(&state, "SELECT COUNT(*) FROM library").await, 1);
    run_cli(
        &state,
        &[
            "library",
            "delete",
            "--actor",
            "lifecycle-admin",
            "--library-id",
            &id,
            "--confirm",
            "Renamed",
        ],
    )
    .await
    .unwrap();
    assert_eq!(count(&state, "SELECT COUNT(*) FROM library").await, 0);
    assert_eq!(count(&state, "SELECT COUNT(*) FROM track").await, 0);
    assert!(moved.join("Kept.wav").exists(), "a file was touched");
}
