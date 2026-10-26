use super::*;

/// The secret does not survive being formatted.
///
/// The failure message deliberately carries neither the secret nor the
/// formatted output: if this guard were removed, that output would *be* the
/// token, and a panic message is a log line. The previous slice of this RFC
/// earned a CodeQL `rust/cleartext-logging` alert for exactly that, on a
/// test whose subject was also non-disclosure.
#[test]
fn a_link_request_does_not_print_its_secret() {
    let request = LinkScrobbleRequest {
        secret: "a-token-nobody-should-read".to_owned(),
    };
    let shown = format!("{request:?}");
    assert!(
        !shown.contains("a-token-nobody-should-read"),
        "the debug output carried the secret"
    );
    assert!(shown.contains("[redacted]"), "the secret was not replaced");
}
