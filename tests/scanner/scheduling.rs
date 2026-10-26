//! When a scan runs: the watcher, queued and scoped scans, and the boot.

use super::*;

/// RFC-021 decision 6: a change in a watched library is scanned once it
/// settles, by a scan that says it came from the watcher.
#[tokio::test]
async fn a_watched_change_is_scanned_once_it_settles() {
    let (_temp, state, owner, library, root) = incremental_library("watched", |music| {
        write_test_wav(&music.join("First.wav"));
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    let watcher = waveflow_server::library::watcher::spawn(
        state.scanner.clone(),
        state.db.clone(),
        std::time::Duration::from_millis(200),
    );
    // Long enough for the watch to be placed before the change it must see.
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    write_test_wav_of_len(&root.join("Second.wav"), 900);

    let mut watched = None;
    for _ in 0..300 {
        let found: Option<i64> = sqlx::query_scalar(
            "SELECT added FROM scan_job WHERE trigger = 'watch' AND status = 'completed'",
        )
        .fetch_optional(state.db.pool())
        .await
        .unwrap();
        if found.is_some() {
            watched = found;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    watcher.abort();
    assert_eq!(watched, Some(1), "no scan followed the change");
    assert_eq!(available_paths(&state).await, ["First.wav", "Second.wav"]);
}

/// RFC-021 decision 5: a scan queued before a move walks the library as it is
/// when it runs, not as it was when it was queued — the watcher and the
/// interval queue scans that a move can overtake.
#[tokio::test]
async fn a_queued_scan_reads_the_library_as_it_is_when_it_runs() {
    let (temp, state, owner, library, root) = library_with_albums("queued").await;
    let held = state.scanner.try_lock_library(library.id).unwrap();
    // Queued with the old root, and waiting for the lock.
    let queued = state
        .scanner
        .trigger(library.clone(), Some(owner), "manual")
        .await
        .unwrap();
    let moved = temp.path().join("queued-elsewhere");
    std::fs::rename(&root, &moved).unwrap();
    sqlx::query("UPDATE library SET root_path = ? WHERE id = ?")
        .bind(
            std::fs::canonicalize(&moved)
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        )
        .bind(library.id.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    drop(held);
    let record = settled(&state, owner, queued).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!((record.added, record.unavailable), (0, 0));
}

/// RFC-021 decision 6: a settled change whose scan cannot be queued stays
/// pending, rather than waiting for the periodic scan — which an interval of
/// zero turns off. A library the catalogue no longer holds refuses the job, the
/// simplest failure a test can provoke.
#[tokio::test]
async fn a_scan_the_watcher_cannot_queue_stays_pending() {
    let (_temp, _config, state) = test_app().await;
    let ghost = uuid::Uuid::new_v4();
    let watched = std::collections::HashMap::from([(
        ghost,
        LibraryRecord {
            id: ghost,
            name: "ghost".into(),
            root_path: std::path::PathBuf::from("/nowhere"),
        },
    )]);
    let delay = std::time::Duration::from_millis(10);
    let mut settle = waveflow_server::library::watcher::Settle::new(delay);
    let start = tokio::time::Instant::now();
    settle.touch(ghost, start);
    waveflow_server::library::watcher::dispatch_due(
        &mut settle,
        &watched,
        &state.scanner,
        start + delay,
    )
    .await;
    assert!(
        settle.is_pending(ghost),
        "a change whose scan could not be queued was dropped"
    );
}

/// RFC-021 decision 5: a job a previous run committed and never started — it
/// stopped between the commit and the start — runs when the server boots, and
/// its library is not scanned a second time for the boot.
#[tokio::test]
async fn a_scan_a_previous_run_left_queued_runs_at_boot() {
    let (temp, state, owner, library, _root) = library_with_albums("left-queued").await;
    let other_root = temp.path().join("left-queued-other");
    std::fs::create_dir_all(&other_root).unwrap();
    let other = state
        .db
        .create_library(
            owner,
            "other",
            &std::fs::canonicalize(&other_root).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();
    let left = state
        .db
        .create_scan_job(library.id, Some(owner), "library_moved")
        .await
        .unwrap();

    state.scanner.boot().await;

    assert_eq!(
        settled(&state, owner, left).await.status,
        "completed",
        "the queued job was never run"
    );
    let boots = |library: uuid::Uuid| {
        let state = state.clone();
        async move {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM scan_job WHERE library_id = ? AND trigger = 'boot'",
            )
            .bind(library.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap()
        }
    };
    assert_eq!(
        boots(library.id).await,
        0,
        "the resumed library was scanned twice"
    );
    assert_eq!(
        boots(other).await,
        1,
        "the other library missed its boot scan"
    );
    // Let the other library's boot scan finish before the directory goes.
    let other_job: String =
        sqlx::query_scalar("SELECT id FROM scan_job WHERE library_id = ? AND trigger = 'boot'")
            .bind(other.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    settled(&state, owner, other_job.parse().unwrap()).await;
}

async fn path_ids(state: &waveflow_server::AppState) -> Vec<(String, String, i64)> {
    sqlx::query_as("SELECT relative_path, id, is_available FROM track ORDER BY relative_path")
        .fetch_all(state.db.pool())
        .await
        .unwrap()
}

/// RFC-021, what decision 6 left open: a scan the watcher limits to the
/// folders that changed keeps RFC-018's guarantees. Only what it walked may be
/// marked missing, relocation claims no track outside it — a file moved in
/// from a folder it did not walk is a new track, and the old one is left as
/// it was for the next full scan — and a move inside it keeps its identity.
#[tokio::test]
async fn a_scoped_scan_touches_nothing_it_did_not_walk() {
    let (_temp, state, owner, library, root) = incremental_library("scoped", |music| {
        for (folder, length) in [("A", 1_000), ("B", 1_100), ("C", 1_200)] {
            std::fs::create_dir_all(music.join(folder)).unwrap();
            write_test_wav_of_len(&music.join(format!("{folder}/{folder}.wav")), length);
        }
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    let before = path_ids(&state).await;
    let id_of = |path: &str| {
        before
            .iter()
            .find(|(stored, ..)| stored == path)
            .unwrap()
            .1
            .clone()
    };
    let scoped = |folders: &[&str]| {
        let state = state.clone();
        let library = library.clone();
        let folders: Vec<String> = folders.iter().map(|f| (*f).to_owned()).collect();
        async move {
            let scan = state
                .scanner
                .trigger_scoped(library, "watch", &folders)
                .await
                .unwrap();
            settled(&state, owner, scan).await
        }
    };

    // B's file goes, and C's moves into A; only A is walked.
    std::fs::remove_file(root.join("B/B.wav")).unwrap();
    std::fs::rename(root.join("C/C.wav"), root.join("A/From C.wav")).unwrap();
    let record = scoped(&["A"]).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!(
        (record.added, record.moved, record.unavailable),
        (1, 0, 0),
        "{record:?}"
    );
    let after = path_ids(&state).await;
    let row = |path: &str| after.iter().find(|(stored, ..)| stored == path).cloned();
    assert_eq!(
        row("B/B.wav").unwrap().2,
        1,
        "B was not walked: not missing"
    );
    assert_eq!(
        row("C/C.wav").unwrap(),
        ("C/C.wav".to_owned(), id_of("C/C.wav"), 1),
        "C's track was not looked for, so nothing may claim it"
    );
    assert_ne!(row("A/From C.wav").unwrap().1, id_of("C/C.wav"));

    // Walking B marks what B lost, and nothing else.
    let record = scoped(&["B"]).await;
    assert_eq!((record.added, record.unavailable), (0, 1), "{record:?}");
    let after = path_ids(&state).await;
    let row = |path: &str| after.iter().find(|(stored, ..)| stored == path).cloned();
    assert_eq!(row("B/B.wav").unwrap().2, 0);
    assert_eq!(row("C/C.wav").unwrap().2, 1);

    // A move inside the walked folders keeps the track it moves.
    std::fs::create_dir_all(root.join("A/Deeper")).unwrap();
    std::fs::rename(root.join("A/A.wav"), root.join("A/Deeper/A.wav")).unwrap();
    let record = scoped(&["A"]).await;
    assert_eq!((record.moved, record.unavailable), (1, 0), "{record:?}");
    let after = path_ids(&state).await;
    assert_eq!(
        after
            .iter()
            .find(|(stored, ..)| stored == "A/Deeper/A.wav")
            .unwrap()
            .1,
        id_of("A/A.wav")
    );

    // A folder excluded on the way down counts as not discovered, as the
    // full walk would have it.
    std::fs::write(root.join("A/.waveflowignore"), "/Deeper/\n").unwrap();
    let record = scoped(&["A/Deeper"]).await;
    assert_eq!((record.unavailable, record.excluded), (1, 1), "{record:?}");

    // A scope it cannot trust is the whole library.
    let record = scoped(&["../elsewhere"]).await;
    assert_eq!(record.status, "completed");
    let scope: Option<String> = sqlx::query_scalar("SELECT scope_json FROM scan_job WHERE id = ?")
        .bind(record.id.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(scope, None);
    assert_eq!(
        path_ids(&state)
            .await
            .iter()
            .find(|(stored, ..)| stored == "C/C.wav")
            .unwrap()
            .2,
        0,
        "the full walk finds what the scoped ones were not allowed to"
    );
}

/// RFC-021, the per-library switch decision 6 left open: a library whose
/// watch is off is not watched, and a watched change in a library whose walk
/// reads no link is scanned in the folder it happened in.
#[tokio::test]
async fn a_library_is_watched_only_when_it_asks_and_in_the_folder_that_changed() {
    use waveflow_server::services::{LibraryActor, LibraryUpdate};

    let (_temp, state, owner, library, root) = incremental_library("watch-switch", |music| {
        std::fs::create_dir_all(music.join("Sub")).unwrap();
        write_test_wav(&music.join("Sub/First.wav"));
        std::fs::create_dir_all(music.join("Other")).unwrap();
        write_test_wav_of_len(&music.join("Other/Kept.wav"), 777);
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    let set_watch = |watch: bool| {
        let state = state.clone();
        async move {
            state
                .services
                .update_library(
                    LibraryActor::operator(owner),
                    library.id,
                    LibraryUpdate {
                        watch: Some(watch),
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
        }
    };
    assert!(
        set_watch(false).await.scan_id.is_none(),
        "a switch queues no scan"
    );
    assert!(state.db.watched_libraries().await.unwrap().is_empty());
    let watcher = waveflow_server::library::watcher::spawn(
        state.scanner.clone(),
        state.db.clone(),
        std::time::Duration::from_millis(200),
    );
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    write_test_wav_of_len(&root.join("Sub/Unwatched.wav"), 850);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let watched_scans = || async {
        sqlx::query_as::<_, (Option<String>, String)>(
            "SELECT scope_json, status FROM scan_job WHERE trigger = 'watch'",
        )
        .fetch_all(state.db.pool())
        .await
        .unwrap()
    };
    assert!(
        watched_scans().await.is_empty(),
        "a library whose watch is off was scanned by the watcher"
    );

    set_watch(true).await;
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    write_test_wav_of_len(&root.join("Sub/Second.wav"), 900);
    let mut completed = None;
    for _ in 0..300 {
        let scans = watched_scans().await;
        if let Some(done) = scans.iter().find(|(_, status)| status == "completed") {
            completed = Some(done.clone());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    watcher.abort();
    let (scope, _) = completed.expect("no scan followed the change");
    assert_eq!(scope.as_deref(), Some(r#"["Sub"]"#));
    assert_eq!(
        available_paths(&state).await,
        [
            "Other/Kept.wav",
            "Sub/First.wav",
            "Sub/Second.wav",
            "Sub/Unwatched.wav"
        ]
    );
}

/// A scoped scan is no answer to a pending full-scan request: it walks the
/// whole library, reads every file, and only then is the request spent
/// (review of #335).
#[tokio::test]
async fn a_pending_full_scan_is_not_spent_by_a_scoped_one() {
    let (_temp, state, owner, library, root) = incremental_library("scoped-full", |music| {
        for (folder, length) in [("A", 1_000), ("B", 1_100)] {
            std::fs::create_dir_all(music.join(folder)).unwrap();
            write_test_wav_of_len(&music.join(format!("{folder}/{folder}.wav")), length);
        }
    })
    .await;
    scan_once(&state, owner, library.clone()).await;
    std::fs::remove_file(root.join("B/B.wav")).unwrap();
    state.db.request_full_scan(library.id).await.unwrap();
    let scan = state
        .scanner
        .trigger_scoped(library.clone(), "watch", &["A".to_owned()])
        .await
        .unwrap();
    let record = settled(&state, owner, scan).await;
    assert_eq!(record.status, "completed", "{:?}", record.message);
    assert_eq!(
        (record.unavailable, record.skipped),
        (1, 0),
        "the whole library walked, every file read"
    );
    assert!(
        !state.db.full_scan_requested(library.id).await.unwrap(),
        "the request was answered by a full walk"
    );
}

/// A scan's event stream ends once the scan has. The channel behind it used to
/// outlive the job — one per scan, never dropped — so a watcher that came after
/// the last progress got the snapshot and then nothing, kept open by
/// keep-alives until it hung up, and the server held every finished scan's
/// channel for good.
#[tokio::test]
async fn a_finished_scan_s_event_stream_ends() {
    let (_temp, config, state) = test_app().await;
    let (owner, token) = admin_with_api_token(&state, "stream-owner").await;
    let music = config.data_dir.join("stream-music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("One.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library_id = state
        .db
        .create_library(owner, "Stream", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let library = LibraryRecord {
        id: library_id,
        name: "Stream".into(),
        root_path: root,
    };
    let scan = scan_once(&state, owner, library).await;
    let router = waveflow_server::app(&config, state.clone());

    let response = tower::ServiceExt::oneshot(
        router,
        axum::http::Request::get(format!("/api/v2/scans/{scan}/events"))
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    // The whole body, which only exists if the stream ends: a stream that
    // stays open never yields it, and the deadline says so.
    let body = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await
    .expect("the event stream of a finished scan must end")
    .unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("event: snapshot"), "{text}");
    assert!(text.contains("\"completed\""), "{text}");
}
