use super::*;

fn envelope() -> ScrobbleEnvelope {
    ScrobbleEnvelope {
        // 2026-09-13T11:32:42Z, and the millisecond remainder is there on
        // purpose: it must not survive the conversion.
        played_at: 1_789_648_362_750,
        title: "Never Gonna Give You Up".into(),
        artists: vec!["Rick Astley".into(), "The Guest".into()],
        album: Some("Whenever You Need Somebody".into()),
        album_artist: Some("Rick Astley".into()),
        duration_ms: Some(222_000),
        musicbrainz_recording_id: Some("98255a8c-017a-4bc7-8dd6-1fa36124572b".into()),
    }
}

/// The one conversion a listening history would keep forever if it were
/// wrong.
#[test]
fn a_listen_is_dated_in_seconds_and_not_milliseconds() {
    assert_eq!(played_at_seconds(1_789_648_362_750), 1_789_648_362);
    // Truncation towards zero would answer 0 here and -1 is correct; no
    // listen predates 1970, which is why this is worth pinning rather than
    // assuming.
    assert_eq!(played_at_seconds(-1), -1);

    let body = serde_json::to_value(submission(&envelope())).unwrap();
    assert_eq!(body["payload"][0]["listened_at"], 1_789_648_362);
}

#[test]
fn the_body_is_the_shape_listenbrainz_documents() {
    let body = serde_json::to_value(submission(&envelope())).unwrap();
    assert_eq!(body["listen_type"], "single");
    assert_eq!(body["payload"].as_array().unwrap().len(), 1);

    let metadata = &body["payload"][0]["track_metadata"];
    assert_eq!(metadata["track_name"], "Never Gonna Give You Up");
    assert_eq!(metadata["release_name"], "Whenever You Need Somebody");
    // Every credit, in tag order: the envelope records what was heard, and
    // a guest on the track was heard.
    assert_eq!(metadata["artist_name"], "Rick Astley, The Guest");
    assert_eq!(metadata["additional_info"]["duration_ms"], 222_000);
    assert_eq!(
        metadata["additional_info"]["recording_mbid"],
        "98255a8c-017a-4bc7-8dd6-1fa36124572b"
    );
}

/// An absent field is absent, rather than present and null: the destination
/// reads `additional_info` as a set of hints, and a null hint is a claim.
#[test]
fn what_the_listen_does_not_know_is_left_out_entirely() {
    let bare = ScrobbleEnvelope {
        album: None,
        duration_ms: None,
        musicbrainz_recording_id: None,
        ..envelope()
    };
    let body = serde_json::to_value(submission(&bare)).unwrap();
    let metadata = &body["payload"][0]["track_metadata"];
    assert!(metadata.get("release_name").is_none());
    assert!(metadata["additional_info"].get("duration_ms").is_none());
    assert!(metadata["additional_info"].get("recording_mbid").is_none());
    // And what it always knows stays.
    assert_eq!(
        metadata["additional_info"]["submission_client"],
        CLIENT_NAME
    );
}

/// A rate limit may ask in either of the two forms RFC 9110 allows — and
/// two more that are not a request to wait at all.
///
/// Decision 6 says a delay the destination asks for is honoured. Only
/// ListenBrainz's own vendor header was read, so a `429` from the nginx or
/// the CDN in front of a self-hosted instance — which answers with the
/// standard header — was heard as "no delay given".
#[test]
fn a_rate_limit_may_ask_in_seconds_or_in_a_date() {
    fn answered(header: &str, value: &str) -> Option<Duration> {
        let response = axum::http::Response::builder()
            .status(429)
            .header(header, value)
            .body(String::new())
            .unwrap();
        asked_wait(&reqwest::Response::from(response))
    }

    // The vendor's header, in whole seconds.
    assert_eq!(
        answered(RESET_IN_HEADER, "12"),
        Some(Duration::from_secs(12))
    );
    // The standard one, as delta-seconds.
    assert_eq!(answered("retry-after", "30"), Some(Duration::from_secs(30)));

    // And as an HTTP date, which is the form a proxy answers with. Read as
    // a distance from now, so it is asserted as a window rather than a
    // value: the clock moves between building the header and reading it.
    let soon = chrono::Utc::now() + chrono::Duration::seconds(120);
    let asked = answered("retry-after", &soon.to_rfc2822()).expect("a future date is a wait");
    assert!(
        asked > Duration::from_secs(110) && asked <= Duration::from_secs(120),
        "a date two minutes out must read as about two minutes, got {asked:?}"
    );

    // A moment already gone is not a request to wait, and neither is a
    // header nobody can parse. Both read as absent — which sends the entry
    // to the queue's own backoff, not to an immediate retry.
    let past = chrono::Utc::now() - chrono::Duration::seconds(120);
    assert_eq!(answered("retry-after", &past.to_rfc2822()), None);
    assert_eq!(answered("retry-after", "in a little while"), None);
    // RFC 9110's delta-seconds is `1*DIGIT`, so a signed number is not one.
    // `u64::from_str` accepts a leading `+`, which read `+30` as a wait
    // nobody had spelled in a form this is allowed to understand.
    assert_eq!(answered("retry-after", "+30"), None);
    assert_eq!(answered("x-unrelated", "30"), None);
}

#[test]
fn each_answer_earns_the_verdict_the_queue_can_act_on() {
    use reqwest::StatusCode;
    let no_wait = None;

    assert_eq!(
        status_verdict(StatusCode::OK, no_wait),
        ScrobbleVerdict::Accepted
    );
    // A bad token breaks the link rather than the listen.
    assert_eq!(
        status_verdict(StatusCode::UNAUTHORIZED, no_wait),
        ScrobbleVerdict::AuthBroken
    );
    // A refused payload is terminal: no retry can fix it.
    assert_eq!(
        status_verdict(StatusCode::BAD_REQUEST, no_wait),
        ScrobbleVerdict::PermanentReject
    );
    // And a rate limit carries the destination's own answer back, exactly
    // as it was given — bounding it is the queue's business, not this
    // function's.
    assert_eq!(
        status_verdict(StatusCode::TOO_MANY_REQUESTS, Some(Duration::from_secs(12))),
        ScrobbleVerdict::Retryable {
            after: Some(Duration::from_secs(12))
        }
    );
    // A server fault says nothing about the listen.
    assert_eq!(
        status_verdict(StatusCode::BAD_GATEWAY, no_wait),
        ScrobbleVerdict::Retryable { after: None }
    );
    // A redirect is not followed, so it reaches here: the operator's
    // destination moved, and the listen waits for them to notice.
    assert_eq!(
        status_verdict(StatusCode::FOUND, no_wait),
        ScrobbleVerdict::Retryable { after: None }
    );
}
