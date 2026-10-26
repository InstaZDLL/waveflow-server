//! The ListenBrainz adapter on the wire: the shape of a listen, rate limits,
//! ceilings and refused tokens.

use super::*;

/// Points ListenBrainz's declared instance at `base`.
///
/// Replaces the unreachable loopback `Config::for_data_dir` declares for that
/// recipient rather than adding beside it — two instances cannot share a name —
/// and leaves the other recipients alone, because more than one test needs a
/// second destination in the same queue.
fn reaching(config: &mut waveflow_server::Config, base: &str) {
    let url = waveflow_server::scrobblers::validate_destination(base, true)
        .expect("a destination the suite chose");
    let fingerprint = waveflow_server::scrobblers::destination_fingerprint(&url);
    config
        .destinations
        .retain(|declared| declared.provider != ScrobbleProvider::ListenBrainz);
    config
        .destinations
        .push(waveflow_server::config::ScrobbleDestination {
            provider: ScrobbleProvider::ListenBrainz,
            name: "default".to_owned(),
            url,
            fingerprint,
        });
}

/// An app configured to reach that destination.
///
/// This goes through `initialize`, so it exercises the whole chain a real
/// server walks — the destination is validated, the client is built, the
/// adapter is registered — rather than registering a target by hand as the
/// tests above do.
async fn app_reaching(base: &str) -> (tempfile::TempDir, waveflow_server::Config, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    reaching(&mut config, base);
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

#[tokio::test]
async fn a_listen_reaches_the_destination_in_the_shape_it_documents() {
    let destination = spawn_destination(200, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "listenbrainz-listener").await;
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
        .scrobble(fixture.owner, fixture.tagged, true, Some(1_700_000_000_123))
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.accepted, 1);
    assert_eq!(rows(&state).await[0].1, "sent");

    // The scheme ListenBrainz documents, and not `Bearer`.
    let authorizations = destination.authorizations.lock().unwrap().clone();
    assert_eq!(authorizations, vec!["Token lb-secret".to_owned()]);

    let bodies = destination.bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let listen = &bodies[0]["payload"][0];
    assert_eq!(bodies[0]["listen_type"], "single");
    // **Seconds.** The envelope holds epoch milliseconds like everything else
    // here, and sending those unconverted would date this listen some fifty
    // thousand years out — in a history that keeps it.
    assert_eq!(listen["listened_at"], 1_700_000_000);
    assert_eq!(listen["track_metadata"]["track_name"], "Matrix flac");
    assert_eq!(listen["track_metadata"]["artist_name"], "Alpha, Beta");
}

#[tokio::test]
async fn a_rate_limited_listen_waits_at_least_as_long_as_it_was_asked() {
    // An hour, against a first backoff of one minute, so only the destination's
    // own answer can explain the wait.
    let destination = spawn_destination(429, Some(3_600)).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "rate-limited-listener").await;
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
    let before = now_ms();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.retrying, 1);

    // Honouring a request to slow down means not returning before it says, so
    // the wait is the longer of the two and never the destination's in place of
    // ours. Decision 6 promised this and PR #191 had nowhere to carry it.
    let next: i64 =
        sqlx::query_scalar("SELECT next_attempt_at FROM scrobble_outbox ORDER BY id LIMIT 1")
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(
        next >= before + 3_600_000,
        "the wait must honour the destination's own answer, not our shorter backoff"
    );

    // And the link says *why* it is waiting, in the normalised vocabulary
    // decision 12 describes. `rate_limited` was documented there while nothing
    // wrote it, and then written to the wrong table — the outbox row rather
    // than the link that `ScrobbleLinkState` actually reads.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].last_failure.as_deref(), Some("rate_limited"));
}

#[tokio::test]
async fn a_rate_limit_named_only_by_the_standard_header_is_honoured_too() {
    // No vendor header at all — this is the `429` a proxy in front of a
    // self-hosted instance answers with, and decision 6 names `Retry-After` as
    // a delay to honour.
    //
    // The test above cannot tell the two readings apart: it sends
    // `x-ratelimit-reset-in`, so it passes whether or not the adapter ever
    // learned the standard header. For a while it had not — the function that
    // read it was written and never called, and only a dead-code warning said
    // so. This test is what would have said so instead.
    // Half an hour, not a whole one: `RETRY_CEILING_MS` is exactly an hour, so
    // asking for 3600 sits the expected value on the clamp boundary and passes
    // only by the margin the clock happens to add. Half of it tests the same
    // reading with nothing resting on where the ceiling falls.
    let destination = spawn_delaying(429, Some(("retry-after", "1800".to_owned()))).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "proxied-listener").await;
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
    let before = now_ms();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.retrying, 1);

    // Half an hour, against a first backoff of one minute: only the header can
    // explain a wait this long, so reading it is the only way to pass.
    let next: i64 =
        sqlx::query_scalar("SELECT next_attempt_at FROM scrobble_outbox ORDER BY id LIMIT 1")
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(
        next >= before + 1_800_000,
        "a `Retry-After` of half an hour must outlast our own one-minute backoff"
    );
}

#[tokio::test]
async fn a_token_that_cannot_be_a_header_is_refused_when_it_is_pasted() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "pasting-listener").await;

    // A newline is what a copy out of a web page hands you, and a secret
    // carrying one can never be spelled as an HTTP header — so it will never
    // work against any destination. Refused at the only moment the person can
    // fix it, rather than discovered hours later on a background drain.
    assert!(state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb\nsecret"
        )
        .await
        .is_err());
    assert!(
        state
            .services
            .scrobble_links(fixture.owner)
            .await
            .unwrap()
            .is_empty(),
        "a refused token must leave no link behind to queue listens under"
    );

    // And the check is not so eager that it refuses an ordinary one.
    assert!(state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret"
        )
        .await
        .is_ok());
}

#[tokio::test]
async fn a_credential_sealed_before_that_check_breaks_the_link_rather_than_going_uncertain() {
    let destination = spawn_destination(200, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "sealed-badly-listener").await;

    // Sealed and inserted by hand, because `link_scrobble` now refuses this
    // through the front door — and rows sealed before it did are never
    // revalidated. This is the case the adapter's own guard exists for, and
    // without this test that claim would be prose.
    let sealed = state.secret_box.encrypt(b"lb\nsecret").unwrap();
    let now = now_ms();
    // The instance is named here as `link_scrobble` would name it: the drain
    // picks an adapter by `(provider, destination)`, so a row without one is
    // not the case this test is about — it is a row nothing can carry at all.
    let declared = config
        .destinations
        .iter()
        .find(|declared| declared.provider == ScrobbleProvider::ListenBrainz)
        .expect("the suite declares one");
    sqlx::query(
        "INSERT INTO scrobble_link (id, user_id, provider, destination, \
         destination_fingerprint, status, credential_nonce, credential_ciphertext, \
         created_at, updated_at) \
         VALUES (?, ?, 'listenbrainz', ?, ?, 'active', ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(fixture.owner.to_string())
    .bind(declared.name.clone())
    .bind(declared.fingerprint.clone())
    .bind(sealed.nonce.as_slice())
    .bind(sealed.ciphertext.as_slice())
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    // A stored secret that cannot be spelled as a header is a broken
    // authorisation, and is named one.
    assert_eq!(drained.broken, 1);
    assert_eq!(
        drained.uncertain, 0,
        "bytes that never left must not become the one verdict a person cannot undo"
    );
    assert_eq!(rows(&state).await[0].1, "cancelled");
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "broken");

    // And nothing was sent, because there was never a request to send.
    assert_eq!(destination.bodies.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn a_destination_cannot_park_a_listen_past_our_own_ceiling() {
    // About thirty-one thousand years, which is the shape a hostile or merely
    // broken destination takes. `after` is the only value in the whole
    // scheduling path a third party chooses.
    let destination = spawn_destination(429, Some(999_999_999_999)).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "absurd-wait-listener").await;
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
    let before = now_ms();
    // **Two**, and for the same reason the drain-interval test needed two: the
    // resting map is only consulted for a *second* row on the same link, so
    // with one listen the clamped value is computed, stored, and never read. A
    // review caught that this test proved the floor and not the ceiling — the
    // inert half surviving one screen above the place it had just been fixed.
    for _ in 0..2 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    state.services.drain_scrobble_outbox().await.unwrap();

    let next: i64 =
        sqlx::query_scalar("SELECT next_attempt_at FROM scrobble_outbox ORDER BY id LIMIT 1")
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    // Clamped to the hour the backoff already calls the point past which
    // waiting buys nothing, plus at most its own quarter of spread. Unclamped,
    // this row would sit at a moment it never reaches — a third party deciding
    // the listen dies, which is decision 10's concern arriving from the side
    // nobody watches.
    assert!(
        next <= before + 3_600_000 + 900_000 + 10_000,
        "a destination must not be able to schedule a listen beyond our own ceiling"
    );
    // And still a real wait: clamping is not ignoring.
    assert!(next >= before + 3_600_000);

    // The row behind it carries the clamp itself, exactly. `defer_scrobble`
    // binds `next_attempt_at` and `updated_at` from one `now`, so this
    // difference is the deferral and nothing else — an hour, where the
    // destination asked for thirty-one thousand years.
    let waits: Vec<i64> =
        sqlx::query_scalar("SELECT next_attempt_at - updated_at FROM scrobble_outbox ORDER BY id")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert_eq!(waits.len(), 2);
    assert_eq!(
        waits[1], 3_600_000,
        "the ceiling must bound what a destination can ask the queue to wait"
    );
}

// The herd this file used to test for lives in
// `services::scrobbling::tests::the_spread_scatters_neighbouring_rows_across_the_interval`
// now. Two of the fixes in this branch turned out to contradict each other
// here: a rate-limited link now rests for the rest of the pass, so only one row
// is ever offered, and a test that needed four waits in one pass could no
// longer get them. The property is a pure function's, and proving it through a
// drain, a database and an HTTP server proved less while costing more. What a
// rate limit does to the *batch* is `a_destination_that_asked_for_room_is_not_offered_the_rest_of_the_batch`;
// what it does to the *wait* is `a_rate_limited_listen_waits_at_least_as_long_as_it_was_asked`.

#[tokio::test]
async fn a_destination_that_asked_for_room_is_not_offered_the_rest_of_the_batch() {
    let destination = spawn_destination(429, Some(3_600)).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "resting-listener").await;
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

    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    // One submission, not four. The batch is chosen before the first verdict,
    // so without the link resting, being told to slow down would be answered by
    // sending the rest of it — each row earning its own refusal and spending
    // its own attempt, at a destination that had just asked for room.
    assert_eq!(destination.bodies.lock().unwrap().len(), 1);
    assert_eq!(drained.retrying, 1);

    // The one that was refused waits and has spent an attempt; the three behind
    // it were never offered, so they have spent nothing and come back next
    // pass.
    let rows = rows(&state).await;
    // Asserted before the loop below, which would otherwise pass vacuously if
    // the four calls ever stopped queueing four rows.
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].2, 1);
    for row in &rows[1..] {
        assert_eq!(row.1, "pending");
        assert_eq!(row.2, 0, "a row that was never offered has spent nothing");
    }
}

#[tokio::test]
async fn a_drain_interval_longer_than_the_ceiling_does_not_kill_the_drain() {
    let destination = spawn_destination(429, Some(3_600)).await;
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    reaching(&mut config, &destination.base);
    // Longer than `RETRY_CEILING`, which nothing forbids: `parse_positive_env`
    // bounds this below zero and nowhere above. That made the rest floor exceed
    // the rest ceiling, and `Ord::clamp` panics when its minimum exceeds its
    // maximum — inside a `tokio::spawn`ed loop, so the queue would have stopped
    // for the life of the process. The silent stop this whole RFC exists to
    // prevent, reachable by one plausible setting.
    config.scrobbling.drain_interval = Duration::from_secs(7_200);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let fixture = fixture(&config, &state, "slow-interval-listener").await;
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
    // **Two**, and that is the whole difference between this test and the one
    // it replaced. With a single listen the bounds are computed, written into
    // the resting map, and never read — the map is only consulted for a
    // *second* row on the same link — so the wait asserted below came from
    // `reschedule_scrobble`'s own ceiling and the assertion could not fail for
    // the reason its comment gave. A review caught that; the earlier version of
    // this file had the same shape twice before.
    for _ in 0..2 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    // The first assertion is that this returns at all rather than unwinding.
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.retrying, 1);
    assert_eq!(drained.rested, 1);

    let waits: Vec<i64> =
        sqlx::query_scalar("SELECT next_attempt_at - updated_at FROM scrobble_outbox ORDER BY id")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    // Asserted before either row is indexed, which is the guard this file
    // argues for twice elsewhere — indexing panics rather than passing
    // vacuously, but the inconsistency is worth closing where it is noticed.
    assert_eq!(waits.len(), 2);
    // The row that was offered waits by its own schedule, capped at the hour.
    assert!((3_600_000..=4_600_000).contains(&waits[0]));
    // The row behind it waits by the bounds under test — and this is the half
    // that needed a second listen to exist at all. The floor wins here, because
    // a pass every two hours means a one-hour deferral would have the row
    // offered again before the next pass could serve it.
    assert_eq!(waits[1], 7_200_000);
}

#[tokio::test]
async fn a_rate_limit_that_names_no_delay_still_rests_the_link() {
    // `429` carrying no `X-RateLimit-Reset-In` at all — a destination under
    // load, or one whose proxy stripped the header on the way back.
    //
    // The **status** is what says room was asked for; the header only says how
    // much. Passing the missing header through as `after: None` made this
    // indistinguishable from a connect failure, so the link rested for nothing
    // and the cause was recorded as an ordinary `retryable` — both of the
    // things that value carries, bypassed by a header being absent.
    let destination = spawn_destination(429, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "silent-limit-listener").await;
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

    state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(
        destination.bodies.lock().unwrap().len(),
        1,
        "a rate limit that named no delay must still stop the rest of the batch"
    );
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].last_failure.as_deref(), Some("rate_limited"));
}

#[tokio::test]
async fn a_rate_limited_backlog_does_not_starve_another_destination() {
    let listenbrainz = spawn_destination(429, Some(3_600)).await;
    // A batch of four, so the backlog below fills a whole pass on its own —
    // the shape a real server reaches the moment one destination is limited
    // and the other is not.
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    reaching(&mut config, &listenbrainz.base);
    config.scrobbling.batch = 4;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let fixture = fixture(&config, &state, "starved-listener").await;

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

    // A second destination, linked after the backlog exists and reachable.
    let maloja = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::Maloja,
        "default",
        std::sync::Arc::clone(&maloja) as std::sync::Arc<dyn ScrobbleTarget>,
    );
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

    // The first pass is filled by the rate-limited backlog: one row offered and
    // refused, the other three moved out of the way.
    let first = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(first.retrying, 1);
    assert_eq!(
        first.rested, 3,
        "a pass that moves rows aside must say so, or it looks like a pass that did nothing"
    );
    // The second must reach the destination that is perfectly willing.
    state.services.drain_scrobble_outbox().await.unwrap();

    // Skipping the rested rows rather than deferring them leaves them due with
    // an *older* `next_attempt_at`, so they keep sorting ahead of this one and
    // it is never reached — not slowly, never.
    assert_eq!(
        maloja.seen().len(),
        1,
        "a rate-limited backlog must not starve a destination that is answering"
    );
}

#[tokio::test]
async fn the_spread_reaches_the_rows_the_drain_actually_reschedules() {
    // A server fault rather than a rate limit, on purpose: a `500` earns
    // `Retryable { after: None }`, which does not rest the link — so all four
    // rows are rescheduled in one pass and their waits can only differ by the
    // spread.
    let destination = spawn_destination(500, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "scattered-listener").await;
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

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.retrying, 4);

    // This is the test the deleted integration one should have been. A review
    // pointed out that after that deletion, *nothing* failed if the spread
    // stopped being applied in production: the unit test proves the pure
    // function, and the helper beside it re-implements the composition rather
    // than calling the path the drain takes. Removing
    // `.saturating_add(retry_spread(..))` from `reschedule_scrobble` left the
    // whole suite green.
    let waits: Vec<i64> =
        sqlx::query_scalar("SELECT next_attempt_at - updated_at FROM scrobble_outbox ORDER BY id")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert_eq!(waits.len(), 4);
    let mut sorted = waits.clone();
    sorted.sort_unstable();
    let smallest_gap = sorted
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .min()
        .expect("four rows have three gaps");
    assert!(
        smallest_gap >= 1_000,
        "the drain rescheduled four rows {smallest_gap}ms apart, which is still a herd"
    );
}

#[tokio::test]
async fn a_destination_that_refuses_the_token_breaks_the_link() {
    let destination = spawn_destination(401, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "refused-token-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "stale-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.broken, 1);
    assert_eq!(rows(&state).await[0].1, "cancelled");
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "broken");
    assert_eq!(links[0].last_failure.as_deref(), Some("auth_broken"));
}
