//! The queue a listen leaves by. RFC-010.
//!
//! No routes yet: this drives `DomainServices` directly, which is the whole of
//! what the domain half of scrobbling is. What is queued, what is never queued,
//! what each verdict does to a row and what unlinking does to a queue are
//! decided there rather than at a surface, so they are tested there.
//!
//! **Nothing here reaches the network.** Every destination is a double
//! implementing [`ScrobbleTarget`], which is the point of the trait: the drain
//! knows five words and no providers, so the five words are what a test drives.
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
    let target = Recorder::always(ScrobbleVerdict::Retryable);
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
