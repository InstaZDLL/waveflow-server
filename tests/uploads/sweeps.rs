//! Expired and abandoned sessions, and the staging files the sweep reclaims or
//! spares.

use super::*;

/// An expired session takes its staging area with it.
#[tokio::test]
async fn an_expired_session_takes_its_staging_file_with_it() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "expiring").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let staging = root
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    assert!(staging.is_file(), "there is something to clean up");

    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();
    // Any negotiation sweeps first, which is where the cleanup happens.
    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(999, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        !staging.exists(),
        "an abandoned transfer must not keep occupying a disk the quota measures"
    );
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// A session nobody owns leaves nothing behind in the process.
///
/// The per-session locks are created on lookup, so locking before resolving
/// would let any authorised caller mint one entry per uuid they can type — a
/// map that only ever grows, for sessions that never existed. Resolving first
/// bounds the entries to sessions that are real and theirs.
#[tokio::test]
async fn a_session_that_does_not_exist_leaves_nothing_behind() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "phantom").await;

    for _ in 0..16 {
        let stranger = uuid::Uuid::new_v4().to_string();
        let (status, _) =
            put_chunk(&config, &state, &fixture.token, &stranger, 0, &bytes[..512]).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (committed, _) = commit(&config, &state, &fixture.token, &stranger).await;
        assert_eq!(committed, StatusCode::NOT_FOUND);
    }
    assert_eq!(
        state.services.tracked_upload_locks(),
        0,
        "sessions that never existed must not be coordinated"
    );

    // A real one is tracked while it runs, and released when it ends.
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (status, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(state.services.tracked_upload_locks(), 1);

    let mut sent = 512usize;
    let mut index = 1usize;
    while sent < bytes.len() {
        let end = (sent + 512).min(bytes.len());
        let (status, _) = put_chunk(
            &config,
            &state,
            &fixture.token,
            &session,
            index,
            &bytes[sent..end],
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        sent = end;
        index += 1;
    }
    let (status, _) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        state.services.tracked_upload_locks(),
        0,
        "and a finished session is not coordinated any more"
    );
}

/// Cleanup does not wait for somebody to offer another file.
///
/// The sweep otherwise only runs inside a negotiation. Nothing breaks — the
/// quota is decided by that same negotiation — but a client that abandons a
/// batch of transfers and never returns would leave its staging files holding
/// the operator's disk with nothing to signal it.
#[tokio::test]
async fn abandoned_transfers_are_swept_without_a_negotiation() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "abandoned").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let staging = root
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    assert!(staging.is_file(), "there is something to reclaim");

    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();

    // No negotiation, no request of any kind: only the scheduled sweep.
    state.services.spawn_upload_sweeper();
    // The sweep removes the file first and the row second, so waiting on the
    // file alone and then counting the row once reads the gap between the
    // two: a slow runner failed here with the row still present (#280).
    let rows = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM upload_session WHERE id=?")
            .bind(&session)
            .fetch_one(state.db.pool())
            .await
            .unwrap()
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while (staging.exists() || rows().await > 0) && std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    assert!(
        !staging.exists(),
        "an abandoned transfer must not hold the disk until someone happens to upload again"
    );
    assert_eq!(rows().await, 0);
}

/// A staging file that cannot be removed keeps its session.
///
/// File first, row second was chosen so a failure could be retried. A failure
/// that deletes the row anyway is not a retry — it is exactly the orphan that
/// order was avoiding, since the row is the only thing that remembers the file
/// exists. Here the staging path is made undeletable by turning it into a
/// non-empty directory, which `remove_file` refuses for a reason that is not
/// "already gone".
#[tokio::test]
async fn a_staging_file_that_cannot_be_removed_keeps_its_session() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "stuck").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let staging = root
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    std::fs::remove_file(&staging).unwrap();
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("occupied"), b"x").unwrap();

    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();
    // A negotiation sweeps first; the sweep must decline to forget this one.
    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(777, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "the row is the only record that this file exists; dropping it orphans the file"
    );

    // And once the obstruction is gone, the next sweep finishes the job.
    std::fs::remove_dir_all(&staging).unwrap();
    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(778, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(rows, 0, "a retry is what the order was for");
}

/// The same rule when a commit gives up on the bytes it received.
#[tokio::test]
async fn a_failed_commit_keeps_a_session_whose_file_it_could_not_remove() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "stuck-commit").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (sent, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes).await;
    assert_eq!(sent, StatusCode::OK);

    // The transfer looks complete to the catalogue, and the file it points at
    // can be neither hashed nor removed.
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let staging = root
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    std::fs::remove_file(&staging).unwrap();
    std::fs::create_dir(&staging).unwrap();
    std::fs::write(staging.join("occupied"), b"x").unwrap();

    let (status, _) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "a commit that could not clean up must leave the record that says so"
    );
    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        0
    );
}

/// A file stuck under its final name is reclaimed, and only when nobody else
/// could be using it.
///
/// A commit that renamed the file and then could neither move it back nor
/// remove it leaves one there. Nothing else looks under that name, so without
/// the sweep it would outlive the row that remembers it and be lost for good.
///
/// The guards are the point. That name is the hash, not the session's own
/// property: the unique index is per member, so two members can hold sessions
/// for the same hash in one library, and a successful commit by either produces
/// exactly that file.
#[tokio::test]
async fn a_file_stuck_under_its_final_name_is_reclaimed_unless_claimed() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "stuck-final").await;
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let hash = blake3_of(&bytes);
    let relative = format!(".waveflow-managed/{hash}.wav");
    let stuck = root.join(&relative);

    // The state a commit leaves when it can neither move the file back nor
    // remove it: a session, and a file under the final name.
    let _session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    std::fs::create_dir_all(stuck.parent().unwrap()).unwrap();
    std::fs::write(&stuck, &bytes).unwrap();
    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();

    // A track claiming that path first: the file is in use, and the sweep must
    // not touch it.
    let scan_id = state
        .db
        .create_scan_job(fixture.library, Some(fixture.owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan_id, 1, false).await.unwrap();
    let mut input = catalog_input(1, "Claiming Artist");
    input.relative_path = relative.clone();
    input.full_hash = hash.clone();
    state
        .db
        .apply_catalog_track(fixture.library, scan_id, &input, None, false)
        .await
        .unwrap();

    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(555, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        stuck.is_file(),
        "a file a track is using must survive somebody else's abandoned session"
    );

    // Now the same state with nothing claiming it. The track goes first: while
    // it exists the negotiation answers `present`, which is the whole point of
    // the first half.
    sqlx::query("DELETE FROM track WHERE library_id=?")
        .bind(fixture.library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let _session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();

    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(556, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !stuck.exists(),
        "and one nobody is using must not outlive the row that remembers it"
    );
}

/// And spared while another member could still be committing it.
///
/// Between a commit's rename and its transaction there is no track yet, so the
/// track guard alone would let a stranger's expired session delete the file
/// that commit is about to catalogue. A live session on the same hash is what
/// says the name is not free.
#[tokio::test]
async fn a_file_another_session_may_be_committing_is_spared() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "contested-final").await;

    let password = security::generate_token("test-password-");
    let hashed = security::hash_password(&password).unwrap();
    let other = state
        .db
        .create_account("contesting-manager", &hashed, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            fixture.owner,
            fixture.library,
            other,
            LibraryRole::Manager,
            now_ms(),
        )
        .await
        .unwrap();
    let as_other = Fixture {
        owner: other,
        library: fixture.library,
        token: login_token(
            &waveflow_server::app(&config, state.clone()),
            "contesting-manager",
            &password,
        )
        .await,
    };

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let hash = blake3_of(&bytes);
    let stuck = root.join(format!(".waveflow-managed/{hash}.wav"));

    // Both sessions are opened before either expires. A negotiation sweeps
    // before it answers, so opening the second one afterwards would sweep the
    // first while nothing yet contested the name — and the test would pass for
    // a reason that has nothing to do with the guard.
    let _abandoned = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let live = open_session(&config, &state, &as_other, &bytes, "wav").await;
    std::fs::create_dir_all(stuck.parent().unwrap()).unwrap();
    std::fs::write(&stuck, &bytes).unwrap();
    sqlx::query("UPDATE upload_session SET expires_at=1 WHERE user_id=?")
        .bind(fixture.owner.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();

    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(444, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);

    assert!(
        stuck.is_file(),
        "an expired session must not delete the file another one is committing"
    );
    let survivors: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&live)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(survivors, 1, "and the live session is untouched");
}

/// Not being able to tell is not the same as being told there is nothing.
///
/// The sweep asks whether a file is stuck under the final name before deciding
/// the session can go. Reading an I/O error from that question as "absent"
/// drops the row while the file it names may well be there — the orphan this
/// whole path exists to prevent, one question earlier.
///
/// A symlink pointing at itself is the deterministic way to make the question
/// fail: following it errors, while the staging file beside it is an ordinary
/// file the sweep would have removed without a second thought.
///
/// The branch under test is not POSIX-specific, but the way to reach it is:
/// creating a symlink on Windows needs a privilege the runner does not have.
/// Gated rather than made portable — a test that has to be told which error to
/// expect proves less than one that provokes a real one.
#[cfg(unix)]
#[tokio::test]
async fn a_file_the_sweep_cannot_ask_about_keeps_its_session() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "unaskable").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (sent, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(sent, StatusCode::OK);

    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let final_name = root.join(format!(".waveflow-managed/{}.wav", blake3_of(&bytes)));
    std::os::unix::fs::symlink(&final_name, &final_name).unwrap();
    assert!(
        std::fs::metadata(&final_name).is_err(),
        "the point of this test is a path that cannot be resolved"
    );

    sqlx::query("UPDATE upload_session SET expires_at=1")
        .execute(state.db.pool())
        .await
        .unwrap();
    let (status, _) = negotiate(&config, &state, &fixture, vec![offer(333, 4096, "flac")]).await;
    assert_eq!(status, StatusCode::OK);

    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM upload_session WHERE id=?")
        .bind(&session)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "a question that could not be answered must not be answered as 'nothing there'"
    );
    assert!(
        root.join(format!(".waveflow-managed/{session}.part"))
            .is_file(),
        "and the transfer it had already received is still there"
    );
}
