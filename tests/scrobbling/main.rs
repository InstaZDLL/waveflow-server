//! The queue a listen leaves by. RFC-010.
//!
//! No routes yet: this drives `DomainServices` directly, which is the whole of
//! what the domain half of scrobbling is. What is queued, what is never queued,
//! what each verdict does to a row and what unlinking does to a queue are
//! decided there rather than at a surface, so they are tested there.
//!
//! **Most destinations here are doubles**, implementing [`ScrobbleTarget`]
//! directly — which is the point of the trait: the drain knows five words and no
//! providers, so the five words are what those tests drive.
//!
//! **Every test that awaits a [`Destination`] is not.** Those stand a real HTTP
//! server on loopback and let the ListenBrainz adapter talk to it, because what
//! they check exists only on the wire: the `Token` scheme, the JSON shape, the
//! seconds-not-milliseconds timestamp, the header a `429` names its delay in,
//! and what a status line does to a row. A double would prove none of it. They
//! reach `127.0.0.1` and nothing else — no test in this file touches a network
//! this machine does not own.
//!
//! Stated as a rule rather than a list on purpose. This paragraph named three
//! tests and was wrong within the hour, because the list went stale the moment
//! another one was added — twice. **Then the rule itself went stale**: it said
//! "names `spawn_destination`" until a second constructor appeared beside it, so
//! it now names the *type* both return. A function can be joined by a sibling;
//! the thing the test holds cannot.
//!
//! The accounts below are inserted with a placeholder in `password_hash` rather
//! than hashed from a literal. They never log in — this target has no HTTP
//! surface to log in to — and the repository's usual fixture password would add
//! another occurrence of the literal that the `main` ruleset blocks a pull
//! request for.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::future::BoxFuture;
use tower::ServiceExt as _;
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::config::ScrobbleLimits;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::services::{
    ScrobbleEnvelope, ScrobbleProvider, ScrobbleTarget, ScrobbleUnavailable, ScrobbleVerdict,
    ServiceError,
};
use waveflow_server::AppState;

// Not every target uses every fixture, and a shared module is not dead code
// for being partly unused here.
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;
use support::*;

mod adapters;
mod drain;
mod instances;
mod lastfm_journey;
mod listenbrainz;
mod queue;
mod retention;
mod uncertain;

/// A destination that answers what a test told it to, and remembers what it was
/// asked.
struct Recorder {
    answers: Mutex<VecDeque<ScrobbleVerdict>>,
    fallback: ScrobbleVerdict,
    seen: Mutex<Vec<(ScrobbleEnvelope, String)>>,
}

impl Recorder {
    fn always(fallback: ScrobbleVerdict) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            answers: Mutex::new(VecDeque::new()),
            fallback,
            seen: Mutex::new(Vec::new()),
        })
    }

    /// Every submission it was handed, with the secret it was handed alongside.
    fn seen(&self) -> Vec<(ScrobbleEnvelope, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl ScrobbleTarget for Recorder {
    fn submit<'a>(
        &'a self,
        envelope: &'a ScrobbleEnvelope,
        secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async move {
            self.seen
                .lock()
                .unwrap()
                .push((envelope.clone(), secret.to_owned()));
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(self.fallback)
        })
    }
}

struct Fixture {
    owner: uuid::Uuid,
    /// A track carrying a title, two credited artists, an album and an album
    /// artist — everything an envelope has room for.
    tagged: uuid::Uuid,
    /// A track carrying none of it, which decision 7 says is never submitted.
    bare: uuid::Uuid,
}

/// An account, a library, and the two tracks every test here needs.
async fn fixture(config: &waveflow_server::Config, state: &AppState, name: &str) -> Fixture {
    let owner = uuid::Uuid::new_v4();
    let now = now_ms();
    // Inserted rather than created: see the module comment. The value is not a
    // hash of anything and nothing ever verifies against it.
    sqlx::query(
        "INSERT INTO account (id, username, password_hash, role, disabled, created_at, updated_at) \
         VALUES (?, ?, 'this-account-never-authenticates', 'admin', 0, ?, ?)",
    )
    .bind(owner.to_string())
    .bind(name)
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();

    let music = config.data_dir.join(name);
    std::fs::create_dir_all(&music).unwrap();
    generate_audio_fixture(&music.join("tagged.flac"), "flac", "flac");
    write_test_wav(&music.join("bare.wav"));
    let root = std::fs::canonicalize(&music).unwrap();
    let library = state
        .db
        .create_library(owner, name, &root, LibraryVisibility::Private, now)
        .await
        .unwrap();
    run_scan(
        state,
        owner,
        LibraryRecord {
            id: library,
            name: name.into(),
            root_path: root,
        },
    )
    .await;

    let tagged = sqlx::query_scalar::<_, String>(
        "SELECT id FROM track WHERE library_id=? AND id IN (SELECT track_id FROM track_participant)",
    )
    .bind(library.to_string())
    .fetch_one(state.db.pool())
    .await
    .expect("the tagged file must have been scanned with its credits");
    let bare = sqlx::query_scalar::<_, String>(
        "SELECT id FROM track WHERE library_id=? AND id NOT IN \
         (SELECT track_id FROM track_participant)",
    )
    .bind(library.to_string())
    .fetch_one(state.db.pool())
    .await
    .expect("the untagged file must have been scanned with no credit at all");

    Fixture {
        owner,
        tagged: tagged.parse().unwrap(),
        bare: bare.parse().unwrap(),
    }
}

/// Every outbox row, as `(id, state, attempts, retry_of)`.
async fn rows(state: &AppState) -> Vec<(i64, String, i64, Option<i64>)> {
    sqlx::query_as::<_, (i64, String, i64, Option<i64>)>(
        "SELECT id, state, attempts, retry_of FROM scrobble_outbox ORDER BY id",
    )
    .fetch_all(state.db.pool())
    .await
    .unwrap()
}

/// The public name of every row, in the same order as [`rows`].
///
/// Apart from `rows` rather than folded into it: the rowid is what `retry_of`
/// and the ordering speak in, so the assertions about the shape of the queue
/// keep reading that, while the two gestures a person makes are named the way
/// the outside will name them.
async fn public_ids(state: &AppState) -> Vec<uuid::Uuid> {
    sqlx::query_scalar::<_, String>("SELECT public_id FROM scrobble_outbox ORDER BY id")
        .fetch_all(state.db.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|id| id.parse().unwrap())
        .collect()
}

/// Brings every waiting row's next attempt forward, so a test can exhaust the
/// retries without waiting out the growing delay the server really applies.
async fn make_everything_due(state: &AppState) {
    sqlx::query("UPDATE scrobble_outbox SET next_attempt_at=0 WHERE state='pending'")
        .execute(state.db.pool())
        .await
        .unwrap();
}

/// Drives one listen to `uncertain`, which three tests below start from.
async fn one_uncertain_entry(state: &AppState, fixture: &Fixture) -> uuid::Uuid {
    let target = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(rows(state).await[0].1, "uncertain");
    public_ids(state).await[0]
}

/// A stand-in for ListenBrainz, on loopback, that answers what a test told it
/// to and remembers what it was asked.
///
/// A real socket rather than a double of the trait: these tests are the only
/// ones that exercise the adapter itself — the header, the JSON on the wire,
/// and the status-to-verdict mapping — and a double would prove none of it.
struct Destination {
    base: String,
    bodies: std::sync::Arc<Mutex<Vec<serde_json::Value>>>,
    authorizations: std::sync::Arc<Mutex<Vec<String>>>,
}

async fn spawn_destination(status: u16, reset_in: Option<u64>) -> Destination {
    spawn_delaying(
        status,
        reset_in.map(|seconds| ("x-ratelimit-reset-in", seconds.to_string())),
    )
    .await
}

/// The same destination, naming its delay in a header of the caller's choosing.
///
/// Two headers carry that answer: ListenBrainz's own, and the standard
/// `Retry-After` that an nginx or a CDN in front of a self-hosted instance
/// replies with instead. A helper that can only send the first cannot tell
/// whether the adapter reads the second — and it did not, for the length of a
/// branch, while every rate-limit test here stayed green.
async fn spawn_delaying(status: u16, delay: Option<(&'static str, String)>) -> Destination {
    let bodies = std::sync::Arc::new(Mutex::new(Vec::new()));
    let authorizations = std::sync::Arc::new(Mutex::new(Vec::new()));
    // Two paths, one recorder. ListenBrainz submits to the first and Maloja to
    // the second, and a test that means to exercise one destination has no
    // reason to also stand up a second server for it.
    let handler = |bodies: std::sync::Arc<Mutex<Vec<serde_json::Value>>>,
                   authorizations: std::sync::Arc<Mutex<Vec<String>>>,
                   delay: Option<(&'static str, String)>| {
        axum::routing::post(move |headers: axum::http::HeaderMap, body: String| {
            let bodies = std::sync::Arc::clone(&bodies);
            let authorizations = std::sync::Arc::clone(&authorizations);
            let delay = delay.clone();
            async move {
                authorizations.lock().unwrap().push(
                    headers
                        .get(axum::http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                );
                bodies
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&body).expect("the adapter must send JSON"));
                let mut response = axum::response::Response::builder()
                    .status(axum::http::StatusCode::from_u16(status).unwrap());
                if let Some((header, value)) = delay {
                    response = response.header(header, value);
                }
                response.body(axum::body::Body::from("{}")).unwrap()
            }
        })
    };
    let router = axum::Router::new()
        .route(
            "/1/submit-listens",
            handler(
                std::sync::Arc::clone(&bodies),
                std::sync::Arc::clone(&authorizations),
                delay.clone(),
            ),
        )
        .route(
            "/apis/mlj_1/newscrobble",
            handler(
                std::sync::Arc::clone(&bodies),
                std::sync::Arc::clone(&authorizations),
                delay,
            ),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Destination {
        base: format!("http://{address}"),
        bodies,
        authorizations,
    }
}

/// Stops an instance the way a restart does: every connection closed before
/// the next boot opens the file.
///
/// Dropping the state is not that. sqlx closes a dropped pool's connections on
/// their own threads after the drop has returned, and the last of them
/// checkpoints the WAL under an exclusive lock. The next `initialize` raced
/// that close, and on a loaded Windows runner it outwaited `busy_timeout` and
/// failed with `database is locked`. `cli.rs` closes before it restarts for
/// the same reason.
async fn stop(state: AppState) {
    state.db.pool().close().await;
    drop(state);
}

/// A config over `temp`'s data directory declaring exactly these instances.
///
/// Called twice over one `TempDir` in the tests below, which is how a restart
/// is played — closed with [`stop`] in between: the second `initialize` opens
/// the same database under a different configuration, which is the only moment
/// reconciliation runs.
fn declaring(
    temp: &tempfile::TempDir,
    declared: &[(ScrobbleProvider, &str, &str)],
) -> waveflow_server::Config {
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.destinations = declared
        .iter()
        .map(|(provider, name, base)| {
            let url = waveflow_server::scrobblers::validate_destination(base, true)
                .expect("a destination the suite chose");
            let fingerprint = waveflow_server::scrobblers::destination_fingerprint(&url);
            waveflow_server::config::ScrobbleDestination {
                provider: *provider,
                name: (*name).to_owned(),
                url,
                fingerprint,
            }
        })
        .collect();
    config
}

/// A Last.fm application and an `https` address, which is what the journey
/// needs beyond a declared destination.
///
/// The address is never dialled — no test here leaves the process — but it is
/// what the callback URL is built from, and what decides whether Last.fm is
/// available at all.
fn lastfm_app(temp: &tempfile::TempDir) -> waveflow_server::Config {
    let mut config = declaring(
        temp,
        &[(ScrobbleProvider::LastFm, "default", "http://127.0.0.1:3")],
    );
    config.public_url = Some("https://waveflow.example".to_owned());
    config.lastfm = Some(waveflow_server::config::LastFmApplication {
        api_key: "an-application-key".to_owned(),
        secret: "an-application-secret".to_owned(),
    });
    config
}
