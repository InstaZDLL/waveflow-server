//! The drain: destinations that stall, refuse, vanish mid-flight or answer too
//! late.

use super::*;

/// A destination that takes the submission and then simply never answers.
///
/// Not a refusal and not a network failure: the connection is good, the request
/// is gone, and nothing comes back. It is the shape a stalled far end really
/// has, and the one shape no verdict can describe.
struct Stalls {
    calls: Mutex<usize>,
}

impl ScrobbleTarget for Stalls {
    fn submit<'a>(
        &'a self,
        _envelope: &'a ScrobbleEnvelope,
        _secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async move {
            *self.calls.lock().unwrap() += 1;
            std::future::pending::<ScrobbleVerdict>().await
        })
    }
}

/// An app whose scrobbling limits this test chose.
///
/// Tuned before `initialize`, never after: `DomainServices` copies these values
/// out of `Config` when it is built, so changing one on the returned `Config`
/// would silently exercise the old one.
async fn tuned_app(
    tune: impl FnOnce(&mut ScrobbleLimits),
) -> (tempfile::TempDir, waveflow_server::Config, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    tune(&mut config.scrobbling);
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

#[tokio::test]
async fn a_destination_that_never_answers_does_not_hold_the_queue_forever() {
    let (_temp, config, state) =
        tuned_app(|limits| limits.request_timeout = Duration::from_millis(150)).await;
    let fixture = fixture(&config, &state, "stalling-listener").await;
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
    let target = std::sync::Arc::new(Stalls {
        calls: Mutex::new(0),
    });
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    // The first assertion is that this returns at all. The drain is a
    // background task, so without the deadline it is held for the life of the
    // process and nothing on the server ever says so — which is why the wait is
    // bounded here rather than inside whichever adapter happens to be
    // registered.
    let drained = tokio::time::timeout(
        Duration::from_secs(5),
        state.services.drain_scrobble_outbox(),
    )
    .await
    .expect("a destination that never answers must not hold the drain")
    .unwrap();

    assert_eq!(*target.calls.lock().unwrap(), 1);
    // Uncertain rather than retrying: the request left and the answer did not
    // come back, and those two are what decision 5 calls indistinguishable. The
    // destination may already hold this listen, so sending it again is a
    // decision that belongs to a person.
    assert_eq!(drained.uncertain, 1);
    assert_eq!(drained.retrying, 0);
    assert_eq!(rows(&state).await[0].1, "uncertain");

    // And the link says what happened, without echoing anything of the listen.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "degraded");
    assert_eq!(links[0].last_failure.as_deref(), Some("ambiguous"));
}

#[tokio::test]
async fn a_destination_with_no_adapter_does_not_starve_one_that_has_it() {
    // A batch of one, so the row nothing can carry fills a whole pass on its
    // own. That is the smallest arrangement in which the question can be asked
    // at all, and the shape a real server reaches when one destination has been
    // queueing for a while.
    let (_temp, config, state) = tuned_app(|limits| limits.batch = 1).await;
    let fixture = fixture(&config, &state, "starving-listener").await;
    // Last.fm is the one this process cannot reach: the suite's configuration
    // declares an instance for it and no application credentials, so no adapter
    // is built for it. Declared and adapterless is a shape a real deployment
    // has, which is why the test stands on it rather than on a recipient that
    // merely has no adapter yet.
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::LastFm,
            "default",
            "lastfm-secret",
        )
        .await
        .unwrap();
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::Maloja,
            "default",
            "maloja-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    // One listen, one row per authorisation, and the Last.fm row sorts first —
    // which is what puts the unreachable destination at the head.
    assert_eq!(rows(&state).await.len(), 2);

    // Only one of the two can be reached by this process. That is the ordinary
    // state of affairs while adapters are being added one at a time.
    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::Maloja,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let first = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(first.unserviced, 1);
    assert_eq!(first.accepted, 0);

    // The second pass must reach the other destination. Without the row
    // stepping aside, the same unserviceable one is fetched again every time
    // and the Maloja listen is never sent — not late, never: nothing advances
    // a row no adapter can carry.
    let second = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(second.accepted, 1);
    assert_eq!(target.seen().len(), 1);
    assert_eq!(target.seen()[0].1, "maloja-secret");
}

#[tokio::test]
async fn a_refused_listen_is_not_offered_to_the_destination_again() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "refused-listener").await;
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
    let target = Recorder::always(ScrobbleVerdict::PermanentReject);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.rejected, 1);
    assert_eq!(rows(&state).await[0].1, "rejected");

    // No retry can fix a payload the far end has read and will not have, so
    // this is terminal — and unlike `uncertain` it asks nothing of anybody.
    make_everything_due(&state).await;
    assert_eq!(
        state.services.drain_scrobble_outbox().await.unwrap(),
        Default::default()
    );
    assert_eq!(target.seen().len(), 1);

    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].last_failure.as_deref(), Some("rejected"));
    assert_eq!(links[0].pending, 0);
    assert_eq!(links[0].uncertain, 0);
}

#[tokio::test]
async fn a_listen_interrupted_mid_flight_is_uncertain_rather_than_sent_again() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "interrupted-listener").await;
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

    // What a process that died between emitting a submission and recording its
    // verdict leaves behind: a row claimed long ago and never settled. Written
    // directly because the only other way to produce it is to kill the server.
    sqlx::query("UPDATE scrobble_outbox SET state='sending', updated_at=?")
        .bind(now_ms() - 600_000)
        .execute(state.db.pool())
        .await
        .unwrap();

    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.recovered, 1);
    // Uncertain, because nobody knows whether the destination took it. Returned
    // to the queue instead, it would have been sent a second time — a duplicate
    // arriving with no person having chosen it, which is the one choice this
    // server never makes.
    assert_eq!(rows(&state).await[0].1, "uncertain");
    assert_eq!(
        target.seen().len(),
        0,
        "an interrupted submission must not be re-emitted"
    );
    // And it is counted where a person will be asked to decide about it.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].uncertain, 1);
    assert_eq!(links[0].health, "degraded");
}

#[tokio::test]
async fn a_silent_destination_costs_one_entry_a_pass_and_not_the_whole_queue() {
    let (_temp, config, state) =
        tuned_app(|limits| limits.request_timeout = Duration::from_millis(150)).await;
    let fixture = fixture(&config, &state, "silent-listener").await;
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
    for _ in 0..4 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }
    assert_eq!(rows(&state).await.len(), 4);

    let target = std::sync::Arc::new(Stalls {
        calls: Mutex::new(0),
    });
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    // One deadline spent, not four. The cost that matters is not the twenty
    // seconds saved here: it is that each `uncertain` row is a decision
    // decision 13 lets a person make exactly once, so a single outage would
    // otherwise turn into a heap of irreversible manual choices.
    assert_eq!(*target.calls.lock().unwrap(), 1);
    assert_eq!(drained.uncertain, 1);

    // The one that really was sent is uncertain, because nobody knows whether
    // it arrived. The three behind it were never emitted, so they are untouched
    // and come back next pass.
    let rows = rows(&state).await;
    assert_eq!(rows[0].1, "uncertain");
    for row in &rows[1..] {
        assert_eq!(row.1, "pending");
        assert_eq!(row.2, 0, "an entry that was never sent has spent nothing");
    }
}

/// A destination that answers perfectly well — after something else has
/// already settled the row out from under this pass.
///
/// That is what a concurrent recovery leaves behind, reproduced deterministically
/// instead of by racing a sixty-second margin against the writer gate.
struct SettlesBehindYourBack {
    pool: sqlx::SqlitePool,
}

impl ScrobbleTarget for SettlesBehindYourBack {
    fn submit<'a>(
        &'a self,
        _envelope: &'a ScrobbleEnvelope,
        _secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE scrobble_outbox SET state='uncertain', last_failure='interrupted' \
                 WHERE state='sending'",
            )
            .execute(&self.pool)
            .await
            .unwrap();
            ScrobbleVerdict::Accepted
        })
    }
}

#[tokio::test]
async fn a_verdict_that_arrives_after_its_row_was_settled_is_not_counted() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "late-verdict-listener").await;
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
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::new(SettlesBehindYourBack {
            pool: state.db.pool().clone(),
        }) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    // Where the row ended up is defensible — nobody knows whether the
    // destination took it. What must not happen is the report carrying on as
    // though this pass had written something.
    assert_eq!(
        drained.accepted, 0,
        "a verdict about a row this pass no longer owns is not an acceptance"
    );
    assert_eq!(rows(&state).await[0].1, "uncertain");

    // And above all: no success stamped for a submission the queue cannot
    // claim. `healthy` must never come from a write that did not happen.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert!(
        links[0].last_success_at.is_none(),
        "last_success_at must not record a success the row never accepted"
    );
    assert_eq!(links[0].uncertain, 1);
}
