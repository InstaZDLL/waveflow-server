//! Declared instances: several per recipient, links that outlive or lose
//! theirs, and legacy links.

use super::*;

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
    stop(state).await;

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
    stop(state).await;

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
    stop(state).await;

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
    stop(state).await;

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
    stop(state).await;

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
    stop(state).await;
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
    stop(state).await;

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
