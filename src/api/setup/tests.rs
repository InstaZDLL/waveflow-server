use super::*;
use std::time::Duration;

#[test]
fn the_wait_is_five_seconds_under_the_default_deadline() {
    let wait = first_library_wait(Duration::from_secs(30), Duration::from_millis(400));
    assert_eq!(wait, FIRST_LIBRARY_WAIT);
}

#[test]
fn a_short_deadline_keeps_the_wait_inside_it() {
    // A five-second deadline, one second spent creating the administrator:
    // a five-second wait would answer after the deadline, as a timeout.
    let wait = first_library_wait(Duration::from_secs(5), Duration::from_secs(1));
    assert_eq!(wait, Duration::from_secs(2));
}

#[test]
fn a_deadline_already_spent_waits_for_nothing() {
    let wait = first_library_wait(Duration::from_secs(2), Duration::from_secs(3));
    assert_eq!(wait, Duration::ZERO);
}
