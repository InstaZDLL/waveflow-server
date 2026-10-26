use super::*;

fn an_envelope() -> ScrobbleEnvelope {
    ScrobbleEnvelope {
        played_at: 1_700_000_123_456,
        title: "Zenith".to_owned(),
        artists: vec!["Nova Kern".to_owned(), "Juno Vale".to_owned()],
        album: Some("Convergence".to_owned()),
        album_artist: Some("Nova Kern".to_owned()),
        duration_ms: Some(245_678),
        musicbrainz_recording_id: Some("d2b9c1f0-0000-4000-8000-000000000001".to_owned()),
    }
}

/// Every credit arrives as a credit.
///
/// This is the whole reason decision 11 puts Maloja second. A body that
/// joined the artists would work, and would quietly turn a duo into a band
/// nobody has heard of in somebody's statistics.
#[test]
fn the_envelope_pours_in_without_losing_a_credit() {
    let envelope = an_envelope();
    let body = serde_json::to_value(submission(&envelope, "a-key")).unwrap();
    assert_eq!(
        body["artists"],
        serde_json::json!(["Nova Kern", "Juno Vale"])
    );
    assert_eq!(body["albumartists"], serde_json::json!(["Nova Kern"]));
    assert_eq!(body["album"], "Convergence");
    assert_eq!(body["title"], "Zenith");
}

/// Both instants are seconds, and both are truncated the same way.
///
/// Milliseconds handed over unconverted would date every listen about fifty
/// thousand years from now, and a listening history keeps that for good.
#[test]
fn the_instants_are_seconds_and_not_milliseconds() {
    let envelope = an_envelope();
    let body = serde_json::to_value(submission(&envelope, "a-key")).unwrap();
    assert_eq!(body["time"], 1_700_000_123);
    assert_eq!(body["length"], 245);

    // `div_euclid`, so an instant before 1970 rounds down rather than
    // towards zero. No listen has one; the point is not to depend on that.
    let ancient = ScrobbleEnvelope {
        played_at: -1_500,
        ..an_envelope()
    };
    let body = serde_json::to_value(submission(&ancient, "a-key")).unwrap();
    assert_eq!(body["time"], -2);
}

/// What this server does not know is not sent.
///
/// An absent album and an absent duration are absent, not empty: a
/// destination matching on `album` treats `""` as a title, and Maloja's
/// `duration` is a listened length this server never measured.
#[test]
fn what_is_unknown_is_left_out_rather_than_sent_empty() {
    let bare = ScrobbleEnvelope {
        album: None,
        album_artist: None,
        duration_ms: None,
        ..an_envelope()
    };
    let body = serde_json::to_value(submission(&bare, "a-key")).unwrap();
    let object = body.as_object().unwrap();
    for absent in ["album", "albumartists", "length", "duration"] {
        assert!(!object.contains_key(absent), "{absent} must not be sent");
    }
    // And what is always known is always sent.
    for present in ["key", "title", "artists", "time"] {
        assert!(object.contains_key(present), "{present} must be sent");
    }
}

/// The body carries a secret, so it does not survive being formatted.
///
/// The failure message deliberately carries neither the key nor the
/// formatted output: if this guard were removed, that output would *be* the
/// credential, and a panic message is a log line.
#[test]
fn a_submission_does_not_print_its_key() {
    let envelope = an_envelope();
    let shown = format!("{:?}", submission(&envelope, "a-key-nobody-should-read"));
    assert!(
        !shown.contains("a-key-nobody-should-read"),
        "the debug output carried the key"
    );
    assert!(shown.contains("[redacted]"), "the key was not replaced");
}

/// A status line, and nothing else, decides.
#[test]
fn the_status_line_earns_the_verdict() {
    assert!(matches!(
        status_verdict(reqwest::StatusCode::OK, None),
        ScrobbleVerdict::Accepted
    ));
    for refused in [401, 403] {
        assert!(
            matches!(
                status_verdict(reqwest::StatusCode::from_u16(refused).unwrap(), None),
                ScrobbleVerdict::AuthBroken
            ),
            "{refused}"
        );
    }
    for refused in [400, 413, 422] {
        assert!(
            matches!(
                status_verdict(reqwest::StatusCode::from_u16(refused).unwrap(), None),
                ScrobbleVerdict::PermanentReject
            ),
            "{refused}"
        );
    }
    // A rate limit with no readable header still rests the link: the status
    // says room was asked for, the header only says how much.
    assert!(matches!(
        status_verdict(reqwest::StatusCode::TOO_MANY_REQUESTS, None),
        ScrobbleVerdict::Retryable {
            after: Some(Duration::ZERO)
        }
    ));
    // A redirect is not followed, and means the destination moved.
    assert!(matches!(
        status_verdict(reqwest::StatusCode::FOUND, None),
        ScrobbleVerdict::Retryable { after: None }
    ));
}

/// A `200` whose body is over the ceiling is not read, and the status line
/// decides (RFC-023 decision 1).
///
/// The body is the one refusal this adapter reads, padded: a reader
/// without the ceiling reads all of it and rejects the listen for good,
/// which is the difference this tells apart.
#[tokio::test]
async fn an_answer_over_the_ceiling_leaves_the_status_line_deciding() {
    let padding = " ".repeat(crate::remote::outbound::MAX_ANSWER_BYTES);
    let (url, server) = crate::remote::outbound::tests::answering(axum::body::Body::from(format!(
        "{{\"status\":\"failure\"{padding}}}"
    )))
    .await;
    let base = super::super::validate_destination(&url, true).expect("a destination");
    let client = super::super::outbound_client(Duration::from_secs(5)).expect("a client");
    let target = Maloja::new(client, &base).expect("an adapter");
    let secret = uuid::Uuid::new_v4().to_string();
    let envelope = an_envelope();
    let verdict = target.submit(&envelope, &secret).await;
    server.abort();
    assert!(matches!(verdict, ScrobbleVerdict::Accepted));
}
