//! The limit on failed sign-ins, per address and per account, on both surfaces.

use super::*;

// ── Guarding sign-in against guessing (RFC-017) ──
/// An account to guess at, with a password and a Subsonic credential drawn at
/// run time (for CodeQL), and the router serving it.
struct Guarded {
    router: axum::Router,
    password: String,
    /// A wrong password, drawn at run time too.
    wrong: String,
    subsonic_secret: String,
    api_key: String,
}

async fn guarded(trusted_proxies: &[&str]) -> (tempfile::TempDir, Guarded) {
    let (temp, config, mut state) = test_app().await;
    // Enough permits for every concurrent guess the tests make, whatever the
    // machine: the gate is the gate's tests' subject, not these.
    state.auth = state.auth.clone().with_sign_in_permits(64);
    let password = Uuid::new_v4().to_string();
    let password_hash = security::hash_password(&password).unwrap();
    let now = now_ms();
    let user = state
        .db
        .create_account("Guarded", &password_hash, AccountRole::User, now)
        .await
        .unwrap();
    let subsonic_secret = Uuid::new_v4().simple().to_string();
    let api_key = format!("wfsk_{}", Uuid::new_v4().simple());
    let encrypted = state
        .secret_box
        .encrypt(subsonic_secret.as_bytes())
        .unwrap();
    state
        .db
        .set_subsonic_credential(user, user, &encrypted, &security::token_hash(&api_key), now)
        .await
        .unwrap();
    state.trusted_proxies = trusted_proxies
        .iter()
        .map(|net| net.parse::<ipnet::IpNet>().unwrap())
        .collect::<Vec<_>>()
        .into();
    let router = waveflow_server::app(&config, state);
    (
        temp,
        Guarded {
            router,
            password,
            wrong: Uuid::new_v4().to_string(),
            subsonic_secret,
            api_key,
        },
    )
}

/// `request`, as if it came from `peer`, forwarding `forwarded` if given.
fn from_peer(mut request: Request<Body>, peer: &str, forwarded: Option<&str>) -> Request<Body> {
    let address = std::net::SocketAddr::new(peer.parse().unwrap(), 40_000);
    request
        .extensions_mut()
        .insert(axum::extract::ConnectInfo(address));
    if let Some(forwarded) = forwarded {
        request
            .headers_mut()
            .insert("x-forwarded-for", forwarded.parse().unwrap());
    }
    request
}

async fn native_sign_in(
    router: &axum::Router,
    username: &str,
    password: &str,
    peer: &str,
    forwarded: Option<&str>,
) -> axum::response::Response {
    let request = json_request(
        "/api/v2/auth/login",
        serde_json::json!({
            "username": username,
            "password": password,
            "device_name": "Guessing machine"
        }),
    );
    router
        .clone()
        .oneshot(from_peer(request, peer, forwarded))
        .await
        .unwrap()
}

/// The Subsonic façade's verdict: `Ok(())`, or the error code.
async fn subsonic_sign_in(router: &axum::Router, credentials: &str, peer: &str) -> Result<(), i64> {
    let request = Request::get(format!(
        "/rest/ping.view?{credentials}&v=1.16.1&c=guess&f=json"
    ))
    .body(Body::empty())
    .unwrap();
    let response = router
        .clone()
        .oneshot(from_peer(request, peer, None))
        .await
        .unwrap();
    let body = json_body(response).await;
    let answer = &body["subsonic-response"];
    if answer["status"] == "ok" {
        Ok(())
    } else {
        Err(answer["error"]["code"].as_i64().unwrap())
    }
}

/// RFC-017 decision 1 on the native and web sign-ins: twenty failures from one
/// address and the twenty-first is refused unchecked with `429` and
/// `Retry-After` — the right password included — while the owner, from
/// elsewhere, signs in. The web sign-in shares the limiter.
#[tokio::test]
async fn failed_sign_ins_are_limited_per_address_and_account() {
    let (_temp, app) = guarded(&[]).await;
    for _ in 0..waveflow_server::sign_in::FAILURES_PER_WINDOW {
        let refused =
            native_sign_in(&app.router, "Guarded", &app.wrong, "198.51.100.7", None).await;
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    }
    let throttled =
        native_sign_in(&app.router, "Guarded", &app.password, "198.51.100.7", None).await;
    assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(throttled.headers()["retry-after"], "60");
    // The case the database ignores, the limiter ignores too, and the spaces
    // around the name.
    let throttled =
        native_sign_in(&app.router, "guarded", &app.password, "198.51.100.7", None).await;
    assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);
    let throttled = native_sign_in(
        &app.router,
        " Guarded ",
        &app.password,
        "198.51.100.7",
        None,
    )
    .await;
    assert_eq!(throttled.status(), StatusCode::TOO_MANY_REQUESTS);

    let elsewhere =
        native_sign_in(&app.router, "Guarded", &app.password, "203.0.113.9", None).await;
    assert_eq!(elsewhere.status(), StatusCode::OK);

    let mut web = json_request(
        "/api/v2/web/auth/login",
        serde_json::json!({
            "username": "Guarded",
            "password": &app.password,
            "device_name": "Browser"
        }),
    );
    web.headers_mut()
        .insert("origin", "http://waveflow.test".parse().unwrap());
    web.headers_mut()
        .insert("host", "waveflow.test".parse().unwrap());
    let web = app
        .router
        .clone()
        .oneshot(from_peer(web, "198.51.100.7", None))
        .await
        .unwrap();
    assert_eq!(web.status(), StatusCode::TOO_MANY_REQUESTS);
}

/// The attempt is reserved before it is checked: forty concurrent guesses from
/// one address get twenty verdicts and twenty refusals, not forty verdicts.
#[tokio::test]
async fn concurrent_guesses_cannot_slip_past_the_limit() {
    let (_temp, app) = guarded(&[]).await;
    let guesses =
        (0..40).map(|_| native_sign_in(&app.router, "Guarded", &app.wrong, "198.51.100.7", None));
    let statuses: Vec<StatusCode> = futures_util::future::join_all(guesses)
        .await
        .into_iter()
        .map(|response| response.status())
        .collect();
    let checked = statuses
        .iter()
        .filter(|status| **status == StatusCode::UNAUTHORIZED)
        .count();
    let throttled = statuses
        .iter()
        .filter(|status| **status == StatusCode::TOO_MANY_REQUESTS)
        .count();
    assert_eq!(checked, waveflow_server::sign_in::FAILURES_PER_WINDOW);
    assert_eq!(
        throttled,
        40 - waveflow_server::sign_in::FAILURES_PER_WINDOW
    );
}

/// RFC-017 decision 3: an unknown account costs an Argon2 verification too, so
/// its refusal no longer comes back in a fraction of the time a real one's
/// does. Medians of three, each from its own address.
#[tokio::test]
async fn an_unknown_account_costs_a_verification() {
    let (_temp, app) = guarded(&[]).await;
    let mut known = Vec::new();
    let mut unknown = Vec::new();
    for round in 0..3 {
        let started = std::time::Instant::now();
        let refused = native_sign_in(
            &app.router,
            "Guarded",
            &app.wrong,
            &format!("192.0.2.{round}"),
            None,
        )
        .await;
        known.push(started.elapsed());
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
        let started = std::time::Instant::now();
        let refused = native_sign_in(
            &app.router,
            "nobody-by-that-name",
            &app.wrong,
            &format!("192.0.2.{}", round + 10),
            None,
        )
        .await;
        unknown.push(started.elapsed());
        assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    }
    known.sort();
    unknown.sort();
    assert!(
        unknown[1] * 2 >= known[1],
        "an unknown account answered in {:?}, a real one in {:?}",
        unknown[1],
        known[1]
    );
}

/// The Subsonic façade shares the limiter: per address and account, with the
/// username's case folded, so the owner's clients elsewhere keep working —
/// the lockout a username-only window allowed is gone. Its error code stays 40.
#[tokio::test]
async fn subsonic_failures_are_limited_per_address_and_case_folded() {
    let (_temp, app) = guarded(&[]).await;
    for index in 0..waveflow_server::sign_in::FAILURES_PER_WINDOW {
        let name = if index % 2 == 0 { "Guarded" } else { "GUARDED" };
        assert_eq!(
            subsonic_sign_in(&app.router, &format!("u={name}&p=wrong"), "198.51.100.7").await,
            Err(40)
        );
    }
    let right = format!("u=guarded&p={}", app.subsonic_secret);
    assert_eq!(
        subsonic_sign_in(&app.router, &right, "198.51.100.7").await,
        Err(40)
    );
    assert_eq!(
        subsonic_sign_in(&app.router, &right, "203.0.113.9").await,
        Ok(())
    );
}

/// An API key is never throttled: more than twenty invalid keys from one
/// address, distinct and repeated, and the valid key from that same address is
/// still accepted.
#[tokio::test]
async fn api_keys_are_never_throttled() {
    let (_temp, app) = guarded(&[]).await;
    for index in 0..25 {
        let guess = format!("wfsk_guess-{}", index % 7);
        assert_eq!(
            subsonic_sign_in(&app.router, &format!("apiKey={guess}"), "198.51.100.7").await,
            Err(40)
        );
    }
    assert_eq!(
        subsonic_sign_in(
            &app.router,
            &format!("apiKey={}", app.api_key),
            "198.51.100.7"
        )
        .await,
        Ok(())
    );
}

/// RFC-017 decision 2: behind a trusted proxy the client is read from the
/// right of `X-Forwarded-For`, so a client that prefixes a forged address still
/// counts as itself; from an untrusted peer the header is ignored, so changing
/// it buys no fresh budget.
#[tokio::test]
async fn a_trusted_proxy_names_the_client_and_a_stranger_cannot() {
    let (_temp, app) = guarded(&["10.0.0.0/8"]).await;
    let sign_in = |password: String, peer: &'static str, forwarded: String| {
        let router = app.router.clone();
        async move {
            native_sign_in(&router, "Guarded", &password, peer, Some(&forwarded))
                .await
                .status()
        }
    };
    for index in 0..waveflow_server::sign_in::FAILURES_PER_WINDOW {
        let forwarded = format!("192.0.2.{index}, 198.51.100.7");
        assert_eq!(
            sign_in(app.wrong.clone(), "10.0.0.2", forwarded).await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        sign_in(
            app.password.clone(),
            "10.0.0.2",
            "1.1.1.1, 198.51.100.7".into()
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        sign_in(app.password.clone(), "10.0.0.2", "203.0.113.9".into()).await,
        StatusCode::OK
    );

    for index in 0..waveflow_server::sign_in::FAILURES_PER_WINDOW {
        assert_eq!(
            sign_in(
                app.wrong.clone(),
                "192.0.2.200",
                format!("203.0.113.{index}")
            )
            .await,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        sign_in(app.password.clone(), "192.0.2.200", "203.0.113.250".into()).await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

/// A writer for the log, kept so the test can read it back.
#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The log of this whole test binary, at every level: global for the reason
/// `tests/enrichment/lastfm.rs` gives, and read back only for lines no other test
/// here writes.
fn captured_log() -> &'static Captured {
    static LOG: std::sync::OnceLock<Captured> = std::sync::OnceLock::new();
    LOG.get_or_init(|| {
        let captured = Captured::default();
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_writer(captured.clone())
            .with_ansi(false)
            .init();
        captured
    })
}

/// A stored hash that cannot be read is the server's fault: every attempt
/// answers `503` and gives its slot back, so the limiter never stops a burst
/// against that account. The log must, then — one `warn` line however many
/// attempts, the rest counted for the minute's summary (RFC-017: nothing per
/// attempt above `debug`).
#[tokio::test]
async fn an_unreadable_hash_is_not_a_line_per_attempt() {
    let captured = captured_log();
    let (_temp, config, state) = test_app().await;
    state
        .db
        .create_account("Damaged", "not-a-phc-string", AccountRole::User, now_ms())
        .await
        .unwrap();
    let router = waveflow_server::app(&config, state);

    let password = Uuid::new_v4().to_string();
    for _ in 0..waveflow_server::sign_in::FAILURES_PER_WINDOW + 5 {
        let refused = native_sign_in(&router, "Damaged", &password, "198.51.100.40", None).await;
        assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    let log = String::from_utf8_lossy(&captured.0.lock().unwrap()).into_owned();
    let warnings: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(" WARN ") && line.contains("password hash"))
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:#?}");
}
