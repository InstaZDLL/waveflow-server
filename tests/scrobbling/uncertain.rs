//! Uncertain entries: listing them, and the retry and discard a person answers
//! them with.

use super::*;

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
