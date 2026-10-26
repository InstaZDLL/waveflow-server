//! The Maloja and Last.fm adapters on the wire: credits, refusals inside a
//! success, rate limits and tokens.

use super::*;

/// Points Maloja's declared instance at `base`, leaving the others alone.
fn maloja_reaching(config: &mut waveflow_server::Config, base: &str) {
    let url = waveflow_server::scrobblers::validate_destination(base, true)
        .expect("a destination the suite chose");
    let fingerprint = waveflow_server::scrobblers::destination_fingerprint(&url);
    config
        .destinations
        .retain(|declared| declared.provider != ScrobbleProvider::Maloja);
    config
        .destinations
        .push(waveflow_server::config::ScrobbleDestination {
            provider: ScrobbleProvider::Maloja,
            name: "default".to_owned(),
            url,
            fingerprint,
        });
}

/// A listen reaches Maloja in the shape its API documents, with every credit.
///
/// Through `initialize`, so it exercises the whole chain a real server walks —
/// the destination is validated, the client is built, the adapter is registered
/// and keyed by the pair — rather than registering a double by hand.
///
/// **The credits are the point.** ListenBrainz has one `artist_name` field, so
/// its adapter joins them and lets the far end re-match; Maloja takes a list, so
/// a duo stays a duo. An adapter that joined here would work, and would quietly
/// invent a band in somebody's statistics.
#[tokio::test]
async fn a_listen_reaches_maloja_with_every_credit_it_was_heard_with() {
    let destination = spawn_destination(200, None).await;
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    maloja_reaching(&mut config, &destination.base);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "maloja-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "default",
            "maloja-key",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(
            listener.owner,
            listener.tagged,
            true,
            Some(1_700_000_000_123),
        )
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.accepted, 1);
    assert_eq!(rows(&state).await[0].1, "sent");

    let bodies = destination.bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let body = &bodies[0];
    assert_eq!(body["title"], "Matrix flac");
    assert_eq!(body["artists"], serde_json::json!(["Alpha", "Beta"]));
    // **Seconds.** The envelope holds epoch milliseconds like everything else
    // here, and sending those unconverted would date this listen some fifty
    // thousand years out — in a history that keeps it.
    assert_eq!(body["time"], 1_700_000_000);
    // And the key travels in the body, which is where every version of Maloja
    // accepts it — so nothing went out in an `Authorization` header.
    assert_eq!(body["key"], "maloja-key");
    assert_eq!(
        destination.authorizations.lock().unwrap().clone(),
        vec![String::new()],
        "the key must not also travel in a header"
    );
}

/// A destination answering `200` with a body of the test's choosing.
///
/// The recorder above always answers `{}`, which is the shape of a success.
/// Two of the three destinations announce a refusal *inside* a `200`, and
/// nothing but a body can say so.
async fn spawn_answering(path: &'static str, body: &'static str) -> String {
    spawn_answering_with(path, 200, body, None).await
}

/// The same, with a status line and a header of the test's choosing.
///
/// Two destinations announce refusals inside a `200`, and one of them also
/// announces its own error codes beside a `429` — where the *header* is what
/// says how long to wait, and the body must not be allowed to answer instead.
async fn spawn_answering_with(
    path: &'static str,
    status: u16,
    body: &'static str,
    header: Option<(&'static str, &'static str)>,
) -> String {
    let router = axum::Router::new().route(
        path,
        axum::routing::post(move || async move {
            let mut response = axum::response::Response::builder()
                .status(axum::http::StatusCode::from_u16(status).unwrap())
                .header(axum::http::header::CONTENT_TYPE, "application/json");
            if let Some((name, value)) = header {
                response = response.header(name, value);
            }
            response.body(axum::body::Body::from(body)).unwrap()
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

/// A refusal Maloja announces inside a `200` is a refusal.
///
/// The first draft of that adapter read the status line alone, on the strength
/// of decision 12 — what this server publishes is a state, never an echo. That
/// governs what reaches a *member*; it says nothing about what an adapter may
/// read to reach a verdict. The cost of the earlier reading was a silent gap:
/// a listen Maloja refused, recorded here as `sent`.
#[tokio::test]
async fn a_refusal_maloja_announces_in_a_success_is_not_a_success() {
    let base = spawn_answering(
        "/apis/mlj_1/newscrobble",
        r#"{"status":"failure","error":{"type":"nonexistent_track"}}"#,
    )
    .await;
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    maloja_reaching(&mut config, &base);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "refused-by-maloja-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "default",
            "maloja-key",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.accepted, 0);
    assert_eq!(
        rows(&state).await[0].1,
        "rejected",
        "a refusal announced in the body must not be recorded as sent"
    );
}

/// A Last.fm session key that has stopped working breaks the link.
///
/// Last.fm answers `200` carrying `"error": 9` for it. Read by the status line
/// alone, the link would stay `healthy` and every listen would be recorded as
/// `sent` while nothing was recorded anywhere — a silent gap, which is the
/// exact failure this RFC spends itself making visible.
#[tokio::test]
async fn a_last_fm_session_key_that_died_breaks_the_link_rather_than_looking_sent() {
    let base = spawn_answering(
        "/2.0/",
        r#"{"error":9,"message":"Invalid session key - Please re-authenticate."}"#,
    )
    .await;
    let temp = tempfile::tempdir().unwrap();
    let mut config = lastfm_app(&temp);
    let url = waveflow_server::scrobblers::validate_destination(&base, true).unwrap();
    let fingerprint = waveflow_server::scrobblers::destination_fingerprint(&url);
    config.destinations = vec![waveflow_server::config::ScrobbleDestination {
        provider: ScrobbleProvider::LastFm,
        name: "default".to_owned(),
        url,
        fingerprint,
    }];
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "stale-lastfm-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::LastFm,
            "default",
            "a-session-key",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.accepted, 0);
    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links[0].health, "broken");
    assert_eq!(links[0].last_failure.as_deref(), Some("auth_broken"));
}

/// An app reaching one Last.fm instance at `base`, with an application.
async fn lastfm_app_reaching(
    temp: &tempfile::TempDir,
    base: &str,
) -> (waveflow_server::Config, AppState) {
    let mut config = lastfm_app(temp);
    let url = waveflow_server::scrobblers::validate_destination(base, true).unwrap();
    let fingerprint = waveflow_server::scrobblers::destination_fingerprint(&url);
    config.destinations = vec![waveflow_server::config::ScrobbleDestination {
        provider: ScrobbleProvider::LastFm,
        name: "default".to_owned(),
        url,
        fingerprint,
    }];
    let state = waveflow_server::initialize(&config).await.unwrap();
    (config, state)
}

/// A failing status line decides, and the body does not answer for it.
///
/// Last.fm names its own rate limit `error: 29` in the body of the `429` that
/// carries `Retry-After`. Letting the body answer returns this adapter's own
/// zero wait — which is not a shorter wait, it is the *header discarded*, and
/// the header is the only thing that says how much room was asked for.
///
/// The number below separates the two readings: the queue takes the larger of
/// its own backoff and what was asked, so a discarded header lands on the
/// backoff — about a minute — and an honoured one lands past two.
#[tokio::test]
async fn a_rate_limit_is_read_from_the_header_and_not_from_the_body_beside_it() {
    let base = spawn_answering_with(
        "/2.0/",
        429,
        r#"{"error":29,"message":"Rate limit exceeded"}"#,
        Some(("retry-after", "240")),
    )
    .await;
    let temp = tempfile::tempdir().unwrap();
    let (config, state) = lastfm_app_reaching(&temp, &base).await;
    let listener = fixture(&config, &state, "rate-limited-lastfm-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::LastFm,
            "default",
            "a-session-key",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();

    let before = now_ms();
    state.services.drain_scrobble_outbox().await.unwrap();

    let next: i64 = sqlx::query_scalar("SELECT next_attempt_at FROM scrobble_outbox")
        .fetch_one(state.db.pool())
        .await
        .unwrap();
    assert!(
        next - before >= 240_000,
        "the wait must honour the header, not the code beside it: {}ms",
        next - before
    );
}

/// The real adapter asks Last.fm for a token, and puts it in the address.
///
/// The three tests above drive a double, which proves the service's half and
/// nothing about the wire. This one stands a server on loopback and lets the
/// Last.fm adapter talk to it — so what is checked here is what only exists
/// there: that the call is `auth.getToken`, that it is signed, that the token
/// is read out of the answer, and that the address handed to the operator
/// carries the application key and that same token.
#[tokio::test]
async fn the_adapter_asks_last_fm_for_a_token_and_names_it_in_the_address() {
    let base = spawn_answering("/2.0/", r#"{"token":"a-real-request-token"}"#).await;
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    let url = waveflow_server::scrobblers::validate_destination(&base, true)
        .expect("a destination the suite chose");
    let fingerprint = waveflow_server::scrobblers::destination_fingerprint(&url);
    config
        .destinations
        .retain(|declared| declared.provider != ScrobbleProvider::LastFm);
    config
        .destinations
        .push(waveflow_server::config::ScrobbleDestination {
            provider: ScrobbleProvider::LastFm,
            name: "default".to_owned(),
            url,
            fingerprint,
        });
    // An application and no public address: the browser-less journey's whole
    // precondition, and the adapter is registered on the application alone.
    config.public_url = None;
    config.lastfm = Some(waveflow_server::config::LastFmApplication {
        api_key: "an-application-key".to_owned(),
        secret: "an-application-secret".to_owned(),
    });
    let state = waveflow_server::initialize(&config).await.unwrap();

    let approval = state
        .services
        .begin_lastfm_approval("default")
        .await
        .unwrap();

    assert_eq!(approval.token, "a-real-request-token");
    // The website, not the API endpoint the destination names — two different
    // hosts, and the address is built from a constant for that reason.
    assert!(approval
        .authorize_url
        .starts_with("https://www.last.fm/api/auth/"));
    assert!(approval
        .authorize_url
        .contains("api_key=an-application-key"));
    assert!(approval
        .authorize_url
        .contains("token=a-real-request-token"));
}

/// An answer with no token in it is a failure, not an empty token.
///
/// Last.fm reports a bad key or a signature this server computed wrong as a
/// `200` carrying an error object. Read as success, it would hand the operator
/// an address with nothing in it and the journey would fail one step later,
/// with nothing saying why.
#[tokio::test]
async fn an_answer_carrying_no_token_is_refused_rather_than_passed_on() {
    let base = spawn_answering("/2.0/", r#"{"error":4,"message":"Invalid API key"}"#).await;
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    let url = waveflow_server::scrobblers::validate_destination(&base, true)
        .expect("a destination the suite chose");
    let fingerprint = waveflow_server::scrobblers::destination_fingerprint(&url);
    config
        .destinations
        .retain(|declared| declared.provider != ScrobbleProvider::LastFm);
    config
        .destinations
        .push(waveflow_server::config::ScrobbleDestination {
            provider: ScrobbleProvider::LastFm,
            name: "default".to_owned(),
            url,
            fingerprint,
        });
    config.lastfm = Some(waveflow_server::config::LastFmApplication {
        api_key: "an-application-key".to_owned(),
        secret: "an-application-secret".to_owned(),
    });
    let state = waveflow_server::initialize(&config).await.unwrap();

    assert!(matches!(
        state.services.begin_lastfm_approval("default").await,
        Err(ServiceError::Unavailable)
    ));
}
