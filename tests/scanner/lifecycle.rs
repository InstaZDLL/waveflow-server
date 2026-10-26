//! A library's lifecycle: moving its root, who may change it, and deleting it
//! (RFC-021 decision 5).

use super::*;

/// RFC-021 decision 5: a root moved with its tree intact keeps every
/// identifier, and the scan it queues says why it ran.
#[tokio::test]
async fn a_moved_root_keeps_every_identity() {
    let (temp, state, owner, library, root) = library_with_albums("moved").await;
    let before: Vec<(String, String)> =
        sqlx::query_as("SELECT relative_path, id FROM track ORDER BY relative_path")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    let moved = temp.path().join("moved-elsewhere");
    std::fs::rename(&root, &moved).unwrap();

    let updated = state
        .services
        .update_library(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::LibraryUpdate {
                path: Some(moved.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let scan = updated.scan_id.expect("a new root queues a scan");
    let record = settled(&state, owner, scan).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!((record.added, record.unavailable), (0, 0));
    let trigger: String = sqlx::query_scalar("SELECT trigger FROM scan_job WHERE id = ?")
        .bind(scan.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(trigger, "library_moved");
    let after: Vec<(String, String)> = sqlx::query_as(
        "SELECT relative_path, id FROM track WHERE is_available = 1 ORDER BY relative_path",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(after, before);
    let stored: String = sqlx::query_scalar("SELECT root_path FROM library WHERE id = ?")
        .bind(library.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        std::path::PathBuf::from(stored),
        std::fs::canonicalize(&moved).unwrap()
    );
}

/// RFC-021 decision 5: who may do what. The owner renames; the path, the
/// links setting and deletion are an administrator's; a listener does
/// nothing; and nothing that changes the walk happens during a scan.
#[tokio::test]
async fn the_lifecycle_follows_the_role_model() {
    use waveflow_server::services::{LibraryUpdate, ServiceError};

    let (temp, state, admin, library, _root) = library_with_albums("roles").await;
    let owner = insert_account(&state, "roles-owner", "user").await;
    let listener = insert_account(&state, "roles-listener", "user").await;
    sqlx::query("UPDATE library_member SET user_id = ? WHERE library_id = ?")
        .bind(owner.to_string())
        .bind(library.id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner,
            library.id,
            listener,
            waveflow_server::storage::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let rename = |name: &str| LibraryUpdate {
        name: Some(name.into()),
        ..Default::default()
    };

    state
        .services
        .update_library(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            rename("  Renamed  "),
        )
        .await
        .unwrap();
    let name: String = sqlx::query_scalar("SELECT name FROM library WHERE id = ?")
        .bind(library.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(name, "Renamed");
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(listener),
                library.id,
                rename("Mine now")
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(owner),
                library.id,
                rename(" ")
            )
            .await,
        Err(ServiceError::Invalid)
    ));
    let elsewhere = temp.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let repath = LibraryUpdate {
        path: Some(elsewhere.to_string_lossy().into_owned()),
        ..Default::default()
    };
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(owner),
                library.id,
                repath.clone()
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        state
            .services
            .delete_library(
                waveflow_server::services::LibraryActor::operator(owner),
                library.id,
                "Renamed"
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    let scanning = state.scanner.try_lock_library(library.id).unwrap();
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(admin),
                library.id,
                repath
            )
            .await,
        Err(ServiceError::Conflict)
    ));
    assert!(matches!(
        state
            .services
            .delete_library(
                waveflow_server::services::LibraryActor::operator(admin),
                library.id,
                "Renamed"
            )
            .await,
        Err(ServiceError::Conflict)
    ));
    drop(scanning);
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(admin),
                library.id,
                LibraryUpdate {
                    path: Some(temp.path().join("absent").to_string_lossy().into_owned()),
                    ..Default::default()
                }
            )
            .await,
        Err(ServiceError::Invalid)
    ));
    // Somebody else's library, by an outsider: as missing as no library.
    let outsider = insert_account(&state, "roles-outsider", "user").await;
    assert!(matches!(
        state
            .services
            .update_library(
                waveflow_server::services::LibraryActor::operator(outsider),
                library.id,
                rename("Ours")
            )
            .await,
        Err(ServiceError::NotFound)
    ));
}

/// RFC-021 decision 5: deleting a library is confirmed by its name, removes
/// every row it held and nothing on disk, and leaves the other libraries
/// alone.
#[tokio::test]
async fn a_deleted_library_leaves_no_row_behind() {
    use waveflow_server::services::ServiceError;

    let (temp, state, admin, library, root) = library_with_albums("deleted").await;
    let kept_root = temp.path().join("kept");
    std::fs::create_dir_all(&kept_root).unwrap();
    write_test_wav_of_len(&kept_root.join("Kept.wav"), 1234);
    let kept_root = std::fs::canonicalize(&kept_root).unwrap();
    let kept_id = state
        .db
        .create_library(
            admin,
            "kept",
            &kept_root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    scan_once(
        &state,
        admin,
        LibraryRecord {
            id: kept_id,
            name: "kept".into(),
            root_path: kept_root,
        },
    )
    .await;
    let doomed = track_id(&state, "First/0.flac").await;
    sqlx::query(
        "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
         VALUES (?, 'track', ?, 1)",
    )
    .bind(admin.to_string())
    .bind(&doomed)
    .execute(state.db.pool())
    .await
    .unwrap();

    assert!(matches!(
        state
            .services
            .delete_library(
                waveflow_server::services::LibraryActor::operator(admin),
                library.id,
                "Deleted"
            )
            .await,
        Err(ServiceError::Invalid)
    ));
    assert_eq!(count(&state, "SELECT COUNT(*) FROM library").await, 2);

    let deleted = state
        .services
        .delete_library(
            waveflow_server::services::LibraryActor::operator(admin),
            library.id,
            "deleted",
        )
        .await
        .unwrap();
    assert_eq!(deleted.tracks, 3);
    assert_eq!(deleted.albums, 2);

    for table in [
        "SELECT COUNT(*) FROM track WHERE library_id = ?",
        "SELECT COUNT(*) FROM album WHERE library_id = ?",
        "SELECT COUNT(*) FROM artist WHERE library_id = ?",
        "SELECT COUNT(*) FROM genre WHERE library_id = ?",
        "SELECT COUNT(*) FROM track_fts WHERE library_id = ?",
        "SELECT COUNT(*) FROM artist_fts WHERE library_id = ?",
        "SELECT COUNT(*) FROM library_member WHERE library_id = ?",
        "SELECT COUNT(*) FROM scan_job WHERE library_id = ?",
        "SELECT COUNT(*) FROM library_event WHERE library_id = ?",
    ] {
        let left: i64 = sqlx::query_scalar(table)
            .bind(library.id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
        assert_eq!(left, 0, "{table}");
    }
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM user_star").await,
        0,
        "a favourite of a deleted track outlived it"
    );
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(state.db.pool())
        .await
        .unwrap();
    assert!(violations.is_empty());
    assert!(root.join("First/0.flac").exists(), "a file was touched");
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 1").await,
        1,
        "the other library lost its track"
    );
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_event WHERE kind = 'library.deleted' AND subject_id = ?",
    )
    .bind(library.id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(audited, 1);
}

/// RFC-021 decision 5: a new root is never stored without the scan it owes.
/// Here the job cannot be queued — a trigger refuses it — and the root must
/// stay where it was, rather than commit and wait for a scan nobody queued.
#[tokio::test]
async fn a_move_whose_scan_cannot_be_queued_is_not_stored() {
    let (temp, state, owner, library, root) = library_with_albums("move-unqueued").await;
    sqlx::query(
        "CREATE TRIGGER refuse_moved_scans BEFORE INSERT ON scan_job \
         WHEN NEW.trigger = 'library_moved' \
         BEGIN SELECT RAISE(ABORT, 'no scan for you'); END",
    )
    .execute(state.db.pool())
    .await
    .unwrap();
    let elsewhere = temp.path().join("move-unqueued-elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();

    let refused = state
        .services
        .update_library(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::LibraryUpdate {
                path: Some(elsewhere.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await;
    assert!(refused.is_err(), "{refused:?}");
    let stored: String = sqlx::query_scalar("SELECT root_path FROM library WHERE id = ?")
        .bind(library.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        std::path::PathBuf::from(stored),
        root,
        "the new root was stored without the scan it owes"
    );
    // Nor the record of a move that did not happen: the audit row is written
    // in the same transaction, and goes with it.
    let audited: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_event WHERE kind = 'library.moved' AND subject_id = ?",
    )
    .bind(library.id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(audited, 0, "a move that was refused was audited as made");
}
