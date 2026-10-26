//! The Last.fm linking journey, with and without a browser.

use super::*;

/// A double for the half of the journey that would leave this machine.
///
/// Records the request token it was handed, so a test can assert that the one
/// Last.fm appended is the one exchanged — and not, say, a second token this
/// server asked for on its own, which is the shape of the `auth.getToken`
/// mistake the RFC names.
struct ExchangesFor {
    session_key: Result<String, ()>,
    tokens: Mutex<Vec<String>>,
    /// What `auth.getToken` would answer, and how often it was asked.
    ///
    /// Counted as well as answered, because the sharpest thing a test can say
    /// about the web journey is that this was **never called** — the token it
    /// exchanges is the one Last.fm appended to the return, and a server that
    /// minted its own would hold the wrong one.
    minted: Mutex<(String, usize)>,
}

impl ExchangesFor {
    fn key(session_key: &str) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            session_key: Ok(session_key.to_owned()),
            tokens: Mutex::new(Vec::new()),
            minted: Mutex::new(("minted-request-token".to_owned(), 0)),
        })
    }

    fn refusing() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            session_key: Err(()),
            tokens: Mutex::new(Vec::new()),
            minted: Mutex::new(("minted-request-token".to_owned(), 0)),
        })
    }

    fn tokens(&self) -> Vec<String> {
        self.tokens.lock().unwrap().clone()
    }

    /// How many times a request token was asked for.
    fn mintings(&self) -> usize {
        self.minted.lock().unwrap().1
    }
}

impl waveflow_server::services::LastFmSessionExchange for ExchangesFor {
    fn exchange<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<String, ServiceError>> {
        Box::pin(async move {
            self.tokens.lock().unwrap().push(token.to_owned());
            self.session_key.clone().map_err(|()| ServiceError::Invalid)
        })
    }

    fn request_token<'a>(
        &'a self,
    ) -> BoxFuture<'a, Result<waveflow_server::services::LastFmApproval, ServiceError>> {
        Box::pin(async move {
            let mut minted = self.minted.lock().unwrap();
            minted.1 += 1;
            let token = minted.0.clone();
            Ok(waveflow_server::services::LastFmApproval {
                authorize_url: format!("https://www.last.fm/api/auth/?api_key=k&token={token}"),
                token,
            })
        })
    }
}

/// The `state` segment out of an authorisation URL.
fn state_of(authorize_url: &str) -> String {
    let parsed = url::Url::parse(authorize_url).expect("an absolute authorisation URL");
    let callback = parsed
        .query_pairs()
        .find(|(name, _)| name == "cb")
        .map(|(_, value)| value.into_owned())
        .expect("the authorisation URL names where to come back");
    callback
        .rsplit('/')
        .next()
        .expect("the state is the last segment")
        .to_owned()
}

/// A person comes back from Last.fm and the link exists.
///
/// The whole journey, end to end, with only the half that would leave this
/// machine replaced. The return is driven through the router rather than
/// through the service, because everything this test is about — the trailing
/// slash, the cookie, the query string, the headers on the answer — exists at
/// that level and nowhere else.
#[tokio::test]
async fn a_person_who_comes_back_from_last_fm_ends_up_linked() {
    let temp = tempfile::tempdir().unwrap();
    let config = lastfm_app(&temp);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "lastfm-listener").await;
    let exchange = ExchangesFor::key("a-session-key");
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    let started = state
        .services
        .begin_lastfm_authorization(listener.owner, "default")
        .await
        .unwrap();
    // The authorisation page carries this server's application key and the
    // address to come back to — and the random travels in that address's
    // *path*, because Last.fm documents that it appends `/?token=…` and says
    // nothing about what it would do with a `cb` that already carried a `?`.
    assert!(started
        .authorize_url
        .starts_with("https://www.last.fm/api/auth/"));
    assert!(started.authorize_url.contains("api_key=an-application-key"));
    assert!(started
        .authorize_url
        .contains("waveflow.example%2Fapi%2Fv2%2Fscrobble-links%2Flastfm%2Fcallback%2F"));
    let journey = state_of(&started.authorize_url);

    // **With the trailing slash**, which is what Last.fm actually sends the
    // browser to. `axum` does not normalise it, so a route declared only
    // without it would fail at the journey's last step for everybody.
    let router = waveflow_server::app(&config, state.clone());
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri(format!(
                    "/api/v2/scrobble-links/lastfm/callback/{journey}/?token=a-request-token"
                ))
                .header(
                    axum::http::header::COOKIE,
                    format!("waveflow_lastfm_journey={}", started.cookie),
                )
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::SEE_OTHER);
    // The token is worth an hour and worth a profile: the answer keeps nothing
    // and sends nothing onward, and redirects at once to an address without it,
    // which is the one a history keeps.
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CACHE_CONTROL)
            .unwrap(),
        "no-store"
    );
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::REFERRER_POLICY)
            .unwrap(),
        "no-referrer"
    );
    let landing = response
        .headers()
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(!landing.contains("a-request-token"), "{landing}");

    // The token Last.fm appended is the one exchanged — not a second one this
    // server asked for, which is the `auth.getToken` mistake.
    assert_eq!(exchange.tokens(), vec!["a-request-token".to_owned()]);

    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].provider, ScrobbleProvider::LastFm);
    assert_eq!(links[0].destination, "default");
    assert_eq!(links[0].health, "healthy");

    // And the journey is spent: the same return, replayed, links nothing more.
    let router = waveflow_server::app(&config, state.clone());
    let replayed = router
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::GET)
                .uri(format!(
                    "/api/v2/scrobble-links/lastfm/callback/{journey}/?token=a-request-token"
                ))
                .header(
                    axum::http::header::COOKIE,
                    format!("waveflow_lastfm_journey={}", started.cookie),
                )
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replayed.status(), axum::http::StatusCode::NOT_FOUND);
    assert_eq!(exchange.tokens().len(), 1, "nothing was exchanged twice");
}

/// Which cookie the return is offered, in the loop below.
enum Cookie {
    /// The one `authorize` set for this journey.
    Own,
    /// None at all — refused by the route, before the service is reached.
    None,
    /// A well-formed one this server never issued — the case that reaches the
    /// comparison, and the only one that proves it exists.
    Wrong,
}

/// What the return refuses before it exchanges anything.
///
/// A missing token, an empty one, a repeated one, and a return with no cookie.
/// In each the exchange is never called and no link is created, because the
/// call comes after these refusals rather than before them.
///
/// **The repeated one deserves naming.** `?token=a&token=b` lets an extractor
/// choose, and a journey whose outcome depends on which duplicate a reader
/// keeps is not a journey.
#[tokio::test]
async fn the_return_refuses_before_it_exchanges_anything() {
    let temp = tempfile::tempdir().unwrap();
    let config = lastfm_app(&temp);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "refusing-lastfm-listener").await;
    let exchange = ExchangesFor::key("a-session-key");
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    // **The wrong cookie and the missing cookie are two different cases**, and
    // only the first reaches the comparison. An earlier version of this loop
    // carried "no cookie at all" alone: the route refuses that before the
    // service is called, so removing the comparison altogether left the test
    // green. It is the pairing that this is about — a URL that travels through
    // Last.fm lands in a referrer and a history, and whoever found it must not
    // be able to finish the journey with their own token.
    for (case, query, cookie) in [
        ("no token at all", "", Cookie::Own),
        ("an empty token", "?token=", Cookie::Own),
        ("a repeated token", "?token=a&token=b", Cookie::Own),
        ("no cookie at all", "?token=a-request-token", Cookie::None),
        (
            "a cookie from another browser",
            "?token=a-request-token",
            Cookie::Wrong,
        ),
    ] {
        // A journey per case: each is opened fresh, so a refusal cannot pass
        // because a previous case had already spent the state.
        let started = state
            .services
            .begin_lastfm_authorization(listener.owner, "default")
            .await
            .unwrap();
        let journey = state_of(&started.authorize_url);
        let mut request = axum::http::Request::builder()
            .method(axum::http::Method::GET)
            .uri(format!(
                "/api/v2/scrobble-links/lastfm/callback/{journey}{query}"
            ));
        match cookie {
            Cookie::Own => {
                request = request.header(
                    axum::http::header::COOKIE,
                    format!("waveflow_lastfm_journey={}", started.cookie),
                );
            }
            Cookie::Wrong => {
                // Well formed, and never issued by this server.
                request = request.header(
                    axum::http::header::COOKIE,
                    "waveflow_lastfm_journey=a-cookie-nobody-here-set",
                );
            }
            Cookie::None => {}
        }
        let router = waveflow_server::app(&config, state.clone());
        let response = router
            .oneshot(request.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::NOT_FOUND,
            "{case} must be refused"
        );
    }

    assert!(
        exchange.tokens().is_empty(),
        "nothing may be exchanged before the refusals"
    );
    assert!(state
        .services
        .scrobble_links(listener.owner)
        .await
        .unwrap()
        .is_empty());
}

/// The state is spent before the exchange, not after the link.
///
/// What has served once cannot serve again, even when the attempt fails further
/// on. Otherwise a person whose exchange failed would hold a return URL that
/// still works — and that URL has by then travelled through Last.fm.
#[tokio::test]
async fn a_journey_whose_exchange_fails_is_still_spent() {
    let temp = tempfile::tempdir().unwrap();
    let config = lastfm_app(&temp);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "failing-lastfm-listener").await;
    let exchange = ExchangesFor::refusing();
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    let started = state
        .services
        .begin_lastfm_authorization(listener.owner, "default")
        .await
        .unwrap();
    let journey = state_of(&started.authorize_url);

    assert!(state
        .services
        .complete_lastfm_authorization(&journey, &started.cookie, "a-request-token")
        .await
        .is_err());
    assert_eq!(
        exchange.tokens().len(),
        1,
        "the exchange was attempted once"
    );

    // And the second attempt does not even reach it.
    assert!(state
        .services
        .complete_lastfm_authorization(&journey, &started.cookie, "a-request-token")
        .await
        .is_err());
    assert_eq!(
        exchange.tokens().len(),
        1,
        "the state was spent the first time"
    );
    assert!(state
        .services
        .scrobble_links(listener.owner)
        .await
        .unwrap()
        .is_empty());
}

/// One journey at a time: opening a second replaces the first.
///
/// The cookie carries a fixed name, so the browser could not finish the earlier
/// one anyway. Held in the database too, so "one journey" is a property of the
/// data rather than of whoever remembers to delete the previous row.
#[tokio::test]
async fn opening_a_second_journey_replaces_the_first() {
    let temp = tempfile::tempdir().unwrap();
    let config = lastfm_app(&temp);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "impatient-lastfm-listener").await;
    state.services.register_lastfm_exchange(
        "default",
        ExchangesFor::key("a-session-key")
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    let first = state
        .services
        .begin_lastfm_authorization(listener.owner, "default")
        .await
        .unwrap();
    let second = state
        .services
        .begin_lastfm_authorization(listener.owner, "default")
        .await
        .unwrap();

    assert!(state
        .services
        .complete_lastfm_authorization(
            &state_of(&first.authorize_url),
            &first.cookie,
            "a-request-token"
        )
        .await
        .is_err());
    assert!(state
        .services
        .complete_lastfm_authorization(
            &state_of(&second.authorize_url),
            &second.cookie,
            "a-request-token"
        )
        .await
        .is_ok());
}

/// An expired journey is refused, and the purge takes it.
///
/// Twelve minutes, well inside the sixty Last.fm grants its token: we refuse
/// first, and an expired token never surprises us. The purge applies *this*
/// expiry rather than the queue's thirty-day window — borrowing the wrong one
/// would keep a quarter-hour journey alive for a month.
#[tokio::test]
async fn an_expired_journey_is_refused_and_then_purged() {
    let temp = tempfile::tempdir().unwrap();
    let config = lastfm_app(&temp);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "slow-lastfm-listener").await;
    let exchange = ExchangesFor::key("a-session-key");
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );
    let started = state
        .services
        .begin_lastfm_authorization(listener.owner, "default")
        .await
        .unwrap();

    // Aged by hand, because waiting a quarter of an hour is not a test.
    sqlx::query("UPDATE lastfm_authorization SET expires_at = ?")
        .bind(now_ms() - 1)
        .execute(state.db.pool())
        .await
        .unwrap();

    assert!(state
        .services
        .complete_lastfm_authorization(
            &state_of(&started.authorize_url),
            &started.cookie,
            "a-request-token"
        )
        .await
        .is_err());
    assert!(
        exchange.tokens().is_empty(),
        "an expired journey exchanges nothing"
    );

    // The row was spent by that attempt; a journey nobody returns from is what
    // the purge is for.
    let started = state
        .services
        .begin_lastfm_authorization(listener.owner, "default")
        .await
        .unwrap();
    assert_eq!(
        state
            .services
            .purge_lastfm_authorizations(now_ms())
            .await
            .unwrap(),
        0,
        "a live journey is not the purge's business"
    );
    let expired_at = now_ms() + 13 * 60 * 1_000;
    assert_eq!(
        state
            .services
            .purge_lastfm_authorizations(expired_at)
            .await
            .unwrap(),
        1
    );
    assert!(state
        .services
        .complete_lastfm_authorization(
            &state_of(&started.authorize_url),
            &started.cookie,
            "a-request-token"
        )
        .await
        .is_err());
}

/// The return re-checks where the link would be made.
///
/// A quarter of an hour separates the two halves and a server can restart in
/// between — which is why the state lives in a database at all. Destination
/// gone or moved, the return is refused and nothing is created: otherwise the
/// journey would manufacture a link to a machine the person never chose,
/// already wrong at birth.
#[tokio::test]
async fn a_return_to_a_destination_that_moved_creates_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let config = lastfm_app(&temp);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "moved-lastfm-listener").await;
    let exchange = ExchangesFor::key("a-session-key");
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );
    let started = state
        .services
        .begin_lastfm_authorization(listener.owner, "default")
        .await
        .unwrap();
    let journey = state_of(&started.authorize_url);

    // The operator points the same name at another machine and restarts.
    let mut moved = lastfm_app(&temp);
    moved.destinations = declaring(
        &temp,
        &[(ScrobbleProvider::LastFm, "default", "http://127.0.0.1:4")],
    )
    .destinations;
    // Dropped rather than shadowed: two `AppState`s over one database file
    // would each hold their own writer gate, and this test is about what a
    // *restart* does.
    stop(state).await;
    let state = waveflow_server::initialize(&moved).await.unwrap();
    let exchange = ExchangesFor::key("a-session-key");
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    assert!(state
        .services
        .complete_lastfm_authorization(&journey, &started.cookie, "a-request-token")
        .await
        .is_err());
    assert!(
        exchange.tokens().is_empty(),
        "nothing is exchanged towards a machine nobody chose"
    );
    assert!(state
        .services
        .scrobble_links(listener.owner)
        .await
        .unwrap()
        .is_empty());
}

/// Last.fm says why it cannot be linked, rather than failing later.
///
/// Two conditions, and the listing names whichever is missing: an application
/// the operator registered, and an `https` address to bring a person back to.
/// Decision 10's plaintext escape is about the operator's own network and does
/// not reach a journey that starts on the internet.
#[tokio::test]
async fn last_fm_says_why_it_is_unavailable_instead_of_failing_later() {
    let temp = tempfile::tempdir().unwrap();

    // No application at all.
    let bare = declaring(
        &temp,
        &[(ScrobbleProvider::LastFm, "default", "http://127.0.0.1:3")],
    );
    let state = waveflow_server::initialize(&bare).await.unwrap();
    let listener = fixture(&bare, &state, "unavailable-lastfm-listener").await;
    let declared = state.services.scrobble_destinations();
    assert_eq!(declared.len(), 1);
    assert!(!declared[0].available);
    assert_eq!(
        declared[0].unavailable,
        Some(ScrobbleUnavailable::NoApplicationConfigured)
    );
    assert!(matches!(
        state
            .services
            .begin_lastfm_authorization(listener.owner, "default")
            .await
            .unwrap_err(),
        ServiceError::Unavailable
    ));
    stop(state).await;

    // An application, but a public address that is not `https`.
    let mut plaintext = lastfm_app(&temp);
    plaintext.public_url = Some("http://waveflow.example".to_owned());
    let state = waveflow_server::initialize(&plaintext).await.unwrap();
    let declared = state.services.scrobble_destinations();
    assert!(!declared[0].available);
    assert_eq!(
        declared[0].unavailable,
        Some(ScrobbleUnavailable::BrowserJourneyNeedsHttps)
    );
    assert!(matches!(
        state
            .services
            .begin_lastfm_authorization(listener.owner, "default")
            .await
            .unwrap_err(),
        ServiceError::Unavailable
    ));
    stop(state).await;

    // Both, and it is available.
    let state = waveflow_server::initialize(&lastfm_app(&temp))
        .await
        .unwrap();
    let declared = state.services.scrobble_destinations();
    assert!(declared[0].available);
    assert!(declared[0].unavailable.is_none());
}

/// The browser-less journey, and the one requirement it drops.
///
/// RFC-010 deferred this because "writing two journeys at once is two chances
/// to get the temporary state wrong". This one has no temporary state: the
/// request token is Last.fm's, it expires on Last.fm's clock, and a person
/// carries it between two commands. Nothing of ours is written down, so there
/// is no second thing to get wrong — which is what made it safe to write
/// second.
///
/// **No `WAVEFLOW_PUBLIC_URL`.** The web journey needs one because Last.fm has
/// to bring a browser back to an address this server answers; nothing comes
/// back here. A headless server is exactly the deployment that has no such
/// address, and exactly the one this journey is for.
#[tokio::test]
async fn a_journey_without_a_browser_needs_no_public_address() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = lastfm_app(&temp);
    config.public_url = None;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "headless-listener").await;
    let exchange = ExchangesFor::key("a-session-key");
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    // The web journey cannot even start here, which is the contrast.
    assert!(matches!(
        state
            .services
            .begin_lastfm_authorization(listener.owner, "default")
            .await,
        Err(ServiceError::Unavailable)
    ));

    let approval = state
        .services
        .begin_lastfm_approval("default")
        .await
        .unwrap();
    assert_eq!(exchange.mintings(), 1);
    assert!(approval
        .authorize_url
        .starts_with("https://www.last.fm/api/auth/"));
    // The address carries the token, which is why neither of them is ever
    // logged: whoever opens it first is who the session belongs to.
    assert!(approval.authorize_url.contains(&approval.token));

    state
        .services
        .complete_lastfm_approval(listener.owner, "default", &approval.token)
        .await
        .unwrap();

    // The token exchanged is the one that was minted, and there is only one.
    assert_eq!(exchange.tokens(), vec![approval.token]);
    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].provider, ScrobbleProvider::LastFm);
    assert_eq!(links[0].destination, "default");
}

/// The web journey never asks for a token of its own.
///
/// This is the mistake `src/scrobblers/lastfm.rs` warns about in as many
/// words: the two halves of Last.fm's protocol resemble each other enough to
/// be mixed up, and a server that minted a token and then received a different
/// one would hold the wrong one. Until `auth.getToken` existed there was
/// nothing to count; now there is, and the count is zero.
#[tokio::test]
async fn the_journey_with_a_browser_mints_no_token_of_its_own() {
    let temp = tempfile::tempdir().unwrap();
    let config = lastfm_app(&temp);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "no-minting-listener").await;
    let exchange = ExchangesFor::key("a-session-key");
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    let started = state
        .services
        .begin_lastfm_authorization(listener.owner, "default")
        .await
        .unwrap();
    let journey = state_of(&started.authorize_url);
    state
        .services
        .complete_lastfm_authorization(&journey, &started.cookie, "the-token-last-fm-appended")
        .await
        .unwrap();

    assert_eq!(
        exchange.tokens(),
        vec!["the-token-last-fm-appended".to_owned()],
        "the token exchanged is the one Last.fm handed back"
    );
    assert_eq!(
        exchange.mintings(),
        0,
        "and this journey asks Last.fm for no token of its own"
    );
}

/// An instance this server does not offer is refused before anything leaves.
///
/// A token minted for a name no link could be made against is a call made for
/// nothing, and it would be made against a real Last.fm application.
#[tokio::test]
async fn an_approval_for_an_undeclared_instance_asks_last_fm_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let config = lastfm_app(&temp);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let exchange = ExchangesFor::key("a-session-key");
    state.services.register_lastfm_exchange(
        "default",
        std::sync::Arc::clone(&exchange)
            as std::sync::Arc<dyn waveflow_server::services::LastFmSessionExchange>,
    );

    assert!(matches!(
        state
            .services
            .begin_lastfm_approval("nobody-declared-me")
            .await,
        Err(ServiceError::NotFound)
    ));
    assert_eq!(exchange.mintings(), 0);
}
