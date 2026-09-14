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
    ScrobbleEnvelope, ScrobbleProvider, ScrobbleTarget, ScrobbleVerdict, ServiceError,
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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
    // Last.fm is the one this process cannot reach: the suite's configuration
    // declares an instance for it and no application credentials, so no adapter
    // is built for it. Declared and adapterless is a shape a real deployment
    // has, which is why the test stands on it rather than on a recipient that
    // merely has no adapter yet.
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::LastFm,
            "default",
            "lastfm-secret",
        )
        .await
        .unwrap();
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::Maloja,
            "default",
            "maloja-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    // One listen, one row per authorisation, and the Last.fm row sorts first —
    // which is what puts the unreachable destination at the head.
    assert_eq!(rows(&state).await.len(), 2);

    // Only one of the two can be reached by this process. That is the ordinary
    // state of affairs while adapters are being added one at a time.
    let target = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::Maloja,
        "default",
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

/// An ambiguous entry can be found before it is answered, and the list agrees
/// with the counter standing next to it.
///
/// Decision 13 grants a person two gestures — discard this, retry that — and
/// both name an entry by its `public_id`. Nothing published one, so both were
/// unreachable from outside: a choice offered with nothing to choose between.
///
/// **The agreement is the real assertion.** `scrobble_links` stops counting an
/// entry once it has been retried, so a list built on a different predicate
/// would show a decision the count says is already made. They are checked
/// together here, before and after the retry, because either one alone passes
/// while they disagree.
#[tokio::test]
async fn an_ambiguous_entry_can_be_found_before_it_is_answered() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "choosing-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let uncertain_id = one_uncertain_entry(&state, &fixture).await;

    let waiting = state
        .services
        .uncertain_scrobbles(fixture.owner)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].id, uncertain_id, "the entry names itself");
    assert_eq!(waiting[0].provider, ScrobbleProvider::ListenBrainz);
    assert_eq!(waiting[0].attempts, 1);
    // The envelope stays where it is. Decision 12: a state, never an echo of
    // what was heard — which is why there is no title on this type to assert.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].uncertain, waiting.len() as i64);

    // Answered, so it stops asking — and the count beside it stops too.
    state
        .services
        .retry_uncertain_scrobble(fixture.owner, uncertain_id)
        .await
        .unwrap();
    assert!(state
        .services
        .uncertain_scrobbles(fixture.owner)
        .await
        .unwrap()
        .is_empty());
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(
        links[0].uncertain, 0,
        "the list and the counter move together"
    );
    // And the acceptance is spent on both sides: an entry that has stopped
    // being listed has stopped being discardable too. Until this branch
    // `discard_uncertain_scrobble` carried neither of the listing's exclusions,
    // so a client holding the id could still flip a retried original to
    // `discarded` — a gesture nothing offered any more.
    assert!(matches!(
        state
            .services
            .discard_uncertain_scrobble(fixture.owner, uncertain_id)
            .await
            .unwrap_err(),
        ServiceError::NotFound
    ));
}

/// Another account's ambiguous entry is not listed, and not nameable.
///
/// Held by the join in the query rather than by the caller, as everywhere else
/// here: an entry another person must decide about is not one this account may
/// even learn the id of.
#[tokio::test]
async fn one_account_never_sees_another_account_s_ambiguous_entries() {
    let (_temp, config, state) = test_app().await;
    let owner = fixture(&config, &state, "ambiguous-owner").await;
    let stranger = fixture(&config, &state, "ambiguous-stranger").await;
    state
        .services
        .link_scrobble(
            owner.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let uncertain_id = one_uncertain_entry(&state, &owner).await;

    assert!(state
        .services
        .uncertain_scrobbles(stranger.owner)
        .await
        .unwrap()
        .is_empty());
    // And knowing the id changes nothing, which is the half a list alone would
    // not prove. Named rather than merely failed: `.is_err()` would accept a
    // database fault as readily as a refusal, and the claim here is about
    // *which* answer a stranger gets.
    let refused = state
        .services
        .discard_uncertain_scrobble(stranger.owner, uncertain_id)
        .await
        .unwrap_err();
    assert!(matches!(refused, ServiceError::NotFound));
}

/// A broken link keeps its question, and loses one of the two answers.
///
/// `retry_uncertain_scrobble` requires `status='active'`, exactly as the drain
/// does — resubmitting under a token the destination has already refused is not
/// a thing to offer. But discarding stays available and stays meaningful, so the
/// entry goes on being listed.
///
/// The listing's own doc claimed the opposite for a while: that it excluded
/// whatever retry refuses. True of `unlinked`, false of `broken`, and nothing
/// held the difference still. This does.
#[tokio::test]
async fn an_entry_under_a_broken_link_can_be_discarded_but_not_retried() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "broken-link-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let uncertain_id = one_uncertain_entry(&state, &fixture).await;

    // A second listen, refused for its credential, is what marks the link
    // broken. The ambiguous entry above is terminal and is not touched by it.
    let refuses = Recorder::always(ScrobbleVerdict::AuthBroken);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&refuses) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    // The tagged track, not the bare one. `bare` carries no usable tags and is
    // never queued at all — the first version of this test scrobbled it, queued
    // nothing, drained nothing, and left the link perfectly healthy while
    // asserting it was broken.
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();
    // Said out loud rather than inferred from the health string: if queueing
    // ever stopped working, every assertion below would still be reachable by
    // accident and this test would report something it had not exercised.
    assert_eq!(
        refuses.seen().len(),
        1,
        "the destination has to have been asked for its refusal to mean anything"
    );
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].health, "broken", "the token was refused");

    // Still asking, because discarding is still an answer.
    let waiting = state
        .services
        .uncertain_scrobbles(fixture.owner)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].id, uncertain_id);

    // But not this answer: a retry would submit under a credential the
    // destination has already refused.
    let refused = state
        .services
        .retry_uncertain_scrobble(fixture.owner, uncertain_id)
        .await
        .unwrap_err();
    // Named rather than merely failed: `.is_err()` cannot tell "the link is
    // broken, so this entry is not findable for retry" from any other fault,
    // and a test that accepts every failure accepts the wrong one too.
    assert!(matches!(refused, ServiceError::NotFound));

    // The one that remains works, and the question then stops.
    state
        .services
        .discard_uncertain_scrobble(fixture.owner, uncertain_id)
        .await
        .unwrap();
    assert!(state
        .services
        .uncertain_scrobbles(fixture.owner)
        .await
        .unwrap()
        .is_empty());
}

/// Withdrawing the authorisation stops the question, without erasing the answer.
///
/// `retry_uncertain_scrobble` already refuses an entry whose generation is gone
/// — a retry under a withdrawn authorisation would submit to whichever profile
/// is linked now, which is what generations exist to prevent. So an entry that
/// can no longer be retried must stop being offered as a decision, or the
/// surface asks for one of two gestures it will then refuse.
///
/// The row itself stays, and stays `uncertain`. It has stopped being a question;
/// it has not stopped being true, and those are different things.
#[tokio::test]
async fn an_unlinked_generation_stops_asking_for_a_decision() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "withdrawing-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let uncertain_id = one_uncertain_entry(&state, &fixture).await;
    assert_eq!(
        state
            .services
            .uncertain_scrobbles(fixture.owner)
            .await
            .unwrap()
            .len(),
        1
    );

    state
        .services
        .unlink_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "default")
        .await
        .unwrap();

    assert!(state
        .services
        .uncertain_scrobbles(fixture.owner)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        rows(&state).await[0].1,
        "uncertain",
        "the entry stops asking, and is not rewritten for it"
    );
    // Nor discardable. The gesture the list stopped offering is the gesture the
    // service stopped accepting, which is what keeps the two from disagreeing
    // about an entry nobody can act on any more.
    assert!(matches!(
        state
            .services
            .discard_uncertain_scrobble(fixture.owner, uncertain_id)
            .await
            .unwrap_err(),
        ServiceError::NotFound
    ));
}

#[tokio::test]
async fn an_ambiguous_entry_may_be_retried_once_and_not_twice() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "retrying-once-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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

/// The joker stays spent once the retry that spent it is gone.
///
/// Until `retried_at`, all four readers deduced "already answered" from a
/// second row naming this one through `retry_of`. That holds only while rows
/// are immortal, and RFC-010's retention makes them mortal: a retry ends
/// `sent`, so it is the first thing a purge takes. The `DELETE` below is that
/// purge, played by hand because the purge itself belongs to the next slice —
/// what it does to this entry does not wait for it.
///
/// Four assertions rather than one because there were four readers, and the
/// counter is the one that matters most: `link_health` answers `degraded` for a
/// single counted `uncertain`, so a link would sit in permanent false alarm
/// over a listen whose owner answered weeks ago.
#[tokio::test]
async fn a_spent_joker_is_not_returned_when_the_retry_is_purged() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "purged-retry-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let uncertain_id = one_uncertain_entry(&state, &fixture).await;
    state
        .services
        .retry_uncertain_scrobble(fixture.owner, uncertain_id)
        .await
        .unwrap();

    // Retention, played by hand: the retry row leaves, the answered original
    // stays. `uncertain` is never purged, so this is the shape a real purge
    // leaves behind and not an invented one.
    sqlx::query("DELETE FROM scrobble_outbox WHERE retry_of IS NOT NULL")
        .execute(state.db.pool())
        .await
        .unwrap();
    assert_eq!(
        rows(&state).await.len(),
        1,
        "the purge must have taken the retry and left the original"
    );

    assert!(
        state
            .services
            .uncertain_scrobbles(fixture.owner)
            .await
            .unwrap()
            .is_empty(),
        "an answered entry must not come back asking"
    );
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(
        links[0].uncertain, 0,
        "an answered entry must not be counted again"
    );
    assert_eq!(
        links[0].health, "healthy",
        "and the link must not be degraded by a listen already answered"
    );
    assert!(
        matches!(
            state
                .services
                .retry_uncertain_scrobble(fixture.owner, uncertain_id)
                .await
                .unwrap_err(),
            ServiceError::NotFound
        ),
        "the joker was spent once and does not come back"
    );
    assert!(
        matches!(
            state
                .services
                .discard_uncertain_scrobble(fixture.owner, uncertain_id)
                .await
                .unwrap_err(),
            ServiceError::NotFound
        ),
        "nor does the gesture it replaced"
    );
}

#[tokio::test]
async fn a_refused_listen_is_not_offered_to_the_destination_again() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "refused-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
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

/// Points ListenBrainz's declared instance at `base`.
///
/// Replaces the unreachable loopback `Config::for_data_dir` declares for that
/// recipient rather than adding beside it — two instances cannot share a name —
/// and leaves the other recipients alone, because more than one test needs a
/// second destination in the same queue.
fn reaching(config: &mut waveflow_server::Config, base: &str) {
    let url = waveflow_server::scrobblers::validate_destination(base, true)
        .expect("a destination the suite chose");
    let fingerprint = waveflow_server::scrobblers::destination_fingerprint(&url);
    config
        .destinations
        .retain(|declared| declared.provider != ScrobbleProvider::ListenBrainz);
    config
        .destinations
        .push(waveflow_server::config::ScrobbleDestination {
            provider: ScrobbleProvider::ListenBrainz,
            name: "default".to_owned(),
            url,
            fingerprint,
        });
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
    reaching(&mut config, base);
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
async fn a_rate_limit_named_only_by_the_standard_header_is_honoured_too() {
    // No vendor header at all — this is the `429` a proxy in front of a
    // self-hosted instance answers with, and decision 6 names `Retry-After` as
    // a delay to honour.
    //
    // The test above cannot tell the two readings apart: it sends
    // `x-ratelimit-reset-in`, so it passes whether or not the adapter ever
    // learned the standard header. For a while it had not — the function that
    // read it was written and never called, and only a dead-code warning said
    // so. This test is what would have said so instead.
    // Half an hour, not a whole one: `RETRY_CEILING_MS` is exactly an hour, so
    // asking for 3600 sits the expected value on the clamp boundary and passes
    // only by the margin the clock happens to add. Half of it tests the same
    // reading with nothing resting on where the ceiling falls.
    let destination = spawn_delaying(429, Some(("retry-after", "1800".to_owned()))).await;
    let (_temp, config, state) = app_reaching(&destination.base).await;
    let fixture = fixture(&config, &state, "proxied-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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

    // Half an hour, against a first backoff of one minute: only the header can
    // explain a wait this long, so reading it is the only way to pass.
    let next: i64 =
        sqlx::query_scalar("SELECT next_attempt_at FROM scrobble_outbox ORDER BY id LIMIT 1")
            .fetch_one(state.db.pool())
            .await
            .unwrap();
    assert!(
        next >= before + 1_800_000,
        "a `Retry-After` of half an hour must outlast our own one-minute backoff"
    );
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb\nsecret"
        )
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret"
        )
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
    // The instance is named here as `link_scrobble` would name it: the drain
    // picks an adapter by `(provider, destination)`, so a row without one is
    // not the case this test is about — it is a row nothing can carry at all.
    let declared = config
        .destinations
        .iter()
        .find(|declared| declared.provider == ScrobbleProvider::ListenBrainz)
        .expect("the suite declares one");
    sqlx::query(
        "INSERT INTO scrobble_link (id, user_id, provider, destination, \
         destination_fingerprint, status, credential_nonce, credential_ciphertext, \
         created_at, updated_at) \
         VALUES (?, ?, 'listenbrainz', ?, ?, 'active', ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(fixture.owner.to_string())
    .bind(declared.name.clone())
    .bind(declared.fingerprint.clone())
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let before = now_ms();
    // **Two**, and for the same reason the drain-interval test needed two: the
    // resting map is only consulted for a *second* row on the same link, so
    // with one listen the clamped value is computed, stored, and never read. A
    // review caught that this test proved the floor and not the ceiling — the
    // inert half surviving one screen above the place it had just been fixed.
    for _ in 0..2 {
        state
            .services
            .scrobble(fixture.owner, fixture.tagged, true, None)
            .await
            .unwrap();
    }

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

    // The row behind it carries the clamp itself, exactly. `defer_scrobble`
    // binds `next_attempt_at` and `updated_at` from one `now`, so this
    // difference is the deferral and nothing else — an hour, where the
    // destination asked for thirty-one thousand years.
    let waits: Vec<i64> =
        sqlx::query_scalar("SELECT next_attempt_at - updated_at FROM scrobble_outbox ORDER BY id")
            .fetch_all(state.db.pool())
            .await
            .unwrap();
    assert_eq!(waits.len(), 2);
    assert_eq!(
        waits[1], 3_600_000,
        "the ceiling must bound what a destination can ask the queue to wait"
    );
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
    reaching(&mut config, &destination.base);
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
    // Asserted before either row is indexed, which is the guard this file
    // argues for twice elsewhere — indexing panics rather than passing
    // vacuously, but the inconsistency is worth closing where it is noticed.
    assert_eq!(waits.len(), 2);
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
    reaching(&mut config, &listenbrainz.base);
    config.scrobbling.batch = 4;
    let state = waveflow_server::initialize(&config).await.unwrap();
    let fixture = fixture(&config, &state, "starved-listener").await;

    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
        std::sync::Arc::clone(&maloja) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::Maloja,
            "default",
            "maloja-secret",
        )
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
            "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-token",
        )
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-token",
        )
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-token",
        )
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
    // Last.fm, because the first half of this test needs a destination this
    // process has no adapter for, and a declared instance with no application
    // credentials is exactly that.
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::LastFm,
            "default",
            "lb-secret",
        )
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
        ScrobbleProvider::LastFm,
        "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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
            "default",
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
        .unlink_scrobble(fixture.owner, ScrobbleProvider::ListenBrainz, "default")
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
            "default",
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
        "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
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
        "default",
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

/// A day in milliseconds, so the retention tests read as the dates they mean.
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// Retention takes what is finished, and never what is still asking.
///
/// The queue kept everything: one row per listen and per destination, for ever,
/// on a server whose whole point is that somebody listens to music on it. The
/// bound is thirty days by default, and it is counted from the instant a row
/// became terminal — not from when it was queued, which would expire a listen
/// that took three weeks to be abandoned three weeks early.
///
/// **The three exemptions are the assertion.** `pending` has not finished,
/// `sending` is in flight, and `uncertain` is waiting for a person — taking
/// that one away after a month would answer decision 13's question in their
/// place.
#[tokio::test]
async fn retention_takes_what_is_finished_and_never_what_is_still_asking() {
    let (_temp, config, state) = test_app().await;
    let fixture = fixture(&config, &state, "retention-listener").await;
    state
        .services
        .link_scrobble(
            fixture.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();

    // One accepted, so it is `sent` and finished.
    let accepting = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&accepting) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();

    // One ambiguous, so it is `uncertain` and still asking.
    let ambiguous = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&ambiguous) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();

    // And one never drained, so it is `pending`.
    state
        .services
        .scrobble(fixture.owner, fixture.tagged, true, None)
        .await
        .unwrap();

    let states: Vec<String> = rows(&state).await.into_iter().map(|row| row.1).collect();
    assert_eq!(states, ["sent", "uncertain", "pending"]);

    // A day short of the bound takes nothing: the bound is on the finished
    // row's own instant, and that row finished today.
    let now = now_ms();
    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now + 29 * DAY_MS)
            .await
            .unwrap(),
        0,
        "nothing is old enough yet"
    );
    assert_eq!(rows(&state).await.len(), 3);

    // A day past it takes the finished one, and only that one.
    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now + 31 * DAY_MS)
            .await
            .unwrap(),
        1,
        "the finished entry is past the bound"
    );
    let states: Vec<String> = rows(&state).await.into_iter().map(|row| row.1).collect();
    assert_eq!(
        states,
        ["uncertain", "pending"],
        "a listen still asking, and one not yet tried, are not the purge's business"
    );

    // And what the link publishes did not move, because none of it is read
    // from a row a purge can take.
    let links = state.services.scrobble_links(fixture.owner).await.unwrap();
    assert_eq!(links[0].pending, 1);
    assert_eq!(links[0].uncertain, 1);
    assert!(links[0].last_success_at.is_some(), "a column of the link");
}

/// A discarded entry carries the instant it was discarded.
///
/// One of four statements that finish a row, and each is checked on its own:
/// `settle_scrobble` above, this, unlinking, and a refused token. The writer
/// that forgot the instant would leave rows no purge could ever see — the same
/// shape of silent survival `retried_at` exists to close — and a single test
/// driving all four would stop at the first.
#[tokio::test]
async fn a_discarded_entry_records_when_it_was_discarded() {
    let (_temp, config, state) = test_app().await;
    let listener = fixture(&config, &state, "discarding-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let uncertain_id = one_uncertain_entry(&state, &listener).await;
    state
        .services
        .discard_uncertain_scrobble(listener.owner, uncertain_id)
        .await
        .unwrap();

    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now_ms() + 31 * DAY_MS)
            .await
            .unwrap(),
        1
    );
    assert!(rows(&state).await.is_empty());
}

/// A queue cancelled by unlinking carries the instant it was cancelled.
#[tokio::test]
async fn an_unlinked_queue_records_when_it_was_cancelled() {
    let (_temp, config, state) = test_app().await;
    let listener = fixture(&config, &state, "unlinking-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    state
        .services
        .unlink_scrobble(listener.owner, ScrobbleProvider::ListenBrainz, "default")
        .await
        .unwrap();
    assert_eq!(rows(&state).await[0].1, "cancelled");

    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now_ms() + 31 * DAY_MS)
            .await
            .unwrap(),
        1
    );
}

/// And so does a queue a refused token finished.
///
/// The fourth writer, and the one furthest from the other three: it runs inside
/// `mark_link_broken` rather than beside a person's gesture. A link whose queue
/// stayed unpurgeable would keep growing for as long as the operator left the
/// bad token in place.
#[tokio::test]
async fn a_queue_a_refused_token_finished_records_when_it_finished() {
    let (_temp, config, state) = test_app().await;
    let listener = fixture(&config, &state, "broken-link-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::ListenBrainz,
            "default",
            "lb-secret",
        )
        .await
        .unwrap();
    let refusing = Recorder::always(ScrobbleVerdict::AuthBroken);
    state.services.register_scrobble_target(
        ScrobbleProvider::ListenBrainz,
        "default",
        std::sync::Arc::clone(&refusing) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();
    let states: Vec<String> = rows(&state).await.into_iter().map(|row| row.1).collect();
    assert_eq!(states.len(), 2);
    assert!(
        states.iter().all(|state| state == "cancelled"),
        "a broken authorisation finishes the whole queue: {states:?}"
    );

    assert_eq!(
        state
            .services
            .purge_scrobble_outbox(now_ms() + 31 * DAY_MS)
            .await
            .unwrap(),
        2,
        "every row it finished carries when it finished"
    );
}

/// A config over `temp`'s data directory declaring exactly these instances.
///
/// Called twice over one `TempDir` in the tests below, which is how a restart
/// is played: the second `initialize` opens the same database under a different
/// configuration, which is the only moment reconciliation runs.
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

/// Two instances of one recipient, and linking the second leaves the first
/// exactly where it was.
///
/// **This is the assertion the whole slice stands on.** While a live link was
/// identified by its recipient alone, `link_scrobble` withdrew "the live link
/// at this recipient" before inserting — so naming `maloja/bob` unlinked
/// `maloja/alice` and cancelled her queue. The feature would have destroyed
/// itself on its second use, quietly, and the only trace would have been
/// somebody's listens turning `cancelled`.
///
/// The unique index, the adapter registry and that clause are one change in
/// three places; correcting two of the three gives a server that accepts two
/// instances and erases one.
#[tokio::test]
async fn a_second_instance_does_not_withdraw_the_first() {
    let temp = tempfile::tempdir().unwrap();
    let config = declaring(
        &temp,
        &[
            (ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1"),
            (ScrobbleProvider::Maloja, "bob", "http://127.0.0.1:2"),
        ],
    );
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "two-instance-listener").await;

    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "alice",
            "alice-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    assert_eq!(rows(&state).await.len(), 1);

    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "bob",
            "bob-secret",
        )
        .await
        .unwrap();

    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links.len(), 2, "both instances are live");
    assert_eq!(
        links
            .iter()
            .map(|link| link.destination.as_str())
            .collect::<Vec<_>>(),
        ["alice", "bob"]
    );
    assert_eq!(
        rows(&state).await[0].1,
        "pending",
        "the first instance's queue must not have been cancelled by the second"
    );
    assert_eq!(links[0].pending, 1, "and it is still counted as hers");
}

/// Each instance is drained through its own adapter.
///
/// A registry keyed by recipient would hold one adapter for both and hand
/// alice's listens to bob's server — the substitution decision 4 builds a whole
/// tier of identifiers to prevent, arriving one floor up.
#[tokio::test]
async fn each_instance_is_submitted_through_its_own_adapter() {
    let temp = tempfile::tempdir().unwrap();
    let config = declaring(
        &temp,
        &[
            (ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1"),
            (ScrobbleProvider::Maloja, "bob", "http://127.0.0.1:2"),
        ],
    );
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "two-adapter-listener").await;
    for (destination, secret) in [("alice", "alice-secret"), ("bob", "bob-secret")] {
        state
            .services
            .link_scrobble(
                listener.owner,
                ScrobbleProvider::Maloja,
                destination,
                secret,
            )
            .await
            .unwrap();
    }

    let to_alice = Recorder::always(ScrobbleVerdict::Accepted);
    let to_bob = Recorder::always(ScrobbleVerdict::Accepted);
    state.services.register_scrobble_target(
        ScrobbleProvider::Maloja,
        "alice",
        std::sync::Arc::clone(&to_alice) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state.services.register_scrobble_target(
        ScrobbleProvider::Maloja,
        "bob",
        std::sync::Arc::clone(&to_bob) as std::sync::Arc<dyn ScrobbleTarget>,
    );

    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(drained.accepted, 2);

    // Each adapter saw exactly one submission, carrying the secret posed at it
    // and no other.
    assert_eq!(to_alice.seen().len(), 1);
    assert_eq!(to_alice.seen()[0].1, "alice-secret");
    assert_eq!(to_bob.seen().len(), 1);
    assert_eq!(to_bob.seen()[0].1, "bob-secret");
}

/// An instance the configuration no longer names breaks its links, and nothing
/// is remapped onto a sibling.
///
/// Sliding `alice` onto `bob` would send one person's listens to another
/// person's profile. And the queue has to *finish* rather than wait: a
/// `broken` link whose rows stay `pending` is a queue nothing empties —
/// `pending` escapes retention, so the table keeps them for ever and
/// `oldest_pending_at` holds the link `degraded` with them.
#[tokio::test]
async fn an_instance_that_disappears_breaks_its_link_and_finishes_its_queue() {
    let temp = tempfile::tempdir().unwrap();
    let before = declaring(
        &temp,
        &[
            (ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1"),
            (ScrobbleProvider::Maloja, "bob", "http://127.0.0.1:2"),
        ],
    );
    let state = waveflow_server::initialize(&before).await.unwrap();
    let listener = fixture(&before, &state, "vanishing-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "alice",
            "alice-secret",
        )
        .await
        .unwrap();
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    assert_eq!(rows(&state).await[0].1, "pending");
    drop(state);

    // The operator takes `alice` out of the configuration and restarts.
    let after = declaring(
        &temp,
        &[(ScrobbleProvider::Maloja, "bob", "http://127.0.0.1:2")],
    );
    let state = waveflow_server::initialize(&after).await.unwrap();

    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].destination, "alice");
    assert_eq!(links[0].health, "broken");
    assert_eq!(links[0].last_failure.as_deref(), Some("destination_gone"));
    assert_eq!(
        rows(&state).await[0].1,
        "cancelled",
        "a broken link's queue must finish, or nothing will ever empty it"
    );
}

/// A name is not an identity: moving an instance's URL breaks its links too.
///
/// The same substitution by the side door, and the easiest to commit because it
/// looks like fixing a typo. Correcting an address therefore costs relinking —
/// the price of a distinction no heuristic can make for the operator.
#[tokio::test]
async fn an_instance_that_moved_breaks_its_link_though_its_name_did_not_change() {
    let temp = tempfile::tempdir().unwrap();
    let before = declaring(
        &temp,
        &[(ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1")],
    );
    let state = waveflow_server::initialize(&before).await.unwrap();
    let listener = fixture(&before, &state, "moving-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "alice",
            "alice-secret",
        )
        .await
        .unwrap();
    drop(state);

    // Same name, different machine.
    let after = declaring(
        &temp,
        &[(ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:2")],
    );
    let state = waveflow_server::initialize(&after).await.unwrap();
    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links[0].health, "broken");
}

/// And a trailing slash is not a different machine.
///
/// A link broken by a slash an editor added while tidying a configuration file
/// would be a punishment for nothing, so the canonical form settles it rather
/// than leaving it to whoever reads the string.
#[tokio::test]
async fn a_trailing_slash_added_to_a_configuration_breaks_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let before = declaring(
        &temp,
        &[(
            ScrobbleProvider::Maloja,
            "alice",
            "http://127.0.0.1:1/maloja",
        )],
    );
    let state = waveflow_server::initialize(&before).await.unwrap();
    let listener = fixture(&before, &state, "tidied-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "alice",
            "alice-secret",
        )
        .await
        .unwrap();
    drop(state);

    let after = declaring(
        &temp,
        &[(
            ScrobbleProvider::Maloja,
            "alice",
            "http://127.0.0.1:1/maloja/",
        )],
    );
    let state = waveflow_server::initialize(&after).await.unwrap();
    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(
        links[0].health, "healthy",
        "a trailing slash is the same machine"
    );
}

/// Writes one link the way a server running before this slice wrote them.
///
/// No destination, no fingerprint — the two columns the catch-up exists to
/// fill. Inserted rather than produced, because the code that produced them is
/// the code this slice replaces.
async fn one_legacy_link(state: &AppState, owner: uuid::Uuid, provider: &str) {
    let sealed = state.secret_box.encrypt(b"a-secret-from-before").unwrap();
    let now = now_ms();
    sqlx::query(
        "INSERT INTO scrobble_link (id, user_id, provider, status, credential_nonce, \
         credential_ciphertext, created_at, updated_at) \
         VALUES (?, ?, ?, 'active', ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(owner.to_string())
    .bind(provider)
    .bind(sealed.nonce.as_slice())
    .bind(sealed.ciphertext.as_slice())
    .bind(now)
    .bind(now)
    .execute(state.db.pool())
    .await
    .unwrap();
}

/// A link written before instances had names takes the only one there is.
///
/// The catch-up asserts rather than verifies — nothing in the database says
/// which URL was configured yesterday — and that is sound exactly while there
/// is one possible answer.
#[tokio::test]
async fn a_link_from_before_this_slice_takes_the_only_instance_declared() {
    let temp = tempfile::tempdir().unwrap();
    let config = declaring(
        &temp,
        &[(ScrobbleProvider::Maloja, "house", "http://127.0.0.1:1")],
    );
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "legacy-listener").await;
    one_legacy_link(&state, listener.owner, "maloja").await;
    drop(state);

    let state = waveflow_server::initialize(&config).await.unwrap();
    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].destination, "house");
    assert_eq!(
        links[0].health, "healthy",
        "a link nothing moved must survive the catch-up"
    );
}

/// Where the answer is ambiguous, the server refuses to start.
///
/// Picking one would send a queue of waiting listens to somebody else's profile
/// with nothing to show for it. The way out needs no new machinery: boot once
/// with a single instance declared — the one those links meant — and add the
/// others next boot.
#[tokio::test]
async fn a_legacy_link_with_two_candidates_refuses_the_boot() {
    let temp = tempfile::tempdir().unwrap();
    let single = declaring(
        &temp,
        &[(ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1")],
    );
    let state = waveflow_server::initialize(&single).await.unwrap();
    let listener = fixture(&single, &state, "ambiguous-listener").await;
    one_legacy_link(&state, listener.owner, "maloja").await;
    drop(state);

    let both = declaring(
        &temp,
        &[
            (ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1"),
            (ScrobbleProvider::Maloja, "bob", "http://127.0.0.1:2"),
        ],
    );
    assert!(
        waveflow_server::initialize(&both).await.is_err(),
        "an ambiguous catch-up must stop the boot rather than guess"
    );

    // And the way out works: one instance fills the columns, and the second may
    // then be declared.
    let state = waveflow_server::initialize(&single).await.unwrap();
    drop(state);
    let state = waveflow_server::initialize(&both).await.unwrap();
    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links[0].destination, "alice");
    assert_eq!(links[0].health, "healthy");
}

/// A fresh install has nothing to catch up, so it declares as many instances as
/// it likes on the first boot.
///
/// Said out loud because the refusal above, left unqualified, would close the
/// door it exists to protect: nobody with nothing to migrate could ever use the
/// feature.
#[tokio::test]
async fn a_fresh_install_declares_several_instances_on_its_first_boot() {
    let temp = tempfile::tempdir().unwrap();
    let config = declaring(
        &temp,
        &[
            (ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1"),
            (ScrobbleProvider::Maloja, "bob", "http://127.0.0.1:2"),
        ],
    );
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "fresh-listener").await;
    for destination in ["alice", "bob"] {
        state
            .services
            .link_scrobble(
                listener.owner,
                ScrobbleProvider::Maloja,
                destination,
                "a-secret",
            )
            .await
            .unwrap();
    }
    assert_eq!(
        state
            .services
            .scrobble_links(listener.owner)
            .await
            .unwrap()
            .len(),
        2
    );
}

/// A recipient with links and no instance declared for it has nothing to
/// inscribe, so those links are treated as a vanished destination.
///
/// Emptying a URL out of the configuration is already a way of switching a
/// recipient off. Leaving the two columns empty instead would hand the next
/// reconciliation a row it could not read.
///
/// **That last sentence was the whole of this test's intent and none of its
/// assertions** until 2026-09-14: it checked that the link broke and never
/// that a name was written. The row kept a NULL destination, which came out of
/// the API as the empty string — a published pair with no instance in it — and
/// could not be withdrawn at all, because `unlink_scrobble_on` matches
/// `destination=?` and no comparison is true of NULL. A broken link with a
/// blank name and no way to remove it.
#[tokio::test]
async fn a_legacy_link_with_nothing_declared_for_it_is_broken_rather_than_left_empty() {
    let temp = tempfile::tempdir().unwrap();
    let config = declaring(
        &temp,
        &[(ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1")],
    );
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "switched-off-listener").await;
    one_legacy_link(&state, listener.owner, "lastfm").await;
    drop(state);

    let state = waveflow_server::initialize(&config).await.unwrap();
    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].provider, ScrobbleProvider::LastFm);
    assert_eq!(links[0].health, "broken");
    assert_eq!(links[0].last_failure.as_deref(), Some("destination_gone"));
    // The name a link made under #191–#193 had: its recipient declared one
    // instance and nobody had to name it, which is what `default` means.
    assert_eq!(
        links[0].destination, "default",
        "a broken link is still a pair, and the empty string is not an instance"
    );

    // And so it can be withdrawn. This is what the name buys beyond looking
    // right: the gesture addresses the pair, and a blank half addresses
    // nothing.
    assert!(
        state
            .services
            .unlink_scrobble(listener.owner, ScrobbleProvider::LastFm, "default")
            .await
            .unwrap(),
        "the listing named it, so the same name withdraws it"
    );
    assert!(state
        .services
        .scrobble_links(listener.owner)
        .await
        .unwrap()
        .is_empty());
}

/// A retry checks where it would go, not only that it may go.
///
/// The link being live is the first half — decision 4 — and the instance still
/// being the one it was made against is the second. Without it the most
/// dangerous entry in the design, a listen its owner accepts risking twice,
/// would be the one that leaves for a machine nobody chose.
///
/// The mismatch is written straight into the row rather than produced by a
/// restart, because a restart would break the link and the *first* half would
/// refuse — which would prove nothing about the second.
#[tokio::test]
async fn a_retry_refuses_an_instance_that_is_no_longer_the_one_it_was_made_against() {
    let temp = tempfile::tempdir().unwrap();
    let config = declaring(
        &temp,
        &[(ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1")],
    );
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "misaddressed-listener").await;
    state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "alice",
            "alice-secret",
        )
        .await
        .unwrap();
    let ambiguous = Recorder::always(ScrobbleVerdict::Ambiguous);
    state.services.register_scrobble_target(
        ScrobbleProvider::Maloja,
        "alice",
        std::sync::Arc::clone(&ambiguous) as std::sync::Arc<dyn ScrobbleTarget>,
    );
    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    state.services.drain_scrobble_outbox().await.unwrap();
    let uncertain_id = public_ids(&state).await[0];

    // The link stays `active`; only the machine it names has changed under it.
    sqlx::query("UPDATE scrobble_link SET destination_fingerprint='a-different-machine'")
        .execute(state.db.pool())
        .await
        .unwrap();

    assert!(
        matches!(
            state
                .services
                .retry_uncertain_scrobble(listener.owner, uncertain_id)
                .await
                .unwrap_err(),
            ServiceError::NotFound
        ),
        "a retry must not leave for a destination nobody chose"
    );
    // And the joker is not spent by the refusal: the entry is still asking, so
    // an operator who puts the address back has not cost anybody their one
    // chance.
    assert_eq!(
        state
            .services
            .uncertain_scrobbles(listener.owner)
            .await
            .unwrap()
            .len(),
        1
    );
}

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

/// A double for the half of the journey that would leave this machine.
///
/// Records the request token it was handed, so a test can assert that the one
/// Last.fm appended is the one exchanged — and not, say, a second token this
/// server asked for on its own, which is the shape of the `auth.getToken`
/// mistake the RFC names.
struct ExchangesFor {
    session_key: Result<String, ()>,
    tokens: Mutex<Vec<String>>,
}

impl ExchangesFor {
    fn key(session_key: &str) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            session_key: Ok(session_key.to_owned()),
            tokens: Mutex::new(Vec::new()),
        })
    }

    fn refusing() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            session_key: Err(()),
            tokens: Mutex::new(Vec::new()),
        })
    }

    fn tokens(&self) -> Vec<String> {
        self.tokens.lock().unwrap().clone()
    }
}

impl waveflow_server::services::LastFmSessionExchange for ExchangesFor {
    fn exchange<'a>(&'a self, token: &'a str) -> BoxFuture<'a, Result<String, ServiceError>> {
        Box::pin(async move {
            self.tokens.lock().unwrap().push(token.to_owned());
            self.session_key.clone().map_err(|()| ServiceError::Invalid)
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
    drop(state);
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
    assert!(declared[0].unavailable.unwrap().contains("application"));
    assert!(matches!(
        state
            .services
            .begin_lastfm_authorization(listener.owner, "default")
            .await
            .unwrap_err(),
        ServiceError::Unavailable
    ));
    drop(state);

    // An application, but a public address that is not `https`.
    let mut plaintext = lastfm_app(&temp);
    plaintext.public_url = Some("http://waveflow.example".to_owned());
    let state = waveflow_server::initialize(&plaintext).await.unwrap();
    let declared = state.services.scrobble_destinations();
    assert!(!declared[0].available);
    assert!(declared[0].unavailable.unwrap().contains("https"));
    assert!(matches!(
        state
            .services
            .begin_lastfm_authorization(listener.owner, "default")
            .await
            .unwrap_err(),
        ServiceError::Unavailable
    ));
    drop(state);

    // Both, and it is available.
    let state = waveflow_server::initialize(&lastfm_app(&temp))
        .await
        .unwrap();
    let declared = state.services.scrobble_destinations();
    assert!(declared[0].available);
    assert!(declared[0].unavailable.is_none());
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

/// An instance nobody declared cannot be linked, from any surface.
///
/// Decision 10's barrier seen from the service: a member picks a name the
/// server published and never describes a URL, so a name it never published is
/// a resource that is not there. Held here rather than only at the route,
/// because the CLI reaches the same method — `scrobble link` cannot be driven
/// with a secret in `tests/cli.rs`, which refuses to mutate the environment
/// while sibling threads read it, so what both surfaces share is tested where
/// it lives.
///
/// The declared name beside it is the control: without it this would pass on a
/// server that refused *every* link.
#[tokio::test]
async fn an_instance_nobody_declared_cannot_be_linked() {
    let temp = tempfile::tempdir().unwrap();
    let config = declaring(
        &temp,
        &[(ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1")],
    );
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "undeclared-listener").await;

    assert!(
        matches!(
            state
                .services
                .link_scrobble(
                    listener.owner,
                    ScrobbleProvider::Maloja,
                    "an-instance-nobody-declared",
                    "a-secret",
                )
                .await
                .unwrap_err(),
            ServiceError::NotFound
        ),
        "a name the server never published is not a destination"
    );
    // The same recipient, the name it did publish: accepted.
    assert!(state
        .services
        .link_scrobble(
            listener.owner,
            ScrobbleProvider::Maloja,
            "alice",
            "a-secret"
        )
        .await
        .is_ok());
    // Counted, then named. `all` over an empty list is true, so the shape
    // above it — "every link is alice's" — is satisfied by a server that
    // linked nothing at all, which is the one outcome this half exists to rule
    // out.
    let links = state.services.scrobble_links(listener.owner).await.unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].destination, "alice");
}

/// An ambiguous entry names the instance it was queued for, not just the
/// recipient.
///
/// Decision 13 asks a person what to do with a listen whose fate is unknown,
/// and decision 10 lets the operator declare several instances of one
/// recipient — a household where everybody self-hosts a Maloja is the ordinary
/// case, not a corner. An entry that said only `maloja` would put the question
/// without saying which of the two servers may already hold the listen, which
/// is the whole of what a person weighs before sending it again.
///
/// Both entries are made to be otherwise indistinguishable — same track, same
/// verdict, same recipient — so the destination is the only thing that can
/// tell them apart.
#[tokio::test]
async fn an_ambiguous_entry_names_which_instance_it_was_queued_for() {
    let temp = tempfile::tempdir().unwrap();
    let config = declaring(
        &temp,
        &[
            (ScrobbleProvider::Maloja, "alice", "http://127.0.0.1:1"),
            (ScrobbleProvider::Maloja, "bob", "http://127.0.0.1:2"),
        ],
    );
    let state = waveflow_server::initialize(&config).await.unwrap();
    let listener = fixture(&config, &state, "two-instance-chooser").await;
    for (destination, secret) in [("alice", "alice-secret"), ("bob", "bob-secret")] {
        state
            .services
            .link_scrobble(
                listener.owner,
                ScrobbleProvider::Maloja,
                destination,
                secret,
            )
            .await
            .unwrap();
        state.services.register_scrobble_target(
            ScrobbleProvider::Maloja,
            destination,
            Recorder::always(ScrobbleVerdict::Ambiguous) as std::sync::Arc<dyn ScrobbleTarget>,
        );
    }

    state
        .services
        .scrobble(listener.owner, listener.tagged, true, None)
        .await
        .unwrap();
    let drained = state.services.drain_scrobble_outbox().await.unwrap();
    assert_eq!(
        drained.uncertain, 2,
        "one listen, two instances, two entries"
    );

    let waiting = state
        .services
        .uncertain_scrobbles(listener.owner)
        .await
        .unwrap();
    assert_eq!(waiting.len(), 2);
    assert!(
        waiting
            .iter()
            .all(|entry| entry.provider == ScrobbleProvider::Maloja),
        "the recipient alone cannot tell them apart, which is the point"
    );
    let mut instances = waiting
        .iter()
        .map(|entry| entry.destination.as_str())
        .collect::<Vec<_>>();
    instances.sort_unstable();
    assert_eq!(
        instances,
        ["alice", "bob"],
        "each entry names the instance whose queue it is in"
    );
}
