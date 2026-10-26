//! The change feed (RFC-007): sweeps, album events, retention and
//! acknowledgements.

use super::*;

/// A track no scan ever stamped is not a track a scan failed to find.
///
/// `apply_catalog_track_in_transaction` writes a NULL scan for a row that did
/// not come from a scan — a received file. A scan that starts its walk before
/// that file exists cannot find it, and sweeping it would mark a file that is
/// on disk as gone and announce a deletion to every client. A row that predates
/// the scan is the other case: the walk should have found it, and it is swept
/// as before.
#[tokio::test]
async fn a_row_that_never_came_from_a_scan_is_swept_only_if_it_predates_it() {
    let (_temp, config, state) = test_app().await;
    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("unstamped", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("unstamped");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Unstamped",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    let seed = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(seed, 2, false).await.unwrap();
    let mut older = browse_input(
        90_001,
        "Older",
        "Unstamped album",
        "Unstamped artist",
        Some(1),
        Some(1),
    );
    older.full_hash = format!("{:064x}", 0x9001);
    let mut newer = browse_input(
        90_002,
        "Newer",
        "Unstamped album",
        "Unstamped artist",
        Some(2),
        Some(1),
    );
    newer.full_hash = format!("{:064x}", 0x9002);
    for input in [&older, &newer] {
        state
            .db
            .apply_catalog_track(library, seed, input, None, false)
            .await
            .unwrap();
    }

    // The scan that will do the sweeping, and the moment it began.
    let sweeper = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(sweeper, 0, false).await.unwrap();
    let started_at: i64 = sqlx::query_scalar("SELECT started_at FROM scan_job WHERE id=?")
        .bind(sweeper.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();

    // Both rows lose their scan stamp — the state a received file is written
    // in — and are placed either side of the moment the walk began.
    sqlx::query(
        "UPDATE track SET last_seen_scan_id=NULL, created_at=? \
         WHERE library_id=? AND full_hash=?",
    )
    .bind(started_at - 1)
    .bind(library.to_string())
    .bind(format!("{:064x}", 0x9001))
    .execute(state.db.pool())
    .await
    .unwrap();
    sqlx::query(
        "UPDATE track SET last_seen_scan_id=NULL, created_at=? \
         WHERE library_id=? AND full_hash=?",
    )
    .bind(started_at + 1)
    .bind(library.to_string())
    .bind(format!("{:064x}", 0x9002))
    .execute(state.db.pool())
    .await
    .unwrap();

    let swept = state
        .db
        .mark_unseen_unavailable(library, sweeper)
        .await
        .unwrap();

    assert_eq!(swept, 1, "only the row that predates the walk is gone");
    let still_here: i64 =
        sqlx::query_scalar("SELECT is_available FROM track WHERE library_id=? AND full_hash=?")
            .bind(library.to_string())
            .bind(format!("{:064x}", 0x9002))
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(
        still_here, 1,
        "a file that landed after the walk began is not one the walk failed to find"
    );
    let predating: i64 =
        sqlx::query_scalar("SELECT is_available FROM track WHERE library_id=? AND full_hash=?")
            .bind(library.to_string())
            .bind(format!("{:064x}", 0x9001))
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(
        predating, 0,
        "a row the walk should have found is still swept"
    );

    // And the sweep must have said so on the feed, once.
    let deletes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM library_event \
         WHERE library_id=? AND action='delete' AND entity_type='track'",
    )
    .bind(library.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(deletes, 1);
}

/// RFC-007 left open whether an album needs an event of its own; the desktop
/// named the case that says it does, and this is the shape of the answer.
#[tokio::test]
async fn an_album_that_is_merely_retagged_announces_itself_once() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("album-feed", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("album-feed-music");
    std::fs::create_dir_all(&music).unwrap();
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Album feed",
            &root,
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    // Three tracks of one album and one of another. What the feed must not do
    // is announce the first album three times.
    // `existing` is what a rescan carries: the apply matches a file to the row
    // it already has, and passing `None` a second time would try to insert a
    // second track at a path the schema makes unique.
    let apply =
        |index: usize, album: &'static str, year: Option<i64>, existing: Option<uuid::Uuid>| {
            let db = state.db.clone();
            async move {
                let scan = db
                    .create_scan_job(library, Some(owner), "manual")
                    .await
                    .unwrap();
                db.start_scan_job(scan, 1, false).await.unwrap();
                let mut input = catalog_input(index, "Nova Kern");
                input.title = format!("Track {index}");
                input.album = Some(album.into());
                input.album_artist = Some("Nova Kern".into());
                input.year = year;
                db.apply_catalog_track(library, scan, &input, existing, false)
                    .await
                    .unwrap();
                db.finish_scan_job(scan, 0).await.unwrap();
            }
        };
    for index in 0..3 {
        apply(index, "One", Some(2000), None).await;
    }
    apply(3, "Two", Some(2000), None).await;
    let first = state
        .db
        .list_tracks_for_user(owner, library)
        .await
        .unwrap()
        .into_iter()
        .find(|track| track.title == "Track 0")
        .expect("the first track")
        .id;

    let albums_on_feed = |after: i64| {
        let services = state.services.clone();
        async move {
            services
                .library_changes(owner, library, after, 500)
                .await
                .unwrap()
                .events
                .into_iter()
                .filter(|event| event.entity_type == "album")
                .collect::<Vec<_>>()
        }
    };

    let albums = albums_on_feed(0).await;
    assert_eq!(
        albums.len(),
        2,
        "one event per album, not one per track: {albums:?}"
    );
    assert!(albums.iter().all(|event| event.action == "upsert"));
    let cursor = state
        .services
        .library_changes(owner, library, 0, 500)
        .await
        .unwrap()
        .events
        .last()
        .unwrap()
        .cursor;

    // Applying the same tags again writes nothing, so it announces nothing. An
    // album event per rescan would make the feed a scan log and cost the client
    // the refetch this event exists to spare it.
    apply(0, "One", Some(2000), Some(first)).await;
    assert!(
        albums_on_feed(cursor).await.is_empty(),
        "an unchanged album is not a change"
    );

    // And the case the desktop named: the album is retagged without gaining or
    // losing a track, so `song_count` does not move and an incremental walk
    // keyed on it would skip the album entirely.
    apply(0, "One", Some(2001), Some(first)).await;
    let announced = albums_on_feed(cursor).await;
    assert_eq!(announced.len(), 1, "the retag is announced: {announced:?}");
    assert_eq!(announced[0].action, "upsert");
}

/// RFC-007 decision 7: an age and a floor, the floor winning, and the age bound
/// exclusive. Nothing purged before this, so the feed's expiry answer was
/// correct code that had never once run.
#[tokio::test]
async fn retention_cuts_by_age_never_below_the_floor_and_never_at_the_bound() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    // Tuned before `initialize`: `DomainServices` copies these out of `Config`
    // when it is built, so raising a bound on the returned one changes nothing.
    config.library_event_retention.min_events = 2;
    config.library_event_retention.days = 30;
    let state = waveflow_server::initialize(&config).await.unwrap();

    let hash = security::hash_password("correct horse battery staple").unwrap();
    let owner = state
        .db
        .create_account("retention", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("retention-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Retention",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    // Five events through the ordinary path. No album on the inputs, so each
    // track writes one event and nothing else — an album upsert emits its own
    // since #159, and a count this test reasons about must not include them.
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 5, false).await.unwrap();
    for index in 0..5usize {
        let mut input = catalog_input(index, "Nova Kern");
        input.title = format!("Track {index}");
        input.album = None;
        input.album_artist = None;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let cursors: Vec<i64> =
        sqlx::query_scalar("SELECT cursor FROM library_event WHERE library_id=? ORDER BY cursor")
            .bind(library.to_string())
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert_eq!(cursors.len(), 5, "one event per track and no album events");

    // Backdated in SQL because the ages are what this test is about: two far
    // past the bound, one exactly on it, two inside.
    //
    // `now` is handed to the purge rather than read by it. The bound is
    // exclusive, so "exactly thirty days old" is a single instant: read the
    // clock twice and the boundary event lands a millisecond on the wrong side,
    // and this test passes or fails on how long the lines above it took.
    let day = 24 * 60 * 60 * 1000i64;
    let now = now_ms();
    for (cursor, age) in cursors
        .iter()
        .zip([40 * day, 35 * day, 30 * day, 10 * day, 0])
    {
        sqlx::query("UPDATE library_event SET changed_at=? WHERE cursor=?")
            .bind(now - age)
            .bind(cursor)
            .execute(state.db.pool())
            .await
            .unwrap();
    }

    let purged = state.services.purge_library_events(now).await.unwrap();
    assert_eq!(purged.libraries_trimmed, 1);
    // Two go. The thirty-day one sits exactly on the bound and the bound is
    // exclusive — keeping one event too many breaks nobody, cutting one too
    // many sends somebody back to the snapshot.
    assert_eq!(purged.events_removed, 2, "only what is strictly older");

    // The watermark moved with the delete, in the same transaction. Written
    // separately there is a window where it claims less than has gone, and a
    // client reading into it gets a catch-up that looks complete while skipping
    // the gap.
    let watermark: i64 = sqlx::query_scalar("SELECT events_purged_through FROM library WHERE id=?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(watermark, cursors[1], "the highest cursor actually cut");
    assert_eq!(
        state
            .services
            .library_changes(owner, library, watermark, 500)
            .await
            .unwrap()
            .events
            .len(),
        3,
        "the bound and everything after it stay"
    );
    assert!(
        matches!(
            state
                .services
                .library_changes(owner, library, watermark - 1, 500)
                .await,
            Err(ServiceError::Conflict)
        ),
        "a cursor below the watermark has missed events"
    );

    // And the floor wins over the age. Everything left is now ancient, and the
    // floor is two: exactly one may go.
    sqlx::query("UPDATE library_event SET changed_at=? WHERE library_id=?")
        .bind(now - 400 * day)
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let purged = state.services.purge_library_events(now).await.unwrap();
    assert_eq!(
        purged.events_removed, 1,
        "three rows, a floor of two: one may go however old they all are"
    );

    // A second pass changes nothing: the floor is reached and stays reached.
    assert_eq!(
        state.services.purge_library_events(now).await.unwrap(),
        Default::default(),
        "a library at its floor is not trimmed again"
    );
    let watermark: i64 = sqlx::query_scalar("SELECT events_purged_through FROM library WHERE id=?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        state
            .services
            .library_changes(owner, library, watermark, 500)
            .await
            .unwrap()
            .events
            .len(),
        2,
        "the floor kept a usable tail rather than an empty feed"
    );
}

/// RFC-007 decision 8. The acknowledgement says how far a device has read; what
/// it deliberately does not do is hold the purge back.
#[tokio::test]
async fn an_acknowledgement_records_a_device_without_holding_the_purge_back() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.library_event_retention.min_events = 1;
    config.library_event_retention.days = 30;
    let state = waveflow_server::initialize(&config).await.unwrap();

    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("ack", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("ack-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Ack",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 3, false).await.unwrap();
    for index in 0..3usize {
        let mut input = catalog_input(index, "Nova Kern");
        input.title = format!("Track {index}");
        input.album = None;
        input.album_artist = None;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let router = waveflow_server::app(&config, state.clone());
    let (token, device) = login_session(&router, "ack", &password).await;
    let cursors: Vec<i64> =
        sqlx::query_scalar("SELECT cursor FROM library_event WHERE library_id=? ORDER BY cursor")
            .bind(library.to_string())
            .fetch_all(state.db.pool())
            .await
            .unwrap();

    let ack = |cursor: i64, device: String, token: String| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::put(format!("/api/v2/libraries/{library}/events/ack"))
                        .header("authorization", format!("Bearer {token}"))
                        .header("content-type", "application/json")
                        .body(Body::from(
                            serde_json::json!({ "device_id": device, "cursor": cursor })
                                .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }
    };

    assert_eq!(
        ack(cursors[0], device.clone(), token.clone()).await,
        StatusCode::NO_CONTENT
    );
    // A cursor beyond what the feed has written would let a client mark itself
    // caught up with events that do not exist yet, and be silently behind when
    // they arrive.
    assert_eq!(
        ack(cursors[2] + 1, device.clone(), token.clone()).await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Somebody else's device, refused the same way a library one cannot see is
    // — one answer for all of them, or the refusal tells them apart.
    let stranger_hash = security::hash_password("correct horse battery staple").unwrap();
    state
        .db
        .create_account("ack-stranger", &stranger_hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let (stranger_token, stranger_device) =
        login_session(&router, "ack-stranger", "correct horse battery staple").await;
    assert_eq!(
        ack(cursors[0], stranger_device.clone(), token.clone()).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a device that is not this account's"
    );
    // Their own device, and a library they are not a member of. This is the
    // case the membership join answers — the one above is refused a step
    // earlier, by the device check, so it says nothing about tenancy.
    assert_eq!(
        ack(cursors[0], stranger_device, stranger_token).await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "a library this account is not a member of"
    );

    // Never lowered: two of a client's own requests racing must not let the
    // older one win.
    assert_eq!(
        ack(cursors[2], device.clone(), token.clone()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        ack(cursors[0], device.clone(), token).await,
        StatusCode::NO_CONTENT
    );
    let stored: i64 = sqlx::query_scalar(
        "SELECT cursor FROM library_event_ack WHERE library_id=? AND device_id=?",
    )
    .bind(library.to_string())
    .bind(&device)
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(stored, cursors[2], "the highest acknowledged cursor stands");

    // And the point of the whole table: it does not hold the purge back. This
    // device has acknowledged everything, then the feed is aged out anyway —
    // one forgotten phone must not stop a shared library being trimmed.
    sqlx::query("UPDATE library_event_ack SET cursor=? WHERE library_id=?")
        .bind(cursors[0])
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let now = now_ms();
    sqlx::query("UPDATE library_event SET changed_at=? WHERE library_id=?")
        .bind(now - 400 * 24 * 60 * 60 * 1000i64)
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let purged = state.services.purge_library_events(now).await.unwrap();
    assert_eq!(
        purged.events_removed, 2,
        "the floor of one keeps the newest"
    );
    assert_eq!(
        purged.devices_stranded, 1,
        "and the cut is reported against the device it sent back"
    );

    // A pass answers for what it cost, not for what it inherits. Write two more
    // events, age them, and trim again: the device is still below the
    // watermark, but this pass is not what put it there.
    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 2, false).await.unwrap();
    for index in 3..5usize {
        let mut input = catalog_input(index, "Nova Kern");
        input.title = format!("Track {index}");
        input.album = None;
        input.album_artist = None;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();
    sqlx::query("UPDATE library_event SET changed_at=? WHERE library_id=?")
        .bind(now - 400 * 24 * 60 * 60 * 1000i64)
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    let purged = state.services.purge_library_events(now).await.unwrap();
    assert!(purged.events_removed > 0, "there was something left to cut");
    assert_eq!(
        purged.devices_stranded, 0,
        "already stranded is not stranded again — a bill that only grows is not a bill"
    );
}

/// The acknowledgement's ceiling counts the watermark, not only the rows.
///
/// **The configuration here is one the server would refuse to start with.**
/// `parse_positive_env` rejects a floor of zero, so a feed can never actually
/// be trimmed to nothing in production, and this state is unreachable. The
/// guard exists anyway because "the furthest position a client could have
/// reached" should not be an expression that happens to be right thanks to a
/// bound enforced three files away — and a test that never reaches the branch
/// is a guard nobody can check. `Config::for_data_dir` does not validate, which
/// is what lets this be written at all.
#[tokio::test]
async fn a_cursor_at_the_watermark_is_acknowledged_even_with_the_feed_emptied() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.library_event_retention.min_events = 0;
    config.library_event_retention.days = 30;
    let state = waveflow_server::initialize(&config).await.unwrap();

    let password = security::generate_token("test-password-");
    let hash = security::hash_password(&password).unwrap();
    let owner = state
        .db
        .create_account("watermark-ack", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("watermark-ack-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Watermark",
            &std::fs::canonicalize(&music).unwrap(),
            LibraryVisibility::Private,
            now_ms(),
        )
        .await
        .unwrap();

    let scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(scan, 2, false).await.unwrap();
    for index in 0..2usize {
        let mut input = catalog_input(index, "Nova Kern");
        input.title = format!("Track {index}");
        input.album = None;
        input.album_artist = None;
        state
            .db
            .apply_catalog_track(library, scan, &input, None, false)
            .await
            .unwrap();
    }
    state.db.finish_scan_job(scan, 0).await.unwrap();

    let now = now_ms();
    sqlx::query("UPDATE library_event SET changed_at=? WHERE library_id=?")
        .bind(now - 400 * 24 * 60 * 60 * 1000i64)
        .bind(library.to_string())
        .execute(state.db.pool())
        .await
        .unwrap();
    state.services.purge_library_events(now).await.unwrap();

    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM library_event WHERE library_id=?")
            .bind(library.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 0, "the floor of zero let the feed empty");
    let watermark: i64 = sqlx::query_scalar("SELECT events_purged_through FROM library WHERE id=?")
        .bind(library.to_string())
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(watermark > 0);

    // `library_changes` accepts reading from the watermark, so the
    // acknowledgement has to accept recording it — the two must not disagree
    // about the same number.
    let router = waveflow_server::app(&config, state.clone());
    let (token, device) = login_session(&router, "watermark-ack", &password).await;
    assert!(state
        .services
        .library_changes(owner, library, watermark, 500)
        .await
        .is_ok());
    let response = router
        .oneshot(
            Request::put(format!("/api/v2/libraries/{library}/events/ack"))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "device_id": device, "cursor": watermark }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}
