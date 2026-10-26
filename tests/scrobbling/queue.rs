//! What is queued, what never is, and what each verdict and an unlink do to the
//! queue.

use super::*;

/// The one envelope waiting, read back from the queue.
async fn queued_envelope(state: &AppState) -> ScrobbleEnvelope {
    let row = sqlx::query_as::<
        _,
        (
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
        ),
    >(
        "SELECT played_at, title, artists_json, album, album_artist, duration_ms \
         FROM scrobble_outbox ORDER BY id LIMIT 1",
    )
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    ScrobbleEnvelope {
        played_at: row.0,
        title: row.1,
        artists: serde_json::from_str(&row.2).unwrap(),
        album: row.3,
        album_artist: row.4,
        duration_ms: row.5,
        musicbrainz_recording_id: None,
    }
}

#[tokio::test]
async fn an_account_that_linked_nothing_queues_nothing() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "unlinked-listener").await;

    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    // The listen is recorded, as it always was.
    assert_eq!(
        state
            .services
            .history(fixture.owner, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    // And nothing left the server. This is the ordinary installation: a server
    // that was merely upgraded makes no outbound request at all.
    assert!(rows(&state).await.is_empty());
}

#[tokio::test]
async fn a_listen_is_queued_with_what_was_heard_and_a_now_playing_is_not() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "queueing-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-token",
        )
        .await
        .unwrap();

    // A now-playing has no value once it stops being true, so it is never
    // queued — decision 3.
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, false, None)
        .await
        .unwrap();
    assert!(
        rows(&state).await.is_empty(),
        "a now-playing must never be queued for delivery"
    );

    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, Some(1_700_000_000_000))
        .await
        .unwrap();

    let queued = rows(&state).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].1, "pending");
    assert_eq!(queued[0].2, 0);

    // The envelope is the listen as it was heard, not a pointer to the track.
    let envelope = queued_envelope(&state).await;
    assert_eq!(envelope.played_at, 1_700_000_000_000);
    assert_eq!(envelope.title, "Matrix flac");
    assert_eq!(
        envelope.artists,
        vec!["Alpha".to_owned(), "Beta".to_owned()]
    );
    assert_eq!(envelope.album.as_deref(), Some("WaveFlow format matrix"));
    assert_eq!(envelope.album_artist.as_deref(), Some("Matrix Artist"));
}

#[tokio::test]
async fn a_correction_after_the_listen_does_not_change_what_was_queued() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "retagging-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-token",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    // The member corrects the track afterwards, which is what #186 made
    // possible and what decision 2 was rewritten for: a correction rewrites
    // `track_participant`, so reading the track at drain time would submit what
    // it has become rather than what was played.
    state
        .services
        .set_track_metadata(
            fixture.owner,
            fixture.tagged,
            waveflow_server::services::TrackMetadataPatch {
                title: Some(Some("Corrected afterwards".to_owned())),
                artists: Some(Some(vec!["Somebody Else".to_owned()])),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

    let envelope = queued_envelope(&state).await;
    assert_eq!(
        envelope.title, "Matrix flac",
        "the envelope records the listen, and the listen already happened"
    );
    assert_eq!(
        envelope.artists,
        vec!["Alpha".to_owned(), "Beta".to_owned()]
    );
}

#[tokio::test]
async fn a_track_the_server_cannot_name_is_never_queued() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "untagged-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-token",
        )
        .await
        .unwrap();

    state
        .services
        .scrobble(fixture.owner, fixture.bare, true, None)
        .await
        .unwrap();
    assert!(
        rows(&state).await.is_empty(),
        "a submission with no credited artist is unusable at the far end"
    );

    // And the same account, same link, same call, on a track that can be named.
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    assert_eq!(rows(&state).await.len(), 1);
}

#[tokio::test]
async fn an_accepted_listen_leaves_the_queue_and_the_secret_arrives_intact() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "accepting-listener").await;
    // Last.fm, because the first half of this test needs a destination this
    // process has no adapter for, and a declared instance with no application
    // credentials is exactly that.
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::LastFm,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    // Before any adapter is registered, the pass leaves the row exactly as it
    // is. A server missing an adapter is a misconfiguration, and spending the
    // listen's retries on it would destroy the queue the operator is about to
    // repair.
    let unserviced = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(unserviced.unserviced, 1);
    assert_eq!(rows(&state).await[0].2, 0, "no attempt may have been spent");
    // It does step aside, though, so that a destination which *can* be reached
    // is never stuck behind it. Bringing it forward here is what a restart does
    // once the operator has supplied the missing adapter.
    make_everything_due(&state).await;

    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::LastFm,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.accepted, 1);
    assert_eq!(rows(&state).await[0].1, "sent");

    // The secret was sealed under the instance key on the way in and came back
    // out whole.
    let seen = target.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].1, "lb-secret");
    assert_eq!(seen[0].0.title, "Matrix flac");

    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].health, "healthy");
    assert_eq!(links[0].pending, 0);
    assert!(links[0].last_success_at.is_some());

    // A second pass has nothing left to do, which is what "exactly one queueing"
    // looks like from the drain's side.
    assert_eq!(
        state.services.drain_scrobble_outbox().await.unwrap(),
        Default::default()
    );
    assert_eq!(target.seen().len(), 1);
}

#[tokio::test]
async fn an_ambiguous_answer_is_terminal_and_waits_for_a_person() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "ambiguous-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    let target = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.uncertain, 1);
    assert_eq!(rows(&state).await[0].1, "uncertain");

    // No automatic retry, ever. The destination may already hold this listen,
    // and a duplicate in a public history is worse than a gap — decision 5.
    make_everything_due(&state).await;
    assert_eq!(
        state.services.drain_scrobble_outbox().await.unwrap(),
        Default::default()
    );
    assert_eq!(
        target.seen().len(),
        1,
        "an ambiguous result must never be submitted again on the server's own initiative"
    );

    // It is counted, and it is what makes the link degraded: nothing will move
    // it without a person.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].uncertain, 1);
    assert_eq!(links[0].health, "degraded");
}

#[tokio::test]
async fn a_retryable_answer_comes_back_until_the_attempts_run_out() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "retrying-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    let target = Recorder::always(ScrobbleVerdict::Retryable { after: None });
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    // The first failure schedules a later attempt rather than a immediate one.
    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.retrying, 1);
    assert_eq!(rows(&state).await[0].2, 1);
    assert_eq!(
        state.services.drain_scrobble_outbox().await.unwrap(),
        Default::default(),
        "the row is not due again yet"
    );

    // `for_data_dir` allows three attempts, so the third is the last.
    make_everything_due(&state).await;
    assert_eq!(
        state
            .services
            .drain_scrobble_outbox()
            .await
            .unwrap()
            .retrying,
        1
    );
    make_everything_due(&state).await;
    let last = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(last.abandoned, 1);
    let rows = rows(&state).await;
    assert_eq!(rows[0].1, "abandoned");
    assert_eq!(rows[0].2, 3);
    assert_eq!(
        target.seen().len(),
        3,
        "a queue that never empties is a fault, not a state"
    );
}

#[tokio::test]
async fn a_broken_authorisation_finishes_the_queue_rather_than_asking_again() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "broken-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    for _ in 0..3 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }
    assert_eq!(rows(&state).await.len(), 3);

    let target = Recorder::always(ScrobbleVerdict::AuthBroken);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.broken, 1);
    for row in rows(&state).await {
        assert_eq!(
            row.1, "cancelled",
            "every waiting row would fail identically"
        );
    }
    // And the other two were never submitted. The batch was chosen before the
    // first verdict came back; asking a refused question twice more is exactly
    // what `AuthBroken` has already settled.
    assert_eq!(target.seen().len(), 1);

    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "broken");
    assert_eq!(links[0].last_failure.as_deref(), Some("auth_broken"));
}

#[tokio::test]
async fn unlinking_leaves_its_queue_behind_and_a_new_link_does_not_inherit_it() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "relinking-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "first-account",
        )
        .await
        .unwrap();
    for _ in 0..2 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    assert!(state
        .services
        .unlink_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "default")
        .await
        .unwrap());
    // Cancelled rather than deleted: they record listens that really happened
    // and really were never sent, and a queue that erases its own losses cannot
    // be asked what it lost.
    for row in rows(&state).await {
        assert_eq!(row.1, "cancelled");
    }
    assert!(state
        .services
        .scrobble_links(fixture.owner)
        .await
        .unwrap()
        .is_empty());

    // A different account at the same destination. This is the whole reason the
    // queue references a generation and not the pair `(account, destination)`:
    // the two listens above must not land on this profile.
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "second-account",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.accepted, 1);
    let seen = target.seen();
    assert_eq!(
        seen.len(),
        1,
        "the listens queued under the first authorisation must not reach the second"
    );
    assert_eq!(seen[0].1, "second-account");
}
