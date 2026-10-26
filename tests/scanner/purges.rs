//! Purging missing tracks, taking a purge back, and the purge window (RFC-021
//! decision 4).

use super::*;

/// RFC-021 decision 4: the listing says what a purge would cost, and the
/// purge removes only what is still missing, with what hangs off it, and
/// leaves no album, favourite or search row behind with nothing to stand on.
#[tokio::test]
async fn a_purge_removes_only_what_is_still_missing() {
    let (temp, state, owner, library, root) = library_with_albums("purge").await;
    let first = track_id(&state, "First/0.flac").await;
    let second = track_id(&state, "Second/2.flac").await;
    // A loop on the track that goes: its link cascades, and the blob it was
    // the last to name must go with it rather than sit in the store for ever.
    let canvas = "c".repeat(64);
    let canvas_file = temp
        .path()
        .join("data/canvas")
        .join(format!("{canvas}.mp4"));
    std::fs::create_dir_all(canvas_file.parent().unwrap()).unwrap();
    std::fs::write(&canvas_file, b"mp4").unwrap();
    sqlx::query("INSERT INTO canvas (hash, format, byte_size, created_at) VALUES (?, 'mp4', 3, 1)")
        .bind(&canvas)
        .execute(state.db.pool())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO track_canvas (track_id, library_id, canvas_hash, created_at) \
         VALUES (?, ?, ?, 1)",
    )
    .bind(&second)
    .bind(library.id.to_string())
    .bind(&canvas)
    .execute(state.db.pool())
    .await
    .unwrap();
    let second_album: String = sqlx::query_scalar("SELECT album_id FROM track WHERE id = ?")
        .bind(&second)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    let playlist = uuid::Uuid::new_v4().to_string();
    for (statement, binds) in [
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'track', ?, 1)",
            vec![owner.to_string(), second.clone()],
        ),
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'album', ?, 1)",
            vec![owner.to_string(), second_album.clone()],
        ),
        (
            "INSERT INTO playlist (id, owner_user_id, name, public, created_at, updated_at) \
             VALUES (?, ?, 'Kept', 0, 1, 1)",
            vec![playlist.clone(), owner.to_string()],
        ),
        (
            "INSERT INTO playlist_track (playlist_id, track_id, position, added_at) \
             VALUES (?, ?, 0, 1)",
            vec![playlist.clone(), second.clone()],
        ),
        (
            "INSERT INTO play_event (user_id, track_id, submission, played_at) \
             VALUES (?, ?, 1, 1)",
            vec![owner.to_string(), second.clone()],
        ),
    ] {
        let mut query = sqlx::query(statement);
        for bind in binds {
            query = query.bind(bind);
        }
        query.execute(state.db.pool()).await.unwrap();
    }

    let first_bytes = std::fs::read(root.join("First/0.flac")).unwrap();
    std::fs::remove_file(root.join("First/0.flac")).unwrap();
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;

    let listed = state
        .services
        .unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::BrowsePage::default(),
        )
        .await
        .unwrap();
    assert_eq!(listed.total, 2);
    let costly = listed
        .tracks
        .iter()
        .find(|track| track.id.to_string() == second)
        .unwrap();
    assert_eq!(
        (costly.playlist_entries, costly.favorites, costly.plays),
        (1, 1, 1)
    );

    // One of them comes back between the listing and the purge.
    std::fs::write(root.join("First/0.flac"), first_bytes).unwrap();
    scan_once(&state, owner, library.clone()).await;

    let purged = state
        .services
        .purge_unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            waveflow_server::services::PurgeRequest {
                track_ids: Some(vec![first.parse().unwrap(), second.parse().unwrap()]),
                all: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(purged.purged, 1, "a track that came back must be kept");
    // Out of sight at once, deleted once the grace window has passed.
    assert!(purged.restorable_until.is_some());
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE is_available = 0 AND purged_at IS NULL"
        )
        .await,
        0
    );
    assert_eq!(state.services.finish_purges(now_ms()).await.unwrap(), 1);
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 0").await,
        0
    );
    assert_eq!(
        available_paths(&state).await,
        ["First/0.flac", "First/1.flac"]
    );
    for (query, what) in [
        ("SELECT COUNT(*) FROM playlist_track", "the playlist entry"),
        ("SELECT COUNT(*) FROM play_event", "the play"),
        ("SELECT COUNT(*) FROM user_star", "the favourites"),
        (
            "SELECT COUNT(*) FROM album \
             WHERE NOT EXISTS (SELECT 1 FROM track t WHERE t.album_id = album.id)",
            "an empty album",
        ),
        (
            "SELECT COUNT(*) FROM track_fts WHERE track_id NOT IN (SELECT id FROM track)",
            "a search row",
        ),
    ] {
        assert_eq!(count(&state, query).await, 0, "{what} outlived the purge");
    }
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM playlist").await,
        1,
        "the playlist itself is not the track's to take"
    );
    assert_eq!(count(&state, "SELECT COUNT(*) FROM canvas").await, 0);
    assert!(
        !canvas_file.exists(),
        "the purged track's canvas stayed in the store"
    );
}

/// RFC-021 decision 4: a purge is explicit, refused while the library is
/// scanned, and for curators only; the retention pass purges by age.
#[tokio::test]
async fn a_purge_is_explicit_and_waits_for_no_scan() {
    use waveflow_server::services::{PurgeRequest, ServiceError};

    let (_temp, state, owner, library, root) = library_with_albums("purge-rules").await;
    std::fs::remove_file(root.join("First/0.flac")).unwrap();
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;

    let neither = state
        .services
        .purge_unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(owner),
            library.id,
            PurgeRequest::default(),
        )
        .await;
    assert!(
        matches!(neither, Err(ServiceError::Invalid)),
        "a purge naming nothing must be refused as invalid"
    );

    let listener = insert_account(&state, "purge-listener", "user").await;
    state
        .db
        .add_library_member(
            owner,
            library.id,
            listener,
            waveflow_server::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let all = || PurgeRequest {
        track_ids: None,
        all: true,
    };
    assert!(matches!(
        state
            .services
            .purge_unavailable_tracks(
                waveflow_server::services::LibraryActor::operator(listener),
                library.id,
                all()
            )
            .await,
        Err(ServiceError::Forbidden)
    ));
    assert!(matches!(
        state
            .services
            .unavailable_tracks(
                waveflow_server::services::LibraryActor::operator(listener),
                library.id,
                Default::default()
            )
            .await,
        Err(ServiceError::Forbidden)
    ));

    let scanning = state.scanner.try_lock_library(library.id).unwrap();
    assert!(matches!(
        state
            .services
            .purge_unavailable_tracks(
                waveflow_server::services::LibraryActor::operator(owner),
                library.id,
                all()
            )
            .await,
        Err(ServiceError::Conflict)
    ));
    // During that scan, somebody who may not purge is refused as such, and a
    // stranger is told there is no library: a `Conflict` would say there is
    // one, and that it is being scanned.
    let stranger = insert_account(&state, "purge-stranger", "user").await;
    let refused = state
        .services
        .purge_unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(stranger),
            library.id,
            all(),
        )
        .await;
    assert!(
        matches!(refused, Err(ServiceError::NotFound)),
        "a stranger must be told there is no library"
    );
    let refused = state
        .services
        .purge_unavailable_tracks(
            waveflow_server::services::LibraryActor::operator(listener),
            library.id,
            all(),
        )
        .await;
    assert!(
        matches!(refused, Err(ServiceError::Forbidden)),
        "a listener may see the library and may not purge it"
    );
    assert_eq!(
        state
            .services
            .purge_expired_unavailable_tracks(i64::MAX)
            .await
            .unwrap(),
        0,
        "the retention pass must leave a library being scanned alone"
    );
    drop(scanning);

    // One went missing long ago, the other just now.
    sqlx::query("UPDATE track SET unavailable_since = 1000 WHERE relative_path = 'First/0.flac'")
        .execute(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        state
            .services
            .purge_expired_unavailable_tracks(now_ms() - 60_000)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE is_available = 0 AND purged_at IS NULL"
        )
        .await,
        1,
        "the retention purges like anyone else, behind the same grace window"
    );
}

/// The journal entries one account was written, as `(entity_type, entity_id,
/// action)`, oldest first.
async fn journal_of(
    state: &waveflow_server::AppState,
    user: uuid::Uuid,
) -> Vec<(String, String, String, serde_json::Value)> {
    sqlx::query_as::<_, (String, String, String, String)>(
        "SELECT entity_type, entity_id, action, payload_json FROM sync_event \
         WHERE user_id = ? ORDER BY cursor",
    )
    .bind(user.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap()
    .into_iter()
    .map(|(kind, id, action, payload)| (kind, id, action, serde_json::from_str(&payload).unwrap()))
    .collect()
}

/// RFC-021, what was still open: a purge can be taken back within its grace
/// window — explicitly, or by the file coming back — and when it does land,
/// every account that held a favourite, a rating or a playlist entry of what
/// went is told through its own sync journal, as a library deletion tells
/// them too.
#[tokio::test]
async fn a_purge_can_be_taken_back_and_tells_every_journal_when_it_lands() {
    use waveflow_server::services::{LibraryActor, PurgeRequest};

    let (_temp, state, owner, library, root) = library_with_albums("undo").await;
    let actor = LibraryActor::operator(owner);
    let listener = insert_account(&state, "undo-listener", "user").await;
    state
        .db
        .add_library_member(
            owner,
            library.id,
            listener,
            waveflow_server::database::LibraryRole::Listener,
            now_ms(),
        )
        .await
        .unwrap();
    let doomed = track_id(&state, "Second/2.flac").await;
    let kept = track_id(&state, "First/1.flac").await;
    let doomed_album: String = sqlx::query_scalar("SELECT album_id FROM track WHERE id = ?")
        .bind(&doomed)
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    for (statement, user, entity) in [
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'track', ?, 1)",
            listener,
            doomed.clone(),
        ),
        (
            "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
             VALUES (?, 'album', ?, 1)",
            listener,
            doomed_album.clone(),
        ),
        (
            "INSERT INTO user_rating (user_id, entity_type, entity_id, rating, updated_at) \
             VALUES (?, 'track', ?, 4, 1)",
            owner,
            doomed.clone(),
        ),
    ] {
        sqlx::query(statement)
            .bind(user.to_string())
            .bind(entity)
            .execute(state.db.pool())
            .await
            .unwrap();
    }
    let playlist = state
        .services
        .create_playlist(
            listener,
            "Mixed",
            &[kept.parse().unwrap(), doomed.parse().unwrap()],
        )
        .await
        .unwrap();
    let journalled = |entries: &[(String, String, String, serde_json::Value)]| entries.len();
    let listener_before = journalled(&journal_of(&state, listener).await);
    let owner_before = journalled(&journal_of(&state, owner).await);

    let bytes = std::fs::read(root.join("Second/2.flac")).unwrap();
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;
    let only_doomed = || PurgeRequest {
        track_ids: Some(vec![doomed.parse().unwrap()]),
        all: false,
    };

    // Purged: out of the missing listing, into the purged one, nothing gone.
    let purged = state
        .services
        .purge_unavailable_tracks(actor, library.id, only_doomed())
        .await
        .unwrap();
    assert_eq!(purged.purged, 1);
    let until = purged.restorable_until.expect("a grace window by default");
    let missing = state
        .services
        .unavailable_tracks(actor, library.id, Default::default())
        .await
        .unwrap();
    assert_eq!(missing.total, 0);
    let held = state
        .services
        .purged_tracks(actor, library.id, Default::default())
        .await
        .unwrap();
    assert_eq!(held.total, 1);
    assert_eq!(held.tracks[0].id.to_string(), doomed);
    assert_eq!(held.tracks[0].deleted_after, Some(until));
    assert_eq!(held.tracks[0].favorites, 1, "nothing is lost yet");

    // Taken back explicitly.
    let restored = state
        .services
        .restore_purged_tracks(actor, library.id, only_doomed())
        .await
        .unwrap();
    assert_eq!(restored.restored, 1);
    assert_eq!(
        state
            .services
            .unavailable_tracks(actor, library.id, Default::default())
            .await
            .unwrap()
            .total,
        1
    );
    assert_eq!(
        state.services.finish_purges(i64::MAX).await.unwrap(),
        0,
        "a restored track is no longer due"
    );

    // Taken back by the file coming back within the window.
    state
        .services
        .purge_unavailable_tracks(actor, library.id, only_doomed())
        .await
        .unwrap();
    std::fs::write(root.join("Second/2.flac"), &bytes).unwrap();
    scan_once(&state, owner, library.clone()).await;
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE purged_at IS NOT NULL"
        )
        .await,
        0,
        "a track found again is not purged any more"
    );
    assert_eq!(state.services.finish_purges(i64::MAX).await.unwrap(), 0);
    assert_eq!(track_id(&state, "Second/2.flac").await, doomed);
    assert_eq!(
        journalled(&journal_of(&state, listener).await),
        listener_before,
        "nothing reached a journal while the purge could be taken back"
    );

    // Purged for good: the window passes.
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;
    state
        .services
        .purge_unavailable_tracks(actor, library.id, only_doomed())
        .await
        .unwrap();
    assert_eq!(state.services.finish_purges(now_ms()).await.unwrap(), 1);
    assert_eq!(
        count(&state, "SELECT COUNT(*) FROM track WHERE is_available = 0").await,
        0
    );

    let listener_journal = journal_of(&state, listener).await;
    let told = &listener_journal[listener_before..];
    assert!(
        told.iter()
            .any(|(kind, id, action, payload)| kind == "favorite"
                && *id == doomed
                && action == "delete"
                && payload["entity_type"] == "track"),
        "the listener's favourite of the track"
    );
    assert!(
        told.iter()
            .any(|(kind, id, action, payload)| kind == "favorite"
                && *id == doomed_album
                && action == "delete"
                && payload["entity_type"] == "album"),
        "the listener's favourite of the album left empty"
    );
    let playlist_entry = told
        .iter()
        .find(|(kind, id, ..)| kind == "playlist" && *id == playlist.id.to_string())
        .expect("the listener's playlist that lost an entry");
    assert_eq!(playlist_entry.2, "upsert");
    assert_eq!(playlist_entry.3["track_ids"], serde_json::json!([kept]));
    assert_eq!(playlist_entry.3["name"], "Mixed");
    let owner_told = journal_of(&state, owner).await;
    assert!(
        owner_told[owner_before..]
            .iter()
            .any(|(kind, id, action, _)| kind == "rating" && *id == doomed && action == "delete"),
        "the owner's rating"
    );

    // A deletion tells them the same way.
    let before_deletion = journal_of(&state, listener).await.len();
    state
        .services
        .delete_library(actor, library.id, "undo")
        .await
        .unwrap();
    let after = journal_of(&state, listener).await;
    let emptied = after[before_deletion..]
        .iter()
        .find(|(kind, ..)| kind == "playlist")
        .expect("the playlist the deletion emptied");
    assert_eq!(emptied.3["track_ids"], serde_json::json!([]));
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(state.db.pool())
        .await
        .unwrap();
    assert!(violations.is_empty());
}

/// A purge past its grace window is due for deletion: neither listed as
/// restorable nor restored, even before the finishing pass has run (review of
/// #335).
#[tokio::test]
async fn a_purge_past_its_window_is_neither_listed_nor_restored() {
    use waveflow_server::services::{LibraryActor, PurgeRequest};

    let (_temp, state, owner, library, root) = library_with_albums("past-window").await;
    let actor = LibraryActor::operator(owner);
    std::fs::remove_file(root.join("Second/2.flac")).unwrap();
    scan_once(&state, owner, library.clone()).await;
    state
        .services
        .purge_unavailable_tracks(
            actor,
            library.id,
            PurgeRequest {
                track_ids: None,
                all: true,
            },
        )
        .await
        .unwrap();
    // Purged eight days ago, against a window of a week.
    sqlx::query("UPDATE track SET purged_at = ? WHERE purged_at IS NOT NULL")
        .bind(now_ms() - 8 * 24 * 60 * 60 * 1000)
        .execute(state.db.pool())
        .await
        .unwrap();
    let listed = state
        .services
        .purged_tracks(actor, library.id, Default::default())
        .await
        .unwrap();
    assert_eq!((listed.total, listed.tracks.len()), (0, 0));
    let restored = state
        .services
        .restore_purged_tracks(
            actor,
            library.id,
            PurgeRequest {
                track_ids: None,
                all: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        restored.restored, 0,
        "a purge past its window is due, not undone"
    );
    assert_eq!(
        count(
            &state,
            "SELECT COUNT(*) FROM track WHERE purged_at IS NOT NULL"
        )
        .await,
        1
    );
}

/// With the grace window set to zero, the purges a former window left behind
/// are deleted by the pass, which therefore runs whatever the settings
/// (review of #335).
#[tokio::test]
async fn a_zero_window_deletes_the_purges_a_former_one_left() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.purge_grace = std::time::Duration::ZERO;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let owner = insert_account(&state, "zero-window", "admin").await;
    let music = temp.path().join("music");
    std::fs::create_dir_all(&music).unwrap();
    write_test_wav(&music.join("Gone.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let id = state
        .db
        .create_library(owner, "zero", &root, LibraryVisibility::Private, now_ms())
        .await
        .unwrap();
    let library = LibraryRecord {
        id,
        name: "zero".into(),
        root_path: root.clone(),
    };
    scan_once(&state, owner, library.clone()).await;
    std::fs::remove_file(root.join("Gone.wav")).unwrap();
    scan_once(&state, owner, library).await;
    // Purged under a former window, an hour ago.
    sqlx::query("UPDATE track SET purged_at = ? WHERE is_available = 0")
        .bind(now_ms() - 60 * 60 * 1000)
        .execute(state.db.pool())
        .await
        .unwrap();
    state.services.spawn_unavailable_purge();
    for _ in 0..200 {
        if count(&state, "SELECT COUNT(*) FROM track").await == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the purge a former window left was never deleted");
}
