//! Retention of finished rows, and the instant each way of finishing a row
//! records.

use super::*;

/// A day in milliseconds, so the retention tests read as the dates they mean.
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// Retention takes what is finished, and never what is still asking.
///
/// The queue kept everything: one row per listen and per destination, for ever,
/// on a server whose whole point is that somebody listens to music on it. The
/// bound is thirty days by default, and it is counted from the instant a row
/// became terminal — not from when it was queued, which would expire a listen
/// that took three weeks to be abandoned three weeks early.
///
/// **The three exemptions are the assertion.** `pending` has not finished,
/// `sending` is in flight, and `uncertain` is waiting for a person — taking
/// that one away after a month would answer decision 13's question in their
/// place.
#[tokio::test]
async fn retention_takes_what_is_finished_and_never_what_is_still_asking() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "retention-listener").await;
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

    // One accepted, so it is `sent` and finished.
    let accepting = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&accepting) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();

    // One ambiguous, so it is `uncertain` and still asking.
    let ambiguous = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&ambiguous) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();

    // And one never drained, so it is `pending`.
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let states: Vec<String> = rows(&state).await.into_iter().map(|row| row.1).collect();
    assert_eq!(states, ["sent", "uncertain", "pending"]);

    // A day short of the bound takes nothing: the bound is on the finished
    // row's own instant, and that row finished today.
    let now = now_ms();
    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now + 29 * DAY_MS)
            .await
            .unwrap(),
        0,
        "nothing is old enough yet"
    );
    assert_eq!(rows(&state).await.len(), 3);

    // A day past it takes the finished one, and only that one.
    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now + 31 * DAY_MS)
            .await
            .unwrap(),
        1,
        "the finished entry is past the bound"
    );
    let states: Vec<String> = rows(&state).await.into_iter().map(|row| row.1).collect();
    assert_eq!(
        states,
        ["uncertain", "pending"],
        "a listen still asking, and one not yet tried, are not the purge's business"
    );

    // And what the link publishes did not move, because none of it is read
    // from a row a purge can take.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].pending, 1);
    assert_eq!(links[0].uncertain, 1);
    assert!(links[0].last_success_at.is_some(), "a column of the link");
}

/// A discarded entry carries the instant it was discarded.
///
/// One of four statements that finish a row, and each is checked on its own:
/// `settle_scrobble` above, this, unlinking, and a refused token. The writer
/// that forgot the instant would leave rows no purge could ever see — the same
/// shape of silent survival `retried_at` exists to close — and a single test
/// driving all four would stop at the first.
#[tokio::test]
async fn a_discarded_entry_records_when_it_was_discarded() {
    let (_temp, config, state) = test_app().await;
    let listener = fixture(&config, &state, "discarding-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let uncertain_id = one_uncertain_entry(&state, &listener).await;
    state
        .services
        .discard_uncertain_scrobble(listener.owner, uncertain_id)
        .await
        .unwrap();

    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now_ms() + 31 * DAY_MS)
            .await
            .unwrap(),
        1
    );
    assert!(rows(&state).await.is_empty());
}

/// A queue cancelled by unlinking carries the instant it was cancelled.
#[tokio::test]
async fn an_unlinked_queue_records_when_it_was_cancelled() {
    let (_temp, config, state) = test_app().await;
    let listener = fixture(&config, &state, "unlinking-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    state
        .services
        .unlink_scrobble(listener.owner, ScrobbleProvider::ListenBrainz, "default")
        .await
        .unwrap();
    assert_eq!(rows(&state).await[0].1, "cancelled");

    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now_ms() + 31 * DAY_MS)
            .await
            .unwrap(),
        1
    );
}

/// And so does a queue a refused token finished.
///
/// The fourth writer, and the one furthest from the other three: it runs inside
/// `mark_link_broken` rather than beside a person's gesture. A link whose queue
/// stayed unpurgeable would keep growing for as long as the operator left the
/// bad token in place.
#[tokio::test]
async fn a_queue_a_refused_token_finished_records_when_it_finished() {
    let (_temp, config, state) = test_app().await;
    let listener = fixture(&config, &state, "broken-link-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let refusing = Recorder::always(ScrobbleVerdict::AuthBroken);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&refusing) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();
    let states: Vec<String> = rows(&state).await.into_iter().map(|row| row.1).collect();
    assert_eq!(states.len(), 2);
    assert!(
        states.iter().all(|state| state == "cancelled"),
        "a broken authorisation finishes the whole queue: {states:?}"
    );

    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now_ms() + 31 * DAY_MS)
            .await
            .unwrap(),
        2,
        "every row it finished carries when it finished"
    );
}
