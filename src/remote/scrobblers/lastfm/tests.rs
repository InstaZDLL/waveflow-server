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

/// The signature is Last.fm's own recipe, checked against a value computed
/// by hand from it.
///
/// Sorted by name, name and value concatenated with nothing between them,
/// the shared secret appended, MD5 of the UTF-8 bytes. Every one of those is
/// a place to be wrong in a way no test that only signed and compared
/// against itself would ever notice — which is why the expectation here is a
/// literal rather than a second call to `signature`.
#[test]
fn the_signature_follows_the_recipe_last_fm_publishes() {
    use md5::Digest as _;
    let params = [
        ("method", "auth.getSession".to_owned()),
        ("api_key", "the-key".to_owned()),
        ("token", "the-token".to_owned()),
    ];
    // "api_keythe-keymethodauth.getSessiontokenthe-token" + secret.
    let expected = hex::encode(md5::Md5::digest(
        b"api_keythe-keymethodauth.getSessiontokenthe-tokenthe-secret",
    ));
    assert_eq!(signature(&params, "the-secret"), expected);
}

/// `format` and `callback` are outside the signature.
///
/// Last.fm's rule, and it is not cosmetic: including either produces a
/// signature they refuse, and the failure would arrive as an authentication
/// error that looks exactly like a bad session key.
#[test]
fn the_shape_of_the_answer_is_not_part_of_the_signature() {
    let bare = [("method", "track.scrobble".to_owned())];
    let dressed = [
        ("method", "track.scrobble".to_owned()),
        ("format", "json".to_owned()),
        ("callback", "https://example.test/back".to_owned()),
    ];
    assert_eq!(
        signature(&bare, "the-secret"),
        signature(&dressed, "the-secret")
    );
}

/// The order the parameters were built in does not reach the signature.
#[test]
fn the_signature_sorts_before_it_concatenates() {
    let one_way = [
        ("track", "Zenith".to_owned()),
        ("artist", "Nova Kern".to_owned()),
    ];
    let the_other = [
        ("artist", "Nova Kern".to_owned()),
        ("track", "Zenith".to_owned()),
    ];
    assert_eq!(
        signature(&one_way, "the-secret"),
        signature(&the_other, "the-secret")
    );
}

/// The instants are seconds, and every credit is carried.
#[test]
fn the_submission_carries_seconds_and_every_credit() {
    let params = scrobble_params(&an_envelope(), "a-session-key");
    let value = |name: &str| {
        params
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.as_str())
    };
    assert_eq!(value("timestamp"), Some("1700000123"));
    assert_eq!(value("duration"), Some("245"));
    assert_eq!(value("artist"), Some("Nova Kern, Juno Vale"));
    assert_eq!(value("track"), Some("Zenith"));
    assert_eq!(value("album"), Some("Convergence"));
    assert_eq!(value("albumArtist"), Some("Nova Kern"));
    assert_eq!(value("sk"), Some("a-session-key"));
}

/// What this server does not know is not sent.
#[test]
fn what_is_unknown_is_left_out_rather_than_sent_empty() {
    let bare = ScrobbleEnvelope {
        album: None,
        album_artist: None,
        duration_ms: None,
        musicbrainz_recording_id: None,
        ..an_envelope()
    };
    let params = scrobble_params(&bare, "a-session-key");
    for absent in ["album", "albumArtist", "duration", "mbid"] {
        assert!(
            !params.iter().any(|(name, _)| *name == absent),
            "{absent} must not be sent"
        );
    }
}

/// The endpoint keeps the trailing slash Last.fm documents.
///
/// `/2.0` answers a redirect, and this client follows none: without the
/// slash every call would fail, and the adapter would read that failure as
/// the destination having moved.
#[test]
fn the_endpoint_keeps_the_slash_last_fm_answers_on() {
    let base = super::super::validate_destination("https://ws.audioscrobbler.com", false)
        .expect("a valid destination");
    let client = super::super::outbound_client(Duration::from_secs(5)).expect("a client");
    let application = crate::config::LastFmApplication {
        api_key: "a-key".to_owned(),
        secret: "a-secret".to_owned(),
    };
    let target =
        LastFm::new(client, &base, application, Duration::from_secs(5)).expect("an adapter");
    assert_eq!(
        target.endpoint.as_str(),
        "https://ws.audioscrobbler.com/2.0/"
    );
}

/// A status line, and nothing else, decides.
#[test]
fn the_status_line_earns_the_verdict() {
    assert!(matches!(
        status_verdict(reqwest::StatusCode::OK, None),
        ScrobbleVerdict::Accepted
    ));
    assert!(matches!(
        status_verdict(reqwest::StatusCode::FORBIDDEN, None),
        ScrobbleVerdict::AuthBroken
    ));
    assert!(matches!(
        status_verdict(reqwest::StatusCode::BAD_REQUEST, None),
        ScrobbleVerdict::PermanentReject
    ));
    assert!(matches!(
        status_verdict(reqwest::StatusCode::TOO_MANY_REQUESTS, None),
        ScrobbleVerdict::Retryable {
            after: Some(Duration::ZERO)
        }
    ));
}

/// An adapter aimed at a listener of the test's own.
async fn reaching(body: axum::body::Body) -> (LastFm, tokio::task::JoinHandle<()>) {
    let (url, server) = crate::remote::outbound::tests::answering(body).await;
    let base = super::super::validate_destination(&url, true).expect("a destination");
    let client = super::super::outbound_client(Duration::from_secs(5)).expect("a client");
    let application = crate::config::LastFmApplication {
        api_key: uuid::Uuid::new_v4().to_string(),
        secret: uuid::Uuid::new_v4().to_string(),
    };
    let target =
        LastFm::new(client, &base, application, Duration::from_secs(10)).expect("an adapter");
    (target, server)
}

/// A session answer that is well formed and says what it should, but is
/// padded past the ceiling, is not read: the exchange fails as an outage
/// rather than holding the whole body (RFC-023 decision 1).
///
/// Finite on purpose, and parseable: without the ceiling this same answer
/// is read and yields its key, so the test tells the ceiling from its
/// absence by the outcome and not only by the clock.
#[tokio::test]
async fn a_session_answer_over_the_ceiling_is_not_read() {
    use crate::services::{LastFmSessionExchange, ServiceError};
    let key = uuid::Uuid::new_v4().simple().to_string();
    let padding = " ".repeat(crate::remote::outbound::MAX_ANSWER_BYTES);
    let (target, server) = reaching(axum::body::Body::from(format!(
        "{{\"session\":{{\"key\":\"{key}\"}}{padding}}}"
    )))
    .await;
    let token = uuid::Uuid::new_v4().to_string();
    let exchanged = target.exchange(&token).await;
    server.abort();
    assert!(
        matches!(exchanged, Err(ServiceError::Unavailable)),
        "an oversized answer must not be read"
    );

    // And the same answer within the ceiling is read, so the refusal above
    // is the ceiling's and not the listener's.
    let (target, server) = reaching(axum::body::Body::from(format!(
        "{{\"session\":{{\"key\":\"{key}\"}}}}"
    )))
    .await;
    let exchanged = target.exchange(&token).await;
    server.abort();
    assert_eq!(exchanged.ok().as_deref(), Some(key.as_str()));
}

/// The same for the token the CLI journey asks for first.
#[tokio::test]
async fn a_token_answer_over_the_ceiling_is_not_read() {
    use crate::services::{LastFmSessionExchange, ServiceError};
    let token = uuid::Uuid::new_v4().simple().to_string();
    let padding = " ".repeat(crate::remote::outbound::MAX_ANSWER_BYTES);
    let (target, server) = reaching(axum::body::Body::from(format!(
        "{{\"token\":\"{token}\"{padding}}}"
    )))
    .await;
    let minted = target.request_token().await;
    server.abort();
    assert!(matches!(minted, Err(ServiceError::Unavailable)));
}

/// A listen whose acknowledgement is over the ceiling is judged by its
/// status line, as one that will not parse is.
///
/// The body says `"error": 9`, a dead session key, so a reader without the
/// ceiling breaks the link over it; that difference is what this tells
/// apart.
#[tokio::test]
async fn a_scrobble_answer_over_the_ceiling_leaves_the_status_line_deciding() {
    let padding = " ".repeat(crate::remote::outbound::MAX_ANSWER_BYTES);
    let (target, server) =
        reaching(axum::body::Body::from(format!("{{\"error\":9{padding}}}"))).await;
    let envelope = an_envelope();
    let session_key = uuid::Uuid::new_v4().to_string();
    let verdict = target.submit(&envelope, &session_key).await;
    server.abort();
    assert!(matches!(verdict, ScrobbleVerdict::Accepted));
}
