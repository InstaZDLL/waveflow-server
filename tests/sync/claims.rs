//! A claim precedes validation, and an invalid one rolls back.

use super::*;

#[tokio::test]
async fn sync_claim_precedes_state_validation_and_invalid_claims_roll_back() {
    let (_temp, config, state) = test_app().await;
    let hash = security::hash_password(&Uuid::new_v4().to_string()).unwrap();
    let owner = state
        .db
        .create_account("claim-owner", &hash, AccountRole::Admin, now_ms())
        .await
        .unwrap();
    let listener = state
        .db
        .create_account("claim-listener", &hash, AccountRole::User, now_ms())
        .await
        .unwrap();
    let music = config.data_dir.join("claim-music");
    std::fs::create_dir_all(&music).unwrap();
    let library = state
        .db
        .create_library(
            owner,
            "Claim library",
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
    state.db.start_scan_job(scan, 1, false).await.unwrap();
    state
        .db
        .apply_catalog_track(
            library,
            scan,
            &browse_input(
                0,
                "Claimed Song",
                "Claimed Album",
                "Claimed Artist",
                Some(1),
                Some(1),
            ),
            None,
            false,
        )
        .await
        .unwrap();
    state.db.finish_scan_job(scan, 0).await.unwrap();
    let track = state.db.list_tracks_for_user(owner, library).await.unwrap()[0].id;

    let playlist_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    let playlist = state
        .services
        .create_playlist_with_context(owner, "Claimed playlist", &[track], playlist_context)
        .await
        .unwrap();
    state
        .services
        .delete_playlist(owner, playlist.id)
        .await
        .unwrap();
    let missing_playlist_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    assert!(matches!(
        state
            .services
            .update_playlist_with_context(
                owner,
                playlist.id,
                Some("Changed after deletion"),
                None,
                None,
                &[],
                &[],
                Default::default(),
                missing_playlist_context,
            )
            .await,
        Err(ServiceError::NotFound)
    ));
    assert!(matches!(
        state
            .services
            .update_playlist_with_context(
                owner,
                playlist.id,
                Some("Divergent replay after deletion"),
                None,
                None,
                &[],
                &[],
                Default::default(),
                playlist_context,
            )
            .await,
        Err(ServiceError::Conflict)
    ));

    state
        .db
        .add_library_member(owner, library, listener, LibraryRole::Listener, now_ms())
        .await
        .unwrap();
    let inaccessible_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    state
        .services
        .set_star_with_context(listener, "track", track, true, inaccessible_context)
        .await
        .unwrap();
    assert!(state
        .db
        .remove_library_member(owner, library, listener, now_ms())
        .await
        .unwrap());
    state
        .services
        .set_star_with_context(listener, "track", track, true, inaccessible_context)
        .await
        .unwrap();
    // The row survives the replay, but a revoked membership must stop exposing
    // it: favourites are filtered by visibility exactly like ratings are.
    assert!(state
        .services
        .starred_ids(listener)
        .await
        .unwrap()
        .iter()
        .all(|(_, entity_id, _)| *entity_id != track));
    assert!(matches!(
        state
            .services
            .set_rating_with_context(listener, "track", track, 5, inaccessible_context)
            .await,
        Err(ServiceError::Conflict)
    ));
    let fresh_inaccessible_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    assert!(matches!(
        state
            .services
            .set_rating_with_context(listener, "track", track, 5, fresh_inaccessible_context)
            .await,
        Err(ServiceError::NotFound)
    ));

    let invalid_replay_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    state
        .services
        .set_rating_with_context(owner, "track", track, 5, invalid_replay_context)
        .await
        .unwrap();
    assert!(matches!(
        state
            .services
            .set_rating_with_context(owner, "track", track, 4, invalid_replay_context)
            .await,
        Err(ServiceError::Conflict)
    ));

    let rolled_back_context = MutationContext {
        operation_id: Uuid::new_v4(),
        origin_device_id: None,
    };
    assert!(matches!(
        state
            .services
            .set_rating_with_context(owner, "track", track, 6, rolled_back_context)
            .await,
        Err(ServiceError::Invalid)
    ));
    let reservation_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sync_operation WHERE user_id=? AND operation_id=?",
    )
    .bind(owner.to_string())
    .bind(rolled_back_context.operation_id.to_string())
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    assert_eq!(reservation_count, 0);
    state
        .services
        .set_rating_with_context(owner, "track", track, 4, rolled_back_context)
        .await
        .unwrap();

    let oversized_queue = vec![track; MAX_QUEUE_TRACKS + 1];
    assert!(matches!(
        state
            .services
            .save_queue(owner, &oversized_queue, Some(track), 0, Some("limit-test"))
            .await,
        Err(ServiceError::Invalid)
    ));
    let oversized_share = vec![track; MAX_SHARE_TRACKS + 1];
    assert!(matches!(
        state
            .services
            .create_share(owner, &oversized_share, Some("limit-test"), None)
            .await,
        Err(ServiceError::Invalid)
    ));

    state
        .services
        .save_queue(
            owner,
            &[track, track],
            Some(track),
            0,
            Some("duplicate-test"),
        )
        .await
        .unwrap();
    let duplicate_queue = state.services.queue(owner).await.unwrap().unwrap();
    assert_eq!(
        duplicate_queue
            .songs
            .iter()
            .map(|song| song.id)
            .collect::<Vec<_>>(),
        vec![track, track]
    );
    let positions = sqlx::query_scalar::<_, i64>(
        "SELECT position FROM play_queue_track WHERE user_id=? ORDER BY position",
    )
    .bind(owner.to_string())
    .fetch_all(state.db.pool())
    .await
    .unwrap();
    assert_eq!(positions, vec![0, 1]);

    let aggregate_playlist = state
        .services
        .create_playlist(owner, "Unavailable aggregate", &[track])
        .await
        .unwrap();
    let aggregate_share = state
        .services
        .create_share(owner, &[track], Some("Unavailable aggregate"), None)
        .await
        .unwrap();
    let empty_scan = state
        .db
        .create_scan_job(library, Some(owner), "manual")
        .await
        .unwrap();
    state.db.start_scan_job(empty_scan, 1, false).await.unwrap();
    assert_eq!(
        state
            .db
            .mark_unseen_unavailable(library, empty_scan)
            .await
            .unwrap(),
        1
    );
    state.db.finish_scan_job(empty_scan, 1).await.unwrap();
    let updated_playlist = state
        .services
        .update_playlist(
            owner,
            aggregate_playlist.id,
            Some("Unavailable aggregate renamed"),
            None,
            None,
            &[],
            &[],
            Default::default(),
        )
        .await
        .unwrap();
    assert_eq!(updated_playlist.name, "Unavailable aggregate renamed");
    assert!(updated_playlist.songs.is_empty());
    let persisted_playlist_tracks: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM playlist_track WHERE playlist_id=?")
            .bind(aggregate_playlist.id.to_string())
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert_eq!(persisted_playlist_tracks, 1);
    let unavailable_queue = state.services.queue(owner).await.unwrap().unwrap();
    assert!(unavailable_queue.songs.is_empty());
    // The saved row still names a current track. The projection must not, once
    // that track is no longer among the songs it hands back.
    assert!(unavailable_queue.current.is_none());
    assert!(state
        .services
        .shares(owner)
        .await
        .unwrap()
        .into_iter()
        .find(|share| share.id == aggregate_share.id)
        .unwrap()
        .songs
        .is_empty());
    // A visitor sees the same thing as the owner: the share survives its last
    // track going unavailable, rather than answering not-found after the visit
    // has already been counted.
    let visited = state
        .services
        .public_share(
            aggregate_share
                .url_token
                .as_deref()
                .expect("a freshly created share carries its token"),
        )
        .await
        .expect("a share outlives a track that went unavailable");
    assert!(visited.songs.is_empty());
    state.services.sync_snapshot(owner, 100).await.unwrap();
}
