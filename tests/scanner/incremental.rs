//! Full and incremental scans: what is skipped, what is read again, and
//! unreadable folders (RFC-018).

use super::*;

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

    let first = scan_once(&state, owner, library.clone()).await;
    assert_eq!(job_of(first).await.added, 1);

    // The second run recognises the file and does nothing, which is the
    // behaviour worth keeping.
    let second = scan_once(&state, owner, library.clone()).await;
    let second = job_of(second).await;
    assert_eq!(second.skipped, 1);
    assert_eq!(second.updated, 0);

    // Asking changes that, on a file that has not moved by a single byte.
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
    state.db.request_full_scan_everywhere().await.unwrap();
    assert!(state.db.full_scan_requested(library_id).await.unwrap());
    let third = scan_once(&state, owner, library.clone()).await;
    let third = job_of(third).await;
    assert_eq!(third.skipped, 0, "a full scan skips nothing");
    assert_eq!(third.updated, 1);

    // And the request is spent, so the next run is ordinary again.
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
    let fourth = scan_once(&state, owner, library).await;
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

    // Nor can a run that started before the request arrived: it read the
    // catalogue under the old rules, so completing it says nothing about the
    // new ones.
    let ordinary = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(ordinary, 1, false).await.unwrap();
    state.db.finish_scan_job(ordinary, 0).await.unwrap();
    assert!(
        state.db.full_scan_requested(library_id).await.unwrap(),
        "an ordinary run cannot spend a request it never honoured"
    );

    // Only a completed full run spends it.
    let completed = state
        .db
        .create_scan_job(library_id, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(completed, 1, true).await.unwrap();
    state.db.finish_scan_job(completed, 0).await.unwrap();
    assert!(!state.db.full_scan_requested(library_id).await.unwrap());
}

async fn sidecar_signature(state: &waveflow_server::AppState) -> Option<String> {
    sqlx::query_scalar("SELECT sidecar_signature FROM track")
        .fetch_one(state.db.pool())
        .await
        .unwrap()
}

/// Rewrites `path` in place with different bytes of the same length, then
/// puts its modification time back: a change only reading the file can see.
fn rewrite_keeping_size_and_time(path: &std::path::Path) {
    let modified = std::fs::metadata(path).unwrap().modified().unwrap();
    let mut bytes = std::fs::read(path).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 0x5a;
    std::fs::write(path, &bytes).unwrap();
    let file = std::fs::File::options().write(true).open(path).unwrap();
    file.set_times(std::fs::FileTimes::new().set_modified(modified))
        .unwrap();
}

/// RFC-018 decision 1: a file whose size, modification time and sidecars are
/// unchanged is skipped without being read. A change only its bytes show —
/// same size, same time — therefore goes unseen until a full scan reads it,
/// which is the trade the RFC states.
#[tokio::test]
async fn an_unchanged_file_is_skipped_without_being_read() {
    let (_temp, state, owner, library, root) = incremental_library("incremental-skip", |music| {
        write_test_wav(&music.join("Only.wav"))
    })
    .await;
    let first = scan_once(&state, owner, library.clone()).await;
    assert_eq!(job(&state, owner, first).await.added, 1);
    let hash = full_hash(&state).await;

    rewrite_keeping_size_and_time(&root.join("Only.wav"));
    let second = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(second.skipped, 1);
    assert_eq!(second.updated, 0);
    assert_eq!(
        full_hash(&state).await,
        hash,
        "the file was read: its new hash was recorded"
    );

    state.db.request_full_scan_everywhere().await.unwrap();
    let full = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(full.updated, 1);
    assert_ne!(full_hash(&state).await, hash);
}

/// A sidecar added, edited or removed beside an unchanged audio file is still
/// read: the signature it is compared on covers both sidecars, absent ones
/// included.
#[tokio::test]
async fn a_sidecar_changed_beside_an_unchanged_file_is_read() {
    let (_temp, state, owner, library, root) =
        incremental_library("incremental-sidecar", |music| {
            write_test_wav(&music.join("Song.wav"))
        })
        .await;
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(
        sidecar_signature(&state).await.as_deref(),
        Some("lrc:-|txt:-")
    );

    std::fs::write(root.join("Song.lrc"), "[00:01.00]A line\n").unwrap();
    let added = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(added.skipped, 0, "the new sidecar was not read");
    assert_eq!(added.updated, 1);

    let unchanged = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(unchanged.skipped, 1);

    std::fs::remove_file(root.join("Song.lrc")).unwrap();
    let removed = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(removed.skipped, 0, "the removed sidecar went unnoticed");
    assert_eq!(removed.updated, 1);
}

/// `NULL` means never computed and never matches: right after the migration
/// every file is read once, which fills the signature, and the scan after it
/// skips.
#[tokio::test]
async fn a_signature_never_computed_is_filled_by_the_next_scan() {
    let (_temp, state, owner, library, _root) = incremental_library("incremental-null", |music| {
        write_test_wav(&music.join("Only.wav"))
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    sqlx::query("UPDATE track SET sidecar_signature = NULL")
        .execute(state.db.pool())
        .await
        .unwrap();

    scan_once(&state, owner, library.clone()).await;
    assert_eq!(
        sidecar_signature(&state).await.as_deref(),
        Some("lrc:-|txt:-"),
        "an unchanged file read for want of a signature must record one"
    );
    let next = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(next.skipped, 1);
}

/// RFC-018 decision 3: a folder the walk cannot read costs that folder, not
/// the scan, and its tracks are not marked missing. Unix only: it is the one
/// place a test can take a folder's permissions away.
#[cfg(unix)]
#[tokio::test]
async fn an_unreadable_folder_costs_that_folder_only() {
    use std::os::unix::fs::PermissionsExt;

    let (_temp, state, owner, library, root) =
        incremental_library("incremental-unreadable", |music| {
            std::fs::create_dir_all(music.join("Readable")).unwrap();
            std::fs::create_dir_all(music.join("Locked")).unwrap();
            write_test_wav(&music.join("Readable/One.wav"));
            write_test_wav_of_len(&music.join("Locked/Two.wav"), 900);
        })
        .await;
    let first = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(first.added, 2);

    /// Puts the permissions back however the test ends, or the temporary
    /// directory could not be removed.
    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    let locked = root.join("Locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unlock = Unlock(locked.clone());
    if std::fs::read_dir(&locked).is_ok() {
        // Root reads through `chmod 000`: nothing here can be made unreadable.
        eprintln!("skipped: the folder stays readable to this user");
        return;
    }
    let second = scan_once(&state, owner, library.clone()).await;
    drop(unlock);

    let second = job(&state, owner, second).await;
    assert_eq!(second.errors, 1);
    assert_eq!(
        second.unavailable, 0,
        "a track under the unreadable folder was marked missing"
    );
    let available: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM track WHERE is_available = 1")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(available, 2);
}

/// When everything a library holds sits under the unreadable folder, the walk
/// finds no file and the scan runs no batch: the folder's error must still be
/// counted on the job, and its track still not marked missing.
#[cfg(unix)]
#[tokio::test]
async fn an_unreadable_folder_holding_everything_is_still_counted() {
    use std::os::unix::fs::PermissionsExt;

    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
    }

    let (_temp, state, owner, library, root) =
        incremental_library("incremental-all-locked", |music| {
            std::fs::create_dir_all(music.join("Locked")).unwrap();
            write_test_wav(&music.join("Locked/Only.wav"));
        })
        .await;
    assert_eq!(
        job(
            &state,
            owner,
            scan_once(&state, owner, library.clone()).await
        )
        .await
        .added,
        1
    );

    let locked = root.join("Locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unlock = Unlock(locked.clone());
    if std::fs::read_dir(&locked).is_ok() {
        eprintln!("skipped: the folder stays readable to this user");
        return;
    }
    let second = scan_once(&state, owner, library).await;
    drop(unlock);

    let second = job(&state, owner, second).await;
    assert_eq!(
        second.errors, 1,
        "the folder's error was not recorded on the job"
    );
    assert_eq!(second.unavailable, 0);
}

/// A sidecar the scan can list but not open is read past, without its lyrics,
/// and no signature is recorded: once the permission is fixed — which moves
/// neither its size nor its time — the next scan reads it. Unix only, for the
/// permissions.
#[cfg(unix)]
#[tokio::test]
async fn a_sidecar_that_cannot_be_opened_is_read_again_once_it_can() {
    use std::os::unix::fs::PermissionsExt;

    struct Unlock(std::path::PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o644));
        }
    }

    let (_temp, state, owner, library, root) =
        incremental_library("incremental-locked-lyrics", |music| {
            write_test_wav(&music.join("Song.wav"));
            std::fs::write(music.join("Song.lrc"), "[00:01.00]A line\n").unwrap();
        })
        .await;
    let sidecar = root.join("Song.lrc");
    std::fs::set_permissions(&sidecar, std::fs::Permissions::from_mode(0o000)).unwrap();
    let unlock = Unlock(sidecar.clone());
    if std::fs::File::open(&sidecar).is_ok() {
        eprintln!("skipped: the sidecar stays readable to this user");
        return;
    }
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(sidecar_signature(&state).await, None);

    drop(unlock);
    let again = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(again.skipped, 0, "the now-readable sidecar was never read");
    assert_eq!(again.updated, 1);
    assert!(sidecar_signature(&state).await.is_some());
}

/// A sidecar that is a FIFO with no writer does not hang the scan: the
/// signature never opens anything but a regular file, and the read never
/// blocks. `scan_once` gives up after two seconds, so a hang fails here.
#[cfg(unix)]
#[tokio::test]
async fn a_fifo_beside_an_audio_file_does_not_hang_the_scan() {
    let (_temp, state, owner, library, _root) = incremental_library("incremental-fifo", |music| {
        write_test_wav(&music.join("Song.wav"));
        let fifo = std::ffi::CString::new(
            music
                .join("Song.lrc")
                .to_str()
                .expect("a UTF-8 temporary path"),
        )
        .unwrap();
        // SAFETY: a valid, NUL-terminated path and an ordinary mode.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    })
    .await;
    let first = job(
        &state,
        owner,
        scan_once(&state, owner, library.clone()).await,
    )
    .await;
    assert_eq!(first.added, 1);
    assert_eq!(
        sidecar_signature(&state).await.as_deref(),
        Some("lrc:-|txt:-")
    );
    let second = job(&state, owner, scan_once(&state, owner, library).await).await;
    assert_eq!(second.skipped, 1);
}
