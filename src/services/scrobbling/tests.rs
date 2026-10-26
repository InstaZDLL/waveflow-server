use super::links::link_health;
use super::retry::{retry_base, retry_spread};
use super::*;

/// The wire name, the database name, `as_str` and `FromStr` are one fact.
///
/// They were four separate facts until a review noticed that
/// `rename_all = "snake_case"` serialised `ListenBrainz` as
/// `listen_brainz`: a link list would have published a destination that
/// `FromStr` refuses and the `CHECK` constraint has never heard of, so a
/// client could read back a value it could not send in. Nothing had noticed
/// because the surface that serialises it is not written yet — which is
/// precisely when this costs nothing to hold still.
#[test]
fn the_four_spellings_of_a_destination_agree() {
    for provider in [
        ScrobbleProvider::ListenBrainz,
        ScrobbleProvider::Maloja,
        ScrobbleProvider::LastFm,
    ] {
        let name = provider.as_str();
        assert_eq!(
            serde_json::to_string(&provider).unwrap(),
            format!("\"{name}\""),
            "what the API publishes must be what the database stores"
        );
        assert_eq!(ScrobbleProvider::from_str(name).unwrap(), provider);
    }
}

#[test]
fn a_link_that_answers_is_still_degraded_while_its_queue_does_not_move() {
    let hour = Duration::from_secs(3600);
    let now = 10_000_000;

    // Nothing waiting, nothing ambiguous.
    assert_eq!(link_health("active", 0, None, now, hour), "healthy");

    // Waiting, but not for long.
    assert_eq!(
        link_health("active", 0, Some(now - 60_000), now, hour),
        "healthy"
    );

    // This is the silent failure a durable queue exists to make visible: the
    // token is good, the destination answers, and nothing has moved for
    // hours.
    assert_eq!(
        link_health("active", 0, Some(now - 6 * 3_600_000), now, hour),
        "degraded"
    );

    // One ambiguous entry is enough: it is waiting on a person, not on a
    // network, and nothing will move it on its own.
    assert_eq!(link_health("active", 1, None, now, hour), "degraded");

    // A broken authorisation outranks both.
    assert_eq!(
        link_health("broken", 3, Some(now - 6 * 3_600_000), now, hour),
        "broken"
    );
}

/// The ordinary wait, composed the way the drain composes it.
///
/// The two halves are separate in production so that a caller with a floor
/// of its own — a destination that asked for room — can take the larger of
/// the two *bases* and still get a spread on the result. A test that wants
/// the plain wait therefore has to put them back together rather than call
/// a third function that exists only for it to call.
fn wait(attempts: i64, entry_id: i64) -> i64 {
    let base = retry_base(attempts);
    base.saturating_add(retry_spread(base, entry_id))
}

/// Neighbouring rows must land *far* apart, not merely at different
/// milliseconds.
///
/// This is the property an integration test could not hold. Rowids are
/// consecutive, so the unmixed spread put four rows at 1, 2, 3 and 4
/// milliseconds inside a fifteen-minute window — distinct, and a herd all
/// the same. `assert_ne!` tells "absent" from "present" and never "present
/// and useless", so it certified a property the code did not deliver until
/// a review read the arithmetic.
///
/// A pure function, tested purely: dragging the drain, a database and an
/// HTTP server through this proved less and cost more.
#[test]
fn the_spread_scatters_neighbouring_rows_across_the_interval() {
    let base = 3_600_000;
    let mut offsets: Vec<i64> = (1..=4).map(|id| retry_spread(base, id)).collect();
    for offset in &offsets {
        assert!(
            (0..base / 4).contains(offset),
            "the spread never leaves its quarter of the wait"
        );
    }
    offsets.sort_unstable();
    let smallest = offsets
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .min()
        .expect("four offsets have three gaps");
    assert!(
        smallest >= 1_000,
        "neighbouring rows land {smallest}ms apart, which is still a herd"
    );
}

/// The attempt cap and the schedule beside it describe one span of time, so
/// what the cap is *for* is checked here rather than asserted in prose.
///
/// A review found the comment on that constant claiming "most of a day"
/// above a number that delivered two hours and three minutes. The sentence
/// was the honest statement of the intention, so the number moved to meet
/// it — and this exists so the two cannot drift apart again, which prose
/// alone has already failed to prevent once.
#[test]
fn the_default_attempt_cap_carries_a_listen_across_a_day_of_outage() {
    let cap = i64::from(crate::config::DEFAULT_SCROBBLE_MAX_ATTEMPTS);
    // One submission per attempt, and one wait between each pair of them,
    // so a cap of `n` spends `n - 1` waits. Without jitter: the spread only
    // ever adds.
    let covered: i64 = (1..cap).map(|attempt| wait(attempt, 0)).sum();
    let hours = covered / 3_600_000;
    assert!(
        (23..=26).contains(&hours),
        "the default cap covers {hours}h of outage, which is not the day its comment claims"
    );
}

#[test]
fn the_wait_grows_to_an_hour_and_two_rows_never_return_together() {
    let minute = 60_000;
    // The first retry waits about a minute, the later ones about an hour,
    // and never more.
    assert!((minute..minute + minute / 4).contains(&wait(1, 0)));
    assert!(wait(40, 0) <= 3_600_000 + 3_600_000 / 4);
    assert!(wait(40, 0) >= 3_600_000);
    // Monotonic while it grows.
    assert!(wait(3, 0) > wait(1, 0));
    // And two entries that failed in the same instant come back apart.
    assert_ne!(wait(2, 7), wait(2, 8));
}
