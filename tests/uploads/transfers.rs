//! Sending a file in fragments and committing it, and what stops a transfer
//! midway.

use super::*;

/// The whole thing: a file arrives in pieces and is a track by the time the
/// server answers.
///
/// Waiting for the next scan would leave the client with a successful transfer
/// and a catalogue that ignores it, with nothing to say for how long.
#[tokio::test]
async fn a_file_arrives_in_fragments_and_is_a_track_at_once() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "arrives").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;

    let mut sent = 0usize;
    let mut index = 0usize;
    while sent < bytes.len() {
        let end = (sent + 512).min(bytes.len());
        let (status, state_body) = put_chunk(
            &config,
            &state,
            &fixture.token,
            &session,
            index,
            &bytes[sent..end],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "fragment {index}");
        assert_eq!(state_body["next_chunk"], index as i64 + 1);
        assert_eq!(state_body["received_bytes"], end as i64);
        sent = end;
        index += 1;
    }
    assert!(
        index > 1,
        "the point of this test is more than one fragment"
    );

    let (status, committed) = commit(&config, &state, &fixture.token, &session).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        committed["full_hash"],
        blake3_of(&bytes),
        "the hash is what the server computed, and it matches what arrived"
    );
    let track_id = committed["track_id"].as_str().unwrap();

    // The track exists now, not at the next scan.
    let tracks = state
        .db
        .list_tracks_for_user(fixture.owner, fixture.library)
        .await
        .unwrap();
    assert_eq!(tracks.len(), 1);
    assert_eq!(tracks[0].id.to_string(), track_id);

    // Filed by hash, under the directory the server owns, keeping the
    // extension the walk recognises it by.
    let expected = format!(".waveflow-managed/{}.wav", blake3_of(&bytes));
    assert_eq!(tracks[0].relative_path, expected);
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    assert!(root.join(&expected).is_file());

    // The library feed announced it, because the apply did — nothing here had
    // to remember to.
    let events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM library_event \
         WHERE library_id=? AND entity_type='track' AND action='upsert'",
    )
    .bind(fixture.library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(events, 1);

    // And the session is gone, so the space it reserved is now the space the
    // file occupies rather than both at once.
    assert_eq!(open_sessions(&state).await, 0);
}

/// A fragment sent twice is answered, not rejected.
///
/// An acknowledgement lost after the write is what a dropped link ordinarily
/// produces. Treating the client's honest retry as a fault would make the
/// protocol fragile exactly where it exists to absorb interruptions.
#[tokio::test]
async fn a_fragment_sent_twice_is_answered_rather_than_refused() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "repeated").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;

    let (first, first_body) =
        put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);
    assert_eq!(first_body["next_chunk"], 1);

    let (again, again_body) =
        put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(again, StatusCode::OK, "a re-sent fragment is not a fault");
    assert_eq!(
        again_body["next_chunk"], 1,
        "and it does not advance the transfer a second time"
    );
    assert_eq!(again_body["received_bytes"], 512);
}

/// A fragment out of order is refused, because the gap would only surface at
/// the final hash.
#[tokio::test]
async fn a_fragment_from_the_future_is_refused() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "future").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;

    let (status, _) = put_chunk(
        &config,
        &state,
        &fixture.token,
        &session,
        1,
        &bytes[512..1024],
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);

    // And a fragment short of what its position calls for, which would put
    // every later one at the wrong offset.
    let (short, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..10]).await;
    assert_eq!(short, StatusCode::UNPROCESSABLE_ENTITY);
}

/// Bytes that are not what was promised are discarded, and leave nothing.
///
/// The declared hash exists to avoid a transfer. Letting it establish an
/// identity would let any authorised member pass one file off as another.
#[tokio::test]
async fn bytes_that_are_not_what_was_promised_are_discarded() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "mismatch").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;

    // The same length, different content — so only the hash can tell.
    let mut other = bytes.clone();
    let last = other.len() - 1;
    other[last] ^= 0xff;
    let (status, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &other).await;
    assert_eq!(status, StatusCode::OK);

    let (committed, _) = commit(&config, &state, &fixture.token, &session).await;

    assert_eq!(committed, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        0,
        "nothing was catalogued"
    );
    assert_eq!(open_sessions(&state).await, 0, "and the session is closed");
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let leftovers: Vec<_> = std::fs::read_dir(root.join(".waveflow-managed"))
        .map(|dir| dir.filter_map(Result::ok).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "and nothing was left on the disk: {leftovers:?}"
    );
}

/// A file the catalogue cannot read does not get to stay.
///
/// The extension was never proof. Reading the file through the scan's own
/// extractor is, and what it cannot open is removed rather than left occupying
/// a disk the catalogue could never show it on.
#[tokio::test]
async fn a_file_the_catalogue_cannot_read_does_not_stay() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let fixture = fixture(&config, &state, "unreadable").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    let bytes = b"MZ this is not audio, whatever it is called".to_vec();
    let session = open_session(&config, &state, &fixture, &bytes, "flac").await;

    let (status, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes).await;
    assert_eq!(status, StatusCode::OK);

    let (committed, _) = commit(&config, &state, &fixture.token, &session).await;

    assert_eq!(committed, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        0
    );
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    let leftovers: Vec<_> = std::fs::read_dir(root.join(".waveflow-managed"))
        .map(|dir| dir.filter_map(Result::ok).collect())
        .unwrap_or_default();
    assert!(
        leftovers.is_empty(),
        "the file that could not be read was removed: {leftovers:?}"
    );
}

/// A session is not an authorisation the client carries away with it.
///
/// The server already holds this rule for playback: a stream ticket re-checks
/// membership on each redemption so revoking access takes effect immediately
/// rather than when the ticket expires. A transfer lasts far longer than a
/// ticket and costs far more, so a member removed mid-transfer must stop
/// writing at the next request rather than at the end of the file.
#[tokio::test]
async fn a_member_removed_mid_transfer_stops_writing() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (owner_fixture, bytes) = ready_to_send(&config, &state, "revoked-owner").await;

    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let manager = state
        .db
        .create_account("revoked-manager", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    state
        .db
        .add_library_member(
            owner_fixture.owner,
            owner_fixture.library,
            manager,
            LibraryRole::Manager,
            now_ms(),
        )
        .await
        .unwrap();
    let token = login_token(
        &waveflow_server::app(&config, state.clone()),
        "revoked-manager",
        &password,
    )
    .await;
    let as_manager = Fixture {
        owner: manager,
        library: owner_fixture.library,
        token,
    };

    let session = open_session(&config, &state, &as_manager, &bytes, "wav").await;
    let (first, _) = put_chunk(
        &config,
        &state,
        &as_manager.token,
        &session,
        0,
        &bytes[..512],
    )
    .await;
    assert_eq!(first, StatusCode::OK);

    state
        .db
        .remove_library_member(
            owner_fixture.owner,
            owner_fixture.library,
            manager,
            now_ms(),
        )
        .await
        .unwrap();

    let (after, _) = put_chunk(
        &config,
        &state,
        &as_manager.token,
        &session,
        1,
        &bytes[512..1024],
    )
    .await;
    assert_eq!(
        after,
        StatusCode::NOT_FOUND,
        "a session must not outlive the membership that justified it"
    );
    let (committed, _) = commit(&config, &state, &as_manager.token, &session).await;
    assert_eq!(
        committed,
        StatusCode::NOT_FOUND,
        "and the commit is re-checked too, not only the fragments"
    );
    assert_eq!(
        state
            .db
            .list_tracks_for_user(owner_fixture.owner, owner_fixture.library)
            .await
            .unwrap()
            .len(),
        0
    );
}

/// A library closed while a transfer was running stops it, and says which it
/// was.
///
/// Not a 404: the caller is still entitled to be here. It is the library that
/// stopped taking files, and saying so is what lets a client stop instead of
/// retrying.
#[tokio::test]
async fn a_library_closed_mid_transfer_stops_the_session() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "closed-midway").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, false, now_ms())
        .await
        .unwrap();

    let (after, _) = put_chunk(
        &config,
        &state,
        &fixture.token,
        &session,
        1,
        &bytes[512..1024],
    )
    .await;
    assert_eq!(after, StatusCode::CONFLICT);
    let (committed, _) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(committed, StatusCode::CONFLICT);
}

/// A half-written file is never something a scan can index.
///
/// Not because the directory is hidden — nothing about the walk skips hidden
/// directories, and counting on that would be an illusion. Because the walk
/// recognises a file by its extension, and a fragment carries none it knows.
///
/// The fragment here is a **complete, playable WAV** that is only part of what
/// the transfer promised. That is the case that matters: a truncated file would
/// fail to parse and be skipped whatever it was called, so a test built on one
/// proves nothing about the naming rule. This one a scan would happily index —
/// as a track whose hash stops being true the moment the next fragment lands.
#[tokio::test]
async fn nothing_half_written_is_ever_visible_to_a_scan() {
    // Built before the app, because the fragment size has to be the whole
    // file's length and the service copies its limits at startup.
    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("half-written-source.wav");
    write_test_wav(&source);
    let whole = std::fs::read(&source).unwrap();

    // A fragment the size of the whole file, and a promise of more to come: the
    // staging area ends up holding a file that is valid on its own.
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = whole.len() as i64).await;
    let fixture = fixture(&config, &state, "half-written").await;
    state
        .db
        .set_library_accepts_uploads(fixture.owner, fixture.library, true, now_ms())
        .await
        .unwrap();
    let (status, body) = negotiate(
        &config,
        &state,
        &fixture,
        vec![serde_json::json!({
            "full_hash": blake3_of(b"a longer file that will never finish arriving"),
            "size_bytes": whole.len() + 512,
            "extension": "wav",
        })],
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["verdicts"][0]["decision"], "accepted");
    let session = body["verdicts"][0]["session"]["session_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let (sent, sent_body) = put_chunk(&config, &state, &fixture.token, &session, 0, &whole).await;
    assert_eq!(sent, StatusCode::OK);
    assert_eq!(sent_body["received_bytes"], whole.len() as i64);

    let library = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap();
    // The staging file really is inside the library the scan walks, and it
    // really is a file a scan could read — so this test fails for the right
    // reason if the naming rule ever changes.
    let staged = library
        .root_path
        .join(".waveflow-managed")
        .join(format!("{session}.part"));
    assert!(staged.is_file());
    assert_eq!(std::fs::read(&staged).unwrap(), whole);

    run_scan(&state, fixture.owner, library).await;

    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        0,
        "an unfinished transfer must not become a track whose hash stops being true"
    );
}

/// A received file survives the scan that was already running when it landed.
///
/// That scan started its walk before the file existed, so it cannot have found
/// it. Sweeping it would mark a file that is on disk as gone and announce a
/// deletion to every client seconds after announcing the arrival.
#[tokio::test]
async fn a_received_file_survives_a_scan_that_began_before_it() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 4096).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "survives").await;

    // A scan that began before anything was sent — the one whose walk cannot
    // have seen the file.
    let running = state
        .db
        .create_scan_job(fixture.library, Some(fixture.owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(running, 0, false).await.unwrap();

    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (sent, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes).await;
    assert_eq!(sent, StatusCode::OK);
    let (status, committed) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(status, StatusCode::CREATED);
    let track_id = committed["track_id"].as_str().unwrap().to_owned();

    // And now that scan finishes, sweeping what it did not find.
    let swept = state
        .db
        .mark_unseen_unavailable(fixture.library, running)
        .await
        .unwrap();
    assert_eq!(swept, 0);
    let available: i64 = sqlx::query_scalar("SELECT is_available FROM track WHERE id=?")
        .bind(&track_id)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(available, 1, "the file is on disk; it is not gone");
    let deletes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM library_event WHERE library_id=? AND action='delete'",
    )
    .bind(fixture.library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(deletes, 0, "and no deletion was announced");

    // The next ordinary scan walks past it and stamps it, after which it is an
    // ordinary track in every respect.
    let library = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap();
    run_scan(&state, fixture.owner, library).await;
    let stamped: Option<String> =
        sqlx::query_scalar("SELECT last_seen_scan_id FROM track WHERE id=?")
            .bind(&track_id)
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(
        stamped.is_some(),
        "the first scan to walk past a received file stamps it"
    );
    assert_eq!(
        state
            .db
            .list_tracks_for_user(fixture.owner, fixture.library)
            .await
            .unwrap()
            .len(),
        1,
        "and it does not become a second track"
    );
}

/// Committing too early is refused, and costs the client nothing.
///
/// The size check is cheap and it comes first for a reason that is not
/// redundancy: without it the hash would catch the short file anyway, but by
/// then it looks like bytes that are not what was promised — and that verdict
/// destroys the session and its staging area. A client that asked too soon
/// would lose the transfer it had already paid for.
#[tokio::test]
async fn committing_an_unfinished_transfer_does_not_destroy_it() {
    let (_temp, config, state) = upload_app(|limits| limits.chunk_bytes = 512).await;
    let (fixture, bytes) = ready_to_send(&config, &state, "premature").await;
    let session = open_session(&config, &state, &fixture, &bytes, "wav").await;
    let (first, _) = put_chunk(&config, &state, &fixture.token, &session, 0, &bytes[..512]).await;
    assert_eq!(first, StatusCode::OK);

    let (early, _) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(early, StatusCode::UNPROCESSABLE_ENTITY);

    assert_eq!(
        open_sessions(&state).await,
        1,
        "asking too soon must not close the session"
    );
    let root = state
        .db
        .library_for_user(fixture.owner, fixture.library)
        .await
        .unwrap()
        .unwrap()
        .root_path;
    assert!(
        root.join(".waveflow-managed")
            .join(format!("{session}.part"))
            .is_file(),
        "nor throw away what had already arrived"
    );

    // And the transfer picks up exactly where it left off.
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
    let (status, committed) = commit(&config, &state, &fixture.token, &session).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(committed["full_hash"], blake3_of(&bytes));
}
