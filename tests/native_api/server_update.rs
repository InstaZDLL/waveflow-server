//! The update check (RFC-028): nothing is sent until it is on, an
//! administrator decides under `ask`, and a manifest is read as a verdict.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use waveflow_server::config::{Config, UpdateCheckPolicy};

use super::*;

/// The manifests a fake download host serves, and how many it was asked for.
#[derive(Clone, Default)]
struct Host {
    answers: Arc<Mutex<std::collections::HashMap<String, (StatusCode, String)>>>,
    requests: Arc<AtomicUsize>,
    /// Holds every answer back far longer than any check waits.
    stalled: Arc<std::sync::atomic::AtomicBool>,
}

impl Host {
    async fn start() -> (Self, String) {
        let host = Self::default();
        let served = host.clone();
        let router = axum::Router::new().route(
            "/{file}",
            axum::routing::get(
                move |axum::extract::Path(file): axum::extract::Path<String>| {
                    let served = served.clone();
                    async move {
                        served.requests.fetch_add(1, Ordering::SeqCst);
                        if served.stalled.load(Ordering::SeqCst) {
                            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        }
                        served
                            .answers
                            .lock()
                            .unwrap()
                            .get(&file)
                            .cloned()
                            .unwrap_or((StatusCode::NOT_FOUND, String::new()))
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (host, format!("http://{address}"))
    }

    fn serve(&self, file: &str, status: StatusCode, body: String) {
        self.answers
            .lock()
            .unwrap()
            .insert(file.to_owned(), (status, body));
    }

    fn serve_version(&self, file: &str, version: &str) {
        self.serve(
            file,
            StatusCode::OK,
            serde_json::json!({
                "version": version,
                "tag": format!("v{version}"),
                "published_at": "2026-11-06T06:53:33Z",
                "page": format!("https://github.com/InstaZDLL/waveflow-server/releases/tag/v{version}"),
                "notes": format!("https://download.waveflow.app/server/releases/v{version}/RELEASE-NOTES.md"),
            })
            .to_string(),
        );
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

/// An instance under `policy`, its manifests at `base_url`, with an
/// administrator and a listener who authenticate by API token.
struct Instance {
    _temp: tempfile::TempDir,
    state: waveflow_server::AppState,
    router: axum::Router,
    admin_token: String,
    listener_token: String,
}

impl Instance {
    async fn new(policy: UpdateCheckPolicy, base_url: &str) -> Self {
        Self::with_timeout(policy, base_url, std::time::Duration::from_secs(5)).await
    }

    async fn with_timeout(
        policy: UpdateCheckPolicy,
        base_url: &str,
        timeout: std::time::Duration,
    ) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let mut config = Config::for_data_dir(temp.path().join("data"));
        config.update_check.policy = policy;
        config.update_check.base_url = base_url.to_owned();
        config.update_check.timeout = timeout;
        let state = waveflow_server::initialize(&config).await.unwrap();
        // Inserted rather than created, so no literal reaches a cryptographic
        // call: these accounts authenticate by API token.
        let admin = uuid::Uuid::new_v4();
        let listener = uuid::Uuid::new_v4();
        for (id, username, role) in [
            (admin, "update-admin", "admin"),
            (listener, "update-listener", "user"),
        ] {
            sqlx::query(
                "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
                 VALUES (?, ?, 'this-account-never-authenticates', ?, 0, ?, ?)",
            )
            .bind(id.to_string())
            .bind(username)
            .bind(role)
            .bind(now_ms())
            .bind(now_ms())
            .execute(state.db.pool())
            .await
            .unwrap();
        }
        let (_record, admin_token) = state
            .services
            .create_api_token(admin, &[], "update-admin", "updates", &[])
            .await
            .unwrap();
        let (_record, listener_token) = state
            .services
            // Minted by the administrator: only one mints a token here.
            .create_api_token(admin, &[], "update-listener", "updates", &[])
            .await
            .unwrap();
        let router = waveflow_server::app(&config, state.clone());
        Self {
            _temp: temp,
            state,
            router,
            admin_token,
            listener_token,
        }
    }

    async fn call(
        &self,
        method: Method,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = Request::builder()
            .method(method)
            .uri("/api/v2/server/update")
            .header("authorization", format!("Bearer {token}"));
        let body = match body {
            Some(value) => {
                request = request.header("content-type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    async fn status(&self) -> serde_json::Value {
        let (status, body) = self.call(Method::GET, &self.admin_token, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }

    async fn answer(&self, enabled: bool) -> (StatusCode, serde_json::Value) {
        self.call(
            Method::PUT,
            &self.admin_token,
            Some(serde_json::json!({ "enabled": enabled })),
        )
        .await
    }

    async fn check(&self) -> bool {
        self.state.services.check_for_update().await.unwrap()
    }
}

const NEWER: &str = "99.0.0";

#[tokio::test]
async fn nothing_is_sent_until_an_administrator_answers() {
    let (host, base_url) = Host::start().await;
    host.serve_version("stable.json", NEWER);
    let instance = Instance::new(UpdateCheckPolicy::Ask, &base_url).await;

    assert!(!instance.check().await, "no request before an answer");
    assert_eq!(host.requests(), 0);
    let status = instance.status().await;
    assert_eq!(status["policy"], "ask");
    assert_eq!(status["decision"], serde_json::Value::Null);
    assert_eq!(status["enabled"], false);
    assert_eq!(status["latest"], serde_json::Value::Null);
    assert_eq!(status["update_available"], false);
    assert_eq!(status["current"], env!("CARGO_PKG_VERSION"));
}

#[tokio::test]
async fn a_yes_checks_and_reports_a_newer_version() {
    let (host, base_url) = Host::start().await;
    host.serve_version("stable.json", NEWER);
    host.serve_version("beta.json", "100.0.0-rc.1");
    let instance = Instance::new(UpdateCheckPolicy::Ask, &base_url).await;

    let (code, answered) = instance.answer(true).await;
    assert_eq!(code, StatusCode::OK, "{answered}");
    assert_eq!(answered["decision"], true);
    assert_eq!(answered["enabled"], true);

    assert!(instance.check().await);
    // A stable server reads stable.json, and only it.
    assert_eq!(host.requests(), 1);
    let status = instance.status().await;
    assert_eq!(status["channel"], "stable");
    assert_eq!(status["latest"]["version"], NEWER);
    assert_eq!(status["latest"]["tag"], format!("v{NEWER}"));
    assert_eq!(status["update_available"], true);
    assert_eq!(status["failure"], serde_json::Value::Null);
    assert!(status["checked_at"].as_i64().is_some());
}

#[tokio::test]
async fn the_running_version_or_an_older_one_is_not_an_update() {
    let (host, base_url) = Host::start().await;
    let instance = Instance::new(UpdateCheckPolicy::On, &base_url).await;
    for version in [env!("CARGO_PKG_VERSION"), "2.0.0"] {
        host.serve_version("stable.json", version);
        assert!(instance.check().await);
        let status = instance.status().await;
        assert_eq!(status["latest"]["version"], version);
        assert_eq!(status["update_available"], false, "{version}");
    }
}

#[tokio::test]
async fn a_no_stops_the_check_and_hides_what_was_read() {
    let (host, base_url) = Host::start().await;
    host.serve_version("stable.json", NEWER);
    let instance = Instance::new(UpdateCheckPolicy::Ask, &base_url).await;
    instance.answer(true).await;
    assert!(instance.check().await);
    assert_eq!(instance.status().await["update_available"], true);

    let (code, answered) = instance.answer(false).await;
    assert_eq!(code, StatusCode::OK, "{answered}");
    assert_eq!(answered["decision"], false);
    assert_eq!(answered["enabled"], false);
    assert_eq!(answered["latest"], serde_json::Value::Null);
    assert_eq!(answered["update_available"], false);

    assert!(!instance.check().await);
    assert_eq!(host.requests(), 1, "nothing sent after the no");
}

#[tokio::test]
async fn the_environment_decides_under_on_and_off() {
    let (host, base_url) = Host::start().await;
    host.serve_version("stable.json", NEWER);

    let on = Instance::new(UpdateCheckPolicy::On, &base_url).await;
    assert!(on.check().await, "on needs no answer");
    let status = on.status().await;
    assert_eq!(status["policy"], "on");
    assert_eq!(status["decision"], serde_json::Value::Null);
    assert_eq!(status["update_available"], true);
    assert_eq!(on.answer(false).await.0, StatusCode::CONFLICT);

    let off = Instance::new(UpdateCheckPolicy::Off, &base_url).await;
    assert_eq!(off.answer(true).await.0, StatusCode::CONFLICT);
    assert!(!off.check().await);
    let status = off.status().await;
    assert_eq!(status["policy"], "off");
    assert_eq!(status["enabled"], false);
    assert_eq!(host.requests(), 1, "only the instance that is on asked");
}

#[tokio::test]
async fn a_listener_is_refused_both_routes() {
    let (_host, base_url) = Host::start().await;
    let instance = Instance::new(UpdateCheckPolicy::Ask, &base_url).await;
    let (code, _) = instance
        .call(Method::GET, &instance.listener_token, None)
        .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let (code, _) = instance
        .call(
            Method::PUT,
            &instance.listener_token,
            Some(serde_json::json!({ "enabled": true })),
        )
        .await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    assert_eq!(
        instance.status().await["decision"],
        serde_json::Value::Null,
        "the listener's yes was not recorded"
    );
}

#[tokio::test]
async fn a_bad_answer_is_kept_as_a_verdict() {
    let (host, base_url) = Host::start().await;
    let instance = Instance::new(UpdateCheckPolicy::On, &base_url).await;
    let javascript = serde_json::json!({
        "version": NEWER,
        "tag": format!("v{NEWER}"),
        "published_at": "2026-11-06T06:53:33Z",
        "page": "javascript:alert(1)",
        "notes": null,
    })
    .to_string();
    let cases = [
        ("status", StatusCode::INTERNAL_SERVER_ERROR, "{}".to_owned()),
        ("invalid", StatusCode::OK, "not json".to_owned()),
        ("invalid", StatusCode::OK, javascript),
        ("unreadable", StatusCode::OK, "x".repeat(3 * 1024 * 1024)),
    ];
    for (verdict, status, body) in cases {
        host.serve("stable.json", status, body);
        assert!(instance.check().await);
        let reported = instance.status().await;
        assert_eq!(reported["failure"], verdict);
        assert_eq!(reported["latest"], serde_json::Value::Null, "{verdict}");
        assert_eq!(reported["update_available"], false);
    }

    // A good answer clears the verdict; a later failure keeps what it read.
    host.serve_version("stable.json", NEWER);
    assert!(instance.check().await);
    assert_eq!(instance.status().await["failure"], serde_json::Value::Null);
    host.serve(
        "stable.json",
        StatusCode::SERVICE_UNAVAILABLE,
        String::new(),
    );
    assert!(instance.check().await);
    let reported = instance.status().await;
    assert_eq!(reported["failure"], "status");
    assert_eq!(reported["latest"]["version"], NEWER);
}

#[tokio::test]
async fn an_unreachable_host_is_a_verdict() {
    // The test configuration's closed loopback port.
    let instance = Instance::new(UpdateCheckPolicy::On, "http://127.0.0.1:9").await;
    assert!(instance.check().await);
    assert_eq!(instance.status().await["failure"], "unreachable");
}

#[tokio::test]
async fn a_host_that_never_answers_is_given_up_on() {
    let (host, base_url) = Host::start().await;
    host.serve_version("stable.json", NEWER);
    host.stalled.store(true, Ordering::SeqCst);
    let instance = Instance::with_timeout(
        UpdateCheckPolicy::On,
        &base_url,
        std::time::Duration::from_millis(500),
    )
    .await;
    let started = std::time::Instant::now();
    assert!(instance.check().await);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(10),
        "the check waited {:?}",
        started.elapsed()
    );
    assert_eq!(instance.status().await["failure"], "unreachable");
}
