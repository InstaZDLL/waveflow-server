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
//! **Every test that names `spawn_destination` is not.** Those stand a real
//! HTTP server on loopback and let the ListenBrainz adapter talk to it, because
//! what they check exists only on the wire: the `Token` scheme, the JSON shape,
//! the seconds-not-milliseconds timestamp, and what a status line does to a row.
//! A double would prove none of it. They reach `127.0.0.1` and nothing else — no
//! test in this file touches a network this machine does not own.
//!
//! Stated as a rule rather than a list on purpose. This paragraph named three
//! tests and was wrong within the hour, because the list went stale the moment
//! another one was added — twice. A rule cannot drift from the code the way a
//! count does.
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
use waveflow_server::authentication::now_ms;
use waveflow_server::catalog::LibraryRecord;
use waveflow_server::config::ScrobbleLimits;
use waveflow_server::database::LibraryVisibility;
use waveflow_server::services::{
    ScrobbleEnvelope, ScrobbleProvider, ScrobbleTarget, ScrobbleVerdict,
};
use waveflow_server::AppState;

// Not every target uses every fixture, and a shared module is not dead code
// for being partly unused here.
#[allow(dead_code)]
mod support;
use support::*;

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

/// The one envelope waiting, read back from the queue.
async fn queued_envelope(state: &AppState) -> ScrobbleEnvelope {
    let row = sqlx::query_as::<
        _,
        (
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
            Option<i64>,
        ),
    >(
        "SELECT played_at, title, artists_json, album, album_artist, duration_ms \
         FROM scrobble_outbox ORDER BY id LIMIT 1",
    )
    .fetch_one(state.db.pool())
    .await
    .unwrap();
    ScrobbleEnvelope {
        played_at: row.0,
        title: row.1,
        artists: serde_json::from_str(&row.2).unwrap(),
        album: row.3,
        album_artist: row.4,
        duration_ms: row.5,
        musicbrainz_recording_id: None,
    }
}

/// Brings every waiting row's next attempt forward, so a test can exhaust the
/// retries without waiting out the growing delay the server really applies.
async fn make_everything_due(state: &AppState) {
    sqlx::query("UPDATE scrobble_outbox SET next_attempt_at=0 WHERE state='pending'")
        .execute(state.db.pool())
        .await
        .unwrap();
}

/// A destination that takes the submission and then simply never answers.
///
/// Not a refusal and not a network failure: the connection is good, the request
/// is gone, and nothing comes back. It is the shape a stalled far end really
/// has, and the one shape no verdict can describe.
struct Stalls {
    calls: Mutex<usize>,
}

impl ScrobbleTarget for Stalls {
    fn submit<'a>(
        &'a self,
        _envelope: &'a ScrobbleEnvelope,
        _secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async move {
            *self.calls.lock().unwrap() += 1;
            std::future::pending::<ScrobbleVerdict>().await
        })
    }
}

/// An app whose scrobbling limits this test chose.
///
/// Tuned before `initialize`, never after: `DomainServices` copies these values
/// out of `Config` when it is built, so changing one on the returned `Config`
/// would silently exercise the old one.
async fn tuned_app(
    tune: impl FnOnce(&mut ScrobbleLimits),
) -> (tempfile::TempDir, waveflow_server::Config, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    tune(&mut config.scrobbling);
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

#[tokio::test]
async fn a_destination_that_never_answers_does_not_hold_the_queue_forever() {
    let (_temp, config, state) =
        tuned_app(|limits| limits.request_timeout = Duration::from_millis(150)).await;
    let fixture = fixture(&config, &state, "stalling-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    let target = std::sync::Arc::new(Stalls {
        calls: Mutex::new(0),
    });
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    // The first assertion is that this returns at all. The drain is a
    // background task, so without the deadline it is held for the life of the
    // process and nothing on the server ever says so — which is why the wait is
    // bounded here rather than inside whichever adapter happens to be
    // registered.
    let drained = tokio::time::timeout(
        Duration::from_secs(5),
        state.services.drain_scrobble_outbox(),
    )
    .await
    .expect("a destination that never answers must not hold the drain")
    .unwrap();

    assert_eq!(*target.calls.lock().unwrap(), 1);
    // Uncertain rather than retrying: the request left and the answer did not
    // come back, and those two are what decision 5 calls indistinguishable. The
    // destination may already hold this listen, so sending it again is a
    // decision that belongs to a person.
    assert_eq!(drained.uncertain, 1);
    assert_eq!(drained.retrying, 0);
    assert_eq!(rows(&state).await[0].1, "uncertain");

    // And the link says what happened, without echoing anything of the listen.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "degraded");
    assert_eq!(links[0].last_failure.as_deref(), Some("ambiguous"));
}

#[tokio::test]
async fn a_destination_with_no_adapter_does_not_starve_one_that_has_it() {
    // A batch of one, so the row nothing can carry fills a whole pass on its
    // own. That is the smallest arrangement in which the question can be asked
    // at all, and the shape a real server reaches when one destination has been
    // queueing for a while.
    let (_temp, config, state) = tuned_app(|limits| limits.batch = 1).await;
    let fixture = fixture(&config, &state, "starving-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::Maloja, "maloja-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    // One listen, one row per authorisation, and the ListenBrainz row sorts
    // first — which is what puts the unreachable destination at the head.
    assert_eq!(rows(&state).await.len(), 2);

    // Only one of the two can be reached by this process. That is the ordinary
    // state of affairs while adapters are being added one at a time.
    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::Maloja,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let first = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(first.unserviced, 1);
    assert_eq!(first.accepted, 0);

    // The second pass must reach the other destination. Without the row
    // stepping aside, the same unserviceable one is fetched again every time
    // and the Maloja listen is never sent — not late, never: nothing advances
    // a row no adapter can carry.
    let second = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(second.accepted, 1);
    assert_eq!(target.seen().len(), 1);
    assert_eq!(target.seen()[0].1, "maloja-secret");
}

/// Drives one listen to `uncertain`, which three tests below start from.
async fn one_uncertain_entry(state: &AppState, fixture: &Fixture) -> uuid::Uuid {
    let target = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
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

#[tokio::test]
async fn an_ambiguous_entry_may_be_retried_once_and_not_twice() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "retrying-once-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    let uncertain_id = one_uncertain_entry(&state, &fixture).await;

    state
        .services
        .retry_uncertain_scrobble(fixture.owner, uncertain_id)
        .await
        .unwrap();

    // The original stays `uncertain` for good, so nothing in the row itself
    // says it has been answered. Without a guard the same gesture would queue a
    // second copy, and a third a third — and this is the one path in the whole
    // design that can manufacture duplicates on demand. Decision 13 gives that
    // acceptance once, for one listen, deliberately.
    assert!(state
        .services
        .retry_uncertain_scrobble(fixture.owner, uncertain_id)
        .await
        .is_err());
    assert_eq!(rows(&state).await.len(), 2);
}

#[tokio::test]
async fn a_refused_listen_is_not_offered_to_the_destination_again() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "refused-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    let target = Recorder::always(ScrobbleVerdict::PermanentReject);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.rejected, 1);
    assert_eq!(rows(&state).await[0].1, "rejected");

    // No retry can fix a payload the far end has read and will not have, so
    // this is terminal — and unlike `uncertain` it asks nothing of anybody.
    make_everything_due(&state).await;
    assert_eq!(
        state.services.drain_scrobble_outbox().await.unwrap(),
        Default::default()
    );
    assert_eq!(target.seen().len(), 1);

    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].last_failure.as_deref(), Some("rejected"));
    assert_eq!(links[0].pending, 0);
    assert_eq!(links[0].uncertain, 0);
}

#[tokio::test]
async fn a_listen_interrupted_mid_flight_is_uncertain_rather_than_sent_again() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "interrupted-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    // What a process that died between emitting a submission and recording its
    // verdict leaves behind: a row claimed long ago and never settled. Written
    // directly because the only other way to produce it is to kill the server.
    sqlx::query("UPDATE scrobble_outbox SET state='sending', updated_at=?")
        .bind(now_ms() - 600_000)
        .execute(state.db.pool())
        .await
        .unwrap();

    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.recovered, 1);
    // Uncertain, because nobody knows whether the destination took it. Returned
    // to the queue instead, it would have been sent a second time — a duplicate
    // arriving with no person having chosen it, which is the one choice this
    // server never makes.
    assert_eq!(rows(&state).await[0].1, "uncertain");
    assert_eq!(
        target.seen().len(),
        0,
        "an interrupted submission must not be re-emitted"
    );
    // And it is counted where a person will be asked to decide about it.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].uncertain, 1);
    assert_eq!(links[0].health, "degraded");
}

#[tokio::test]
async fn a_silent_destination_costs_one_entry_a_pass_and_not_the_whole_queue() {
    let (_temp, config, state) =
        tuned_app(|limits| limits.request_timeout = Duration::from_millis(150)).await;
    let fixture = fixture(&config, &state, "silent-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    for _ in 0..4 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }
    assert_eq!(rows(&state).await.len(), 4);

    let target = std::sync::Arc::new(Stalls {
        calls: Mutex::new(0),
    });
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    // One deadline spent, not four. The cost that matters is not the twenty
    // seconds saved here: it is that each `uncertain` row is a decision
    // decision 13 lets a person make exactly once, so a single outage would
    // otherwise turn into a heap of irreversible manual choices.
    assert_eq!(*target.calls.lock().unwrap(), 1);
    assert_eq!(drained.uncertain, 1);

    // The one that really was sent is uncertain, because nobody knows whether
    // it arrived. The three behind it were never emitted, so they are untouched
    // and come back next pass.
    let rows = rows(&state).await;
    assert_eq!(rows[0].1, "uncertain");
    for row in &rows[1..] {
        assert_eq!(row.1, "pending");
        assert_eq!(row.2, 0, "an entry that was never sent has spent nothing");
    }
}

/// A destination that answers perfectly well — after something else has
/// already settled the row out from under this pass.
///
/// That is what a concurrent recovery leaves behind, reproduced deterministically
/// instead of by racing a sixty-second margin against the writer gate.
struct SettlesBehindYourBack {
    pool: sqlx::SqlitePool,
}

impl ScrobbleTarget for SettlesBehindYourBack {
    fn submit<'a>(
        &'a self,
        _envelope: &'a ScrobbleEnvelope,
        _secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE scrobble_outbox SET state='uncertain', last_failure='interrupted' \
                 WHERE state='sending'",
            )
            .execute(&self.pool)
            .await
            .unwrap();
            ScrobbleVerdict::Accepted
        })
    }
}

#[tokio::test]
async fn a_verdict_that_arrives_after_its_row_was_settled_is_not_counted() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "late-verdict-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::new(SettlesBehindYourBack {
            pool: state.db.pool().clone(),
        }) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    // Where the row ended up is defensible — nobody knows whether the
    // destination took it. What must not happen is the report carrying on as
    // though this pass had written something.
    assert_eq!(
        drained.accepted, 0,
        "a verdict about a row this pass no longer owns is not an acceptance"
    );
    assert_eq!(rows(&state).await[0].1, "uncertain");

    // And above all: no success stamped for a submission the queue cannot
    // claim. `healthy` must never come from a write that did not happen.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert!(
        links[0].last_success_at.is_none(),
        "last_success_at must not record a success the row never accepted"
    );
    assert_eq!(links[0].uncertain, 1);
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
    let bodies = std::sync::Arc::new(Mutex::new(Vec::new()));
    let authorizations = std::sync::Arc::new(Mutex::new(Vec::new()));
    let router = axum::Router::new().route(
        "/1/submit-listens",
        axum::routing::post({
            let bodies = std::sync::Arc::clone(&bodies);
            let authorizations = std::sync::Arc::clone(&authorizations);
            move |headers: axum::http::HeaderMap, body: String| {
                let bodies = std::sync::Arc::clone(&bodies);
                let authorizations = std::sync::Arc::clone(&authorizations);
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
                    if let Some(seconds) = reset_in {
                        response = response.header("x-ratelimit-reset-in", seconds.to_string());
                    }
                    response.body(axum::body::Body::from("{}")).unwrap()
                }
            }
        }),
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

/// An app configured to reach that destination.
///
/// This goes through `initialize`, so it exercises the whole chain a real
/// server walks — the destination is validated, the client is built, the
/// adapter is registered — rather than registering a target by hand as the
/// tests above do.
async fn app_reaching(base: &str) -> (tempfile::TempDir, waveflow_server::Config, AppState) {
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.listenbrainz_url = Some(base.to_owned());
    let state = waveflow_server::initialize(&config).await.unwrap();
    (temp, config, state)
}

#[tokio::test]
async fn a_listen_reaches_the_destination_in_the_shape_it_documents() {
    let destination = spawn_destination(200, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "listenbrainz-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, Some(1_700_000_000_123))
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.accepted, 1);
    assert_eq!(rows(&state).await[0].1, "sent");

    // The scheme ListenBrainz documents, and not `Bearer`.
    let authorizations = destination.authorizations.lock().unwrap().clone();
    assert_eq!(authorizations, vec!["Token lb-secret".to_owned()]);

    let bodies = destination.bodies.lock().unwrap().clone();
    assert_eq!(bodies.len(), 1);
    let listen = &bodies[0]["payload"][0];
    assert_eq!(bodies[0]["listen_type"], "single");
    // **Seconds.** The envelope holds epoch milliseconds like everything else
    // here, and sending those unconverted would date this listen some fifty
    // thousand years out — in a history that keeps it.
    assert_eq!(listen["listened_at"], 1_700_000_000);
    assert_eq!(listen["track_metadata"]["track_name"], "Matrix flac");
    assert_eq!(listen["track_metadata"]["artist_name"], "Alpha, Beta");
}

#[tokio::test]
async fn a_rate_limited_listen_waits_at_least_as_long_as_it_was_asked() {
    // An hour, against a first backoff of one minute, so only the destination's
    // own answer can explain the wait.
    let destination = spawn_destination(429, Some(3_600)).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "rate-limited-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    let before = now_ms();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.retrying, 1);

    // Honouring a request to slow down means not returning before it says, so
    // the wait is the longer of the two and never the destination's in place of
    // ours. Decision 6 promised this and PR #191 had nowhere to carry it.
    let next: i64 =
        sqlx::query_scalar("SELECT next_attempt_at FROM scrobble_outbox ORDER BY id LIMIT 1")
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(
        next >= before + 3_600_000,
        "the wait must honour the destination's own answer, not our shorter backoff"
    );

    // And the link says *why* it is waiting, in the normalised vocabulary
    // decision 12 describes. `rate_limited` was documented there while nothing
    // wrote it, and then written to the wrong table — the outbox row rather
    // than the link that `ScrobbleLinkState` actually reads.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].last_failure.as_deref(), Some("rate_limited"));
}

#[tokio::test]
async fn a_token_that_cannot_be_a_header_is_refused_when_it_is_pasted() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "pasting-listener").await;

    // A newline is what a copy out of a web page hands you, and a secret
    // carrying one can never be spelled as an HTTP header — so it will never
    // work against any destination. Refused at the only moment the person can
    // fix it, rather than discovered hours later on a background drain.
    assert!(state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb\nsecret")
        .await
        .is_err());
    assert!(
        state
            .services
            .scrobble_links(fixture.owner)
            .await
            .unwrap()
            .is_empty(),
        "a refused token must leave no link behind to queue listens under"
    );

    // And the check is not so eager that it refuses an ordinary one.
    assert!(state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .is_ok());
}

#[tokio::test]
async fn a_credential_sealed_before_that_check_breaks_the_link_rather_than_going_uncertain() {
    let destination = spawn_destination(200, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "sealed-badly-listener").await;

    // Sealed and inserted by hand, because `link_scrobble` now refuses this
    // through the front door — and rows sealed before it did are never
    // revalidated. This is the case the adapter's own guard exists for, and
    // without this test that claim would be prose.
    let sealed = state.secret_box.encrypt(b"lb\nsecret").unwrap();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO scrobble_link (id, user_id, provider, status, credential_nonce, \
         credential_ciphertext, created_at, updated_at) \
         VALUES (?, ?, 'listenbrainz', 'active', ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(fixture.owner.to_string())
    .bind(sealed.nonce.as_slice())
    .bind(sealed.ciphertext.as_slice())
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    // A stored secret that cannot be spelled as a header is a broken
    // authorisation, and is named one.
    assert_eq!(drained.broken, 1);
    assert_eq!(
        drained.uncertain, 0,
        "bytes that never left must not become the one verdict a person cannot undo"
    );
    assert_eq!(rows(&state).await[0].1, "cancelled");
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "broken");

    // And nothing was sent, because there was never a request to send.
    assert_eq!(destination.bodies.lock().unwrap().len(), 0);
}

#[tokio::test]
async fn a_destination_cannot_park_a_listen_past_our_own_ceiling() {
    // About thirty-one thousand years, which is the shape a hostile or merely
    // broken destination takes. `after` is the only value in the whole
    // scheduling path a third party chooses.
    let destination = spawn_destination(429, Some(999_999_999_999)).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "absurd-wait-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    let before = now_ms();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    state.services.drain_scrobble_outbox().await.unwrap();

    let next: i64 =
        sqlx::query_scalar("SELECT next_attempt_at FROM scrobble_outbox ORDER BY id LIMIT 1")
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    // Clamped to the hour the backoff already calls the point past which
    // waiting buys nothing, plus at most its own quarter of spread. Unclamped,
    // this row would sit at a moment it never reaches — a third party deciding
    // the listen dies, which is decision 10's concern arriving from the side
    // nobody watches.
    assert!(
        next <= before + 3_600_000 + 900_000 + 10_000,
        "a destination must not be able to schedule a listen beyond our own ceiling"
    );
    // And still a real wait: clamping is not ignoring.
    assert!(next >= before + 3_600_000);
}

// The herd this file used to test for lives in
// `services::scrobbling::tests::the_spread_scatters_neighbouring_rows_across_the_interval`
// now. Two of the fixes in this branch turned out to contradict each other
// here: a rate-limited link now rests for the rest of the pass, so only one row
// is ever offered, and a test that needed four waits in one pass could no
// longer get them. The property is a pure function's, and proving it through a
// drain, a database and an HTTP server proved less while costing more. What a
// rate limit does to the *batch* is `a_destination_that_asked_for_room_is_not_offered_the_rest_of_the_batch`;
// what it does to the *wait* is `a_rate_limited_listen_waits_at_least_as_long_as_it_was_asked`.

#[tokio::test]
async fn a_destination_that_asked_for_room_is_not_offered_the_rest_of_the_batch() {
    let destination = spawn_destination(429, Some(3_600)).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "resting-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    for _ in 0..4 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    // One submission, not four. The batch is chosen before the first verdict,
    // so without the link resting, being told to slow down would be answered by
    // sending the rest of it — each row earning its own refusal and spending
    // its own attempt, at a destination that had just asked for room.
    assert_eq!(destination.bodies.lock().unwrap().len(), 1);
    assert_eq!(drained.retrying, 1);

    // The one that was refused waits and has spent an attempt; the three behind
    // it were never offered, so they have spent nothing and come back next
    // pass.
    let rows = rows(&state).await;
    // Asserted before the loop below, which would otherwise pass vacuously if
    // the four calls ever stopped queueing four rows.
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].2, 1);
    for row in &rows[1..] {
        assert_eq!(row.1, "pending");
        assert_eq!(row.2, 0, "a row that was never offered has spent nothing");
    }
}

#[tokio::test]
async fn a_drain_interval_longer_than_the_ceiling_does_not_kill_the_drain() {
    let destination = spawn_destination(429, Some(3_600)).await;
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.listenbrainz_url = Some(destination.base.clone());
    // Longer than `RETRY_CEILING`, which nothing forbids: `parse_positive_env`
    // bounds this below zero and nowhere above. That made the rest floor exceed
    // the rest ceiling, and `Ord::clamp` panics when its minimum exceeds its
    // maximum — inside a `tokio::spawn`ed loop, so the queue would have stopped
    // for the life of the process. The silent stop this whole RFC exists to
    // prevent, reachable by one plausible setting.
    config.scrobbling.drain_interval = Duration::from_secs(7_200);
    let state = waveflow_server::initialize(&config).await.unwrap();
    let fixture = fixture(&config, &state, "slow-interval-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    // **Two**, and that is the whole difference between this test and the one
    // it replaced. With a single listen the bounds are computed, written into
    // the resting map, and never read — the map is only consulted for a
    // *second* row on the same link — so the wait asserted below came from
    // `reschedule_scrobble`'s own ceiling and the assertion could not fail for
    // the reason its comment gave. A review caught that; the earlier version of
    // this file had the same shape twice before.
    for _ in 0..2 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    // The first assertion is that this returns at all rather than unwinding.
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.retrying, 1);
    assert_eq!(drained.rested, 1);

    let waits: Vec<i64> =
        sqlx::query_scalar("SELECT next_attempt_at - updated_at FROM scrobble_outbox ORDER BY id")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    // The row that was offered waits by its own schedule, capped at the hour.
    assert!((3_600_000..=4_600_000).contains(&waits[0]));
    // The row behind it waits by the bounds under test — and this is the half
    // that needed a second listen to exist at all. The floor wins here, because
    // a pass every two hours means a one-hour deferral would have the row
    // offered again before the next pass could serve it.
    assert_eq!(waits[1], 7_200_000);
}

#[tokio::test]
async fn a_rate_limit_that_names_no_delay_still_rests_the_link() {
    // `429` carrying no `X-RateLimit-Reset-In` at all — a destination under
    // load, or one whose proxy stripped the header on the way back.
    //
    // The **status** is what says room was asked for; the header only says how
    // much. Passing the missing header through as `after: None` made this
    // indistinguishable from a connect failure, so the link rested for nothing
    // and the cause was recorded as an ordinary `retryable` — both of the
    // things that value carries, bypassed by a header being absent.
    let destination = spawn_destination(429, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "silent-limit-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    for _ in 0..4 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(
        destination.bodies.lock().unwrap().len(),
        1,
        "a rate limit that named no delay must still stop the rest of the batch"
    );
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].last_failure.as_deref(), Some("rate_limited"));
}

#[tokio::test]
async fn a_rate_limited_backlog_does_not_starve_another_destination() {
    let listenbrainz = spawn_destination(429, Some(3_600)).await;
    // A batch of four, so the backlog below fills a whole pass on its own —
    // the shape a real server reaches the moment one destination is limited
    // and the other is not.
    let temp = tempfile::tempdir().unwrap();
    let mut config = waveflow_server::Config::for_data_dir(temp.path().join("data"));
    config.listenbrainz_url = Some(listenbrainz.base.clone());
    config.scrobbling.batch = 4;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let fixture = fixture(&config, &state, "starved-listener").await;

    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    for _ in 0..4 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    // A second destination, linked after the backlog exists and reachable.
    let maloja = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::Maloja,
        std::sync::Arc::clone(&maloja) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::Maloja, "maloja-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    // The first pass is filled by the rate-limited backlog: one row offered and
    // refused, the other three moved out of the way.
    let first = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(first.retrying, 1);
    assert_eq!(
        first.rested, 3,
        "a pass that moves rows aside must say so, or it looks like a pass that did nothing"
    );
    // The second must reach the destination that is perfectly willing.
    state.services.drain_scrobble_outbox().await.unwrap();

    // Skipping the rested rows rather than deferring them leaves them due with
    // an *older* `next_attempt_at`, so they keep sorting ahead of this one and
    // it is never reached — not slowly, never.
    assert_eq!(
        maloja.seen().len(),
        1,
        "a rate-limited backlog must not starve a destination that is answering"
    );
}

#[tokio::test]
async fn the_spread_reaches_the_rows_the_drain_actually_reschedules() {
    // A server fault rather than a rate limit, on purpose: a `500` earns
    // `Retryable { after: None }`, which does not rest the link — so all four
    // rows are rescheduled in one pass and their waits can only differ by the
    // spread.
    let destination = spawn_destination(500, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "scattered-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    for _ in 0..4 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.retrying, 4);

    // This is the test the deleted integration one should have been. A review
    // pointed out that after that deletion, *nothing* failed if the spread
    // stopped being applied in production: the unit test proves the pure
    // function, and the helper beside it re-implements the composition rather
    // than calling the path the drain takes. Removing
    // `.saturating_add(retry_spread(..))` from `reschedule_scrobble` left the
    // whole suite green.
    let waits: Vec<i64> =
        sqlx::query_scalar("SELECT next_attempt_at - updated_at FROM scrobble_outbox ORDER BY id")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert_eq!(waits.len(), 4);
    let mut sorted = waits.clone();
    sorted.sort_unstable();
    let smallest_gap = sorted
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .min()
        .expect("four rows have three gaps");
    assert!(
        smallest_gap >= 1_000,
        "the drain rescheduled four rows {smallest_gap}ms apart, which is still a herd"
    );
}

#[tokio::test]
async fn a_destination_that_refuses_the_token_breaks_the_link() {
    let destination = spawn_destination(401, None).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "refused-token-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "stale-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.broken, 1);
    assert_eq!(rows(&state).await[0].1, "cancelled");
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "broken");
    assert_eq!(links[0].last_failure.as_deref(), Some("auth_broken"));
}

#[tokio::test]
async fn an_account_that_linked_nothing_queues_nothing() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "unlinked-listener").await;

    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    // The listen is recorded, as it always was.
    assert_eq!(
        state
            .services
            .history(fixture.owner, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    // And nothing left the server. This is the ordinary installation: a server
    // that was merely upgraded makes no outbound request at all.
    assert!(rows(&state).await.is_empty());
}

#[tokio::test]
async fn a_listen_is_queued_with_what_was_heard_and_a_now_playing_is_not() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "queueing-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-token")
        .await
        .unwrap();

    // A now-playing has no value once it stops being true, so it is never
    // queued — decision 3.
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, false, None)
        .await
        .unwrap();
    assert!(
        rows(&state).await.is_empty(),
        "a now-playing must never be queued for delivery"
    );

    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, Some(1_700_000_000_000))
        .await
        .unwrap();

    let queued = rows(&state).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].1, "pending");
    assert_eq!(queued[0].2, 0);

    // The envelope is the listen as it was heard, not a pointer to the track.
    let envelope = queued_envelope(&state).await;
    assert_eq!(envelope.played_at, 1_700_000_000_000);
    assert_eq!(envelope.title, "Matrix flac");
    assert_eq!(
        envelope.artists,
        vec!["Alpha".to_owned(), "Beta".to_owned()]
    );
    assert_eq!(envelope.album.as_deref(), Some("WaveFlow format matrix"));
    assert_eq!(envelope.album_artist.as_deref(), Some("Matrix Artist"));
}

#[tokio::test]
async fn a_correction_after_the_listen_does_not_change_what_was_queued() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "retagging-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-token")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    // The member corrects the track afterwards, which is what #186 made
    // possible and what decision 2 was rewritten for: a correction rewrites
    // `track_participant`, so reading the track at drain time would submit what
    // it has become rather than what was played.
    state
        .services
        .set_track_metadata(
            fixture.owner,
            fixture.tagged,
            waveflow_server::services::TrackMetadataPatch {
                title: Some(Some("Corrected afterwards".to_owned())),
                artists: Some(Some(vec!["Somebody Else".to_owned()])),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

    let envelope = queued_envelope(&state).await;
    assert_eq!(
        envelope.title, "Matrix flac",
        "the envelope records the listen, and the listen already happened"
    );
    assert_eq!(
        envelope.artists,
        vec!["Alpha".to_owned(), "Beta".to_owned()]
    );
}

#[tokio::test]
async fn a_track_the_server_cannot_name_is_never_queued() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "untagged-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-token")
        .await
        .unwrap();

    state
        .services
        .scrobble(fixture.owner, fixture.bare, true, None)
        .await
        .unwrap();
    assert!(
        rows(&state).await.is_empty(),
        "a submission with no credited artist is unusable at the far end"
    );

    // And the same account, same link, same call, on a track that can be named.
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    assert_eq!(rows(&state).await.len(), 1);
}

#[tokio::test]
async fn an_accepted_listen_leaves_the_queue_and_the_secret_arrives_intact() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "accepting-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    // Before any adapter is registered, the pass leaves the row exactly as it
    // is. A server missing an adapter is a misconfiguration, and spending the
    // listen's retries on it would destroy the queue the operator is about to
    // repair.
    let unserviced = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(unserviced.unserviced, 1);
    assert_eq!(rows(&state).await[0].2, 0, "no attempt may have been spent");
    // It does step aside, though, so that a destination which *can* be reached
    // is never stuck behind it. Bringing it forward here is what a restart does
    // once the operator has supplied the missing adapter.
    make_everything_due(&state).await;

    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.accepted, 1);
    assert_eq!(rows(&state).await[0].1, "sent");

    // The secret was sealed under the instance key on the way in and came back
    // out whole.
    let seen = target.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].1, "lb-secret");
    assert_eq!(seen[0].0.title, "Matrix flac");

    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].health, "healthy");
    assert_eq!(links[0].pending, 0);
    assert!(links[0].last_success_at.is_some());

    // A second pass has nothing left to do, which is what "exactly one queueing"
    // looks like from the drain's side.
    assert_eq!(
        state.services.drain_scrobble_outbox().await.unwrap(),
        Default::default()
    );
    assert_eq!(target.seen().len(), 1);
}

#[tokio::test]
async fn an_ambiguous_answer_is_terminal_and_waits_for_a_person() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "ambiguous-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    let target = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.uncertain, 1);
    assert_eq!(rows(&state).await[0].1, "uncertain");

    // No automatic retry, ever. The destination may already hold this listen,
    // and a duplicate in a public history is worse than a gap — decision 5.
    make_everything_due(&state).await;
    assert_eq!(
        state.services.drain_scrobble_outbox().await.unwrap(),
        Default::default()
    );
    assert_eq!(
        target.seen().len(),
        1,
        "an ambiguous result must never be submitted again on the server's own initiative"
    );

    // It is counted, and it is what makes the link degraded: nothing will move
    // it without a person.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].uncertain, 1);
    assert_eq!(links[0].health, "degraded");
}

#[tokio::test]
async fn a_retryable_answer_comes_back_until_the_attempts_run_out() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "retrying-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    let target = Recorder::always(ScrobbleVerdict::Retryable { after: None });
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    // The first failure schedules a later attempt rather than a immediate one.
    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.retrying, 1);
    assert_eq!(rows(&state).await[0].2, 1);
    assert_eq!(
        state.services.drain_scrobble_outbox().await.unwrap(),
        Default::default(),
        "the row is not due again yet"
    );

    // `for_data_dir` allows three attempts, so the third is the last.
    make_everything_due(&state).await;
    assert_eq!(
        state
            .services
            .drain_scrobble_outbox()
            .await
            .unwrap()
            .retrying,
        1
    );
    make_everything_due(&state).await;
    let last = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(last.abandoned, 1);
    let rows = rows(&state).await;
    assert_eq!(rows[0].1, "abandoned");
    assert_eq!(rows[0].2, 3);
    assert_eq!(
        target.seen().len(),
        3,
        "a queue that never empties is a fault, not a state"
    );
}

#[tokio::test]
async fn a_broken_authorisation_finishes_the_queue_rather_than_asking_again() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "broken-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    for _ in 0..3 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }
    assert_eq!(rows(&state).await.len(), 3);

    let target = Recorder::always(ScrobbleVerdict::AuthBroken);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.broken, 1);
    for row in rows(&state).await {
        assert_eq!(
            row.1, "cancelled",
            "every waiting row would fail identically"
        );
    }
    // And the other two were never submitted. The batch was chosen before the
    // first verdict came back; asking a refused question twice more is exactly
    // what `AuthBroken` has already settled.
    assert_eq!(target.seen().len(), 1);

    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "broken");
    assert_eq!(links[0].last_failure.as_deref(), Some("auth_broken"));
}

#[tokio::test]
async fn unlinking_leaves_its_queue_behind_and_a_new_link_does_not_inherit_it() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "relinking-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "first-account",
        )
        .await
        .unwrap();
    for _ in 0..2 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

    assert!(state
        .services
        .unlink_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz)
        .await
        .unwrap());
    // Cancelled rather than deleted: they record listens that really happened
    // and really were never sent, and a queue that erases its own losses cannot
    // be asked what it lost.
    for row in rows(&state).await {
        assert_eq!(row.1, "cancelled");
    }
    assert!(state
        .services
        .scrobble_links(fixture.owner)
        .await
        .unwrap()
        .is_empty());

    // A different account at the same destination. This is the whole reason the
    // queue references a generation and not the pair `(account, destination)`:
    // the two listens above must not land on this profile.
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "second-account",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    let drained = state.services.drain_scrobble_outbox().await.unwrap();

    assert_eq!(drained.accepted, 1);
    let seen = target.seen();
    assert_eq!(
        seen.len(),
        1,
        "the listens queued under the first authorisation must not reach the second"
    );
    assert_eq!(seen[0].1, "second-account");
}

#[tokio::test]
async fn retrying_an_uncertain_entry_adds_to_the_history_instead_of_rewriting_it() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "deciding-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    let target = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state.services.drain_scrobble_outbox().await.unwrap();
    // The rowid for the shape of the queue, the public name for the gesture.
    let uncertain_row = rows(&state).await[0].0;
    let uncertain = public_ids(&state).await[0];

    let retry = state
        .services
        .retry_uncertain_scrobble(fixture.owner, uncertain)
        .await
        .unwrap();

    let rows_after = rows(&state).await;
    assert_eq!(rows_after.len(), 2);
    // The ambiguous attempt stays in the record exactly as it happened.
    assert_eq!(
        rows_after[0],
        (uncertain_row, "uncertain".to_owned(), 1, None)
    );
    // The new one says who asked for it, by pointing at the one it repeats.
    assert_eq!(rows_after[1].1, "pending");
    assert_eq!(rows_after[1].2, 0);
    assert_eq!(rows_after[1].3, Some(uncertain_row));
    // And it is answered for by the name the caller was handed, not a rowid.
    assert_eq!(public_ids(&state).await[1], retry);
    assert_ne!(retry, uncertain);

    // It has stopped asking the person for a decision without having stopped
    // being true.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].uncertain, 0);
    assert_eq!(links[0].pending, 1);
}

#[tokio::test]
async fn discarding_an_uncertain_entry_stops_it_counting() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "discarding-listener").await;
    state
        .services
        .link_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "lb-secret")
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    let target = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        std::sync::Arc::clone(&target) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state.services.drain_scrobble_outbox().await.unwrap();
    let uncertain = public_ids(&state).await[0];

    state
        .services
        .discard_uncertain_scrobble(fixture.owner, uncertain)
        .await
        .unwrap();

    assert_eq!(rows(&state).await[0].1, "discarded");
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].uncertain, 0);
    assert_eq!(links[0].health, "healthy");

    // And it is gone for good: a second gesture on it finds nothing to act on.
    assert!(state
        .services
        .discard_uncertain_scrobble(fixture.owner, uncertain)
        .await
        .is_err());
}
