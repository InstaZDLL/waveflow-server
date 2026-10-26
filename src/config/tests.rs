use super::access::normalize_public_url;
use super::access::parse_proxy_addresses;
use super::from_env::DEFAULT_DB_ACQUIRE_TIMEOUT_MS;
use super::metadata::parse_metadata_providers;
use super::{limits::validate_canvas, CanvasLimits};
use super::{limits::validate_uploads, UploadLimits};
use super::{scrobbling::parse_destinations, DEFAULT_SCROBBLE_DESTINATION};
use std::time::Duration;

fn workable() -> UploadLimits {
    UploadLimits {
        max_file_bytes: 1024 * 1024,
        library_quota_bytes: 4 * 1024 * 1024,
        chunk_bytes: 64 * 1024,
        batch_limit: 8,
        sessions_per_user: 2,
        session_ttl: Duration::from_secs(3600),
    }
}

/// RFC-017 decision 2: addresses and blocks alike, and a typo refuses to boot.
#[test]
fn trusted_proxies_are_read_strictly() {
    assert!(parse_proxy_addresses("").unwrap().is_empty());
    let proxies = parse_proxy_addresses(" 10.0.0.0/8, 192.168.1.5 , ::1 ").unwrap();
    assert_eq!(proxies.len(), 3);
    assert!(proxies[0].contains(&"10.1.2.3".parse::<std::net::IpAddr>().unwrap()));
    assert!(proxies[1].contains(&"192.168.1.5".parse::<std::net::IpAddr>().unwrap()));
    assert!(!proxies[1].contains(&"192.168.1.6".parse::<std::net::IpAddr>().unwrap()));
    let mapped = parse_proxy_addresses("::ffff:10.0.0.5").unwrap();
    assert!(mapped[0].contains(&"10.0.0.5".parse::<std::net::IpAddr>().unwrap()));
    assert!(parse_proxy_addresses("10.0.0.0/33").is_err());
    assert!(parse_proxy_addresses("proxy.local").is_err());
}

/// RFC-011 decision 3: empty is off, and every typo refuses to boot rather
/// than starting enrichment narrower than the operator wrote.
#[test]
fn metadata_providers_are_read_strictly() {
    use crate::remote::enrichment::Provider;

    assert_eq!(parse_metadata_providers("", false, false).unwrap(), []);
    assert_eq!(parse_metadata_providers(" , ", false, false).unwrap(), []);
    assert_eq!(
        parse_metadata_providers(" lastfm ", true, false).unwrap(),
        [Provider::LastFm]
    );
    // No key ships in the binary: naming the provider without one is a
    // misconfiguration, not a quiet off.
    assert!(parse_metadata_providers("lastfm", false, false).is_err());
    assert!(parse_metadata_providers("lastfm,lastfm", true, false).is_err());
    assert!(parse_metadata_providers("last.fm", true, false).is_err());
    // Deezer asks for no key, so none is required of it.
    assert_eq!(
        parse_metadata_providers("lastfm, deezer", true, false).unwrap(),
        [Provider::LastFm, Provider::Deezer]
    );
    assert_eq!(
        parse_metadata_providers("deezer", false, false).unwrap(),
        [Provider::Deezer]
    );
    // TheAudioDB asks for a key of its own (RFC-014 decision 1), and the
    // Last.fm one does not stand in for it.
    assert!(parse_metadata_providers("theaudiodb", true, false).is_err());
    assert_eq!(
        parse_metadata_providers("deezer, theaudiodb", false, true).unwrap(),
        [Provider::Deezer, Provider::TheAudioDb]
    );
}

/// RFC-024 decision 1: the list is the switch, and a typo refuses to boot
/// rather than leaving lyrics quietly off.
#[test]
fn lyrics_providers_are_read_strictly() {
    use crate::remote::lrclib::LyricsProvider;

    assert_eq!(super::metadata::parse_lyrics_providers("").unwrap(), []);
    assert_eq!(super::metadata::parse_lyrics_providers(" , ").unwrap(), []);
    assert_eq!(
        super::metadata::parse_lyrics_providers(" lrclib ").unwrap(),
        [LyricsProvider::Lrclib]
    );
    assert!(super::metadata::parse_lyrics_providers("lrclib,lrclib").is_err());
    assert!(super::metadata::parse_lyrics_providers("lrclib.net").is_err());
    // An artist provider is not a lyrics provider.
    assert!(super::metadata::parse_lyrics_providers("lastfm").is_err());
}

#[test]
fn upload_limits_that_contradict_each_other_are_refused() {
    assert!(validate_uploads(&workable()).is_ok());

    // A fragment bigger than the largest file it could belong to.
    let mut oversized_chunk = workable();
    oversized_chunk.chunk_bytes = oversized_chunk.max_file_bytes + 1;
    assert!(validate_uploads(&oversized_chunk).is_err());

    // A file bigger than the whole library may hold.
    let mut oversized_file = workable();
    oversized_file.max_file_bytes = oversized_file.library_quota_bytes + 1;
    assert!(validate_uploads(&oversized_file).is_err());

    // And a fragment this platform could not turn into a body ceiling. The
    // fallback for that conversion must never be "no ceiling", so the value
    // has to be refused before it reaches one.
    if usize::try_from(i64::MAX).is_err() {
        let mut unrepresentable = workable();
        unrepresentable.chunk_bytes = i64::MAX;
        unrepresentable.max_file_bytes = i64::MAX;
        unrepresentable.library_quota_bytes = i64::MAX;
        assert!(validate_uploads(&unrepresentable).is_err());
    }
}

#[test]
fn canvas_limits_that_contradict_each_other_are_refused() {
    let workable = CanvasLimits {
        max_bytes: 256 * 1024,
        max_duration_secs: 15,
        library_quota_bytes: 1024 * 1024,
    };
    assert!(validate_canvas(&workable).is_ok());

    // A single canvas larger than the whole library may hold: the first one
    // placed would be refused by a quota it can never fit under, which is a
    // misconfiguration rather than a verdict.
    let mut oversized = workable;
    oversized.max_bytes = oversized.library_quota_bytes + 1;
    assert!(validate_canvas(&oversized).is_err());

    // And a ceiling this platform cannot represent, for the same reason the
    // upload chunk size is refused: every fallback for a ceiling is wrong.
    if usize::try_from(i64::MAX).is_err() {
        let unrepresentable = CanvasLimits {
            max_bytes: i64::MAX,
            max_duration_secs: 15,
            library_quota_bytes: i64::MAX,
        };
        assert!(validate_canvas(&unrepresentable).is_err());
    }
}

#[test]
fn public_url_is_reduced_to_a_safe_http_origin() {
    assert_eq!(
        normalize_public_url("https://music.example.com:8443/").unwrap(),
        "https://music.example.com:8443"
    );
    for invalid in [
        "ftp://music.example.com",
        "https://user:secret@music.example.com",
        "https://music.example.com/waveflow",
        "https://music.example.com?token=secret",
    ] {
        assert!(normalize_public_url(invalid).is_err(), "{invalid}");
    }
}

/// A destination that cannot be used is never quoted back.
///
/// `validate_destination` refuses a URL carrying credentials, so the value
/// this error branch is most likely to be holding is precisely the one that
/// must not be printed — and a startup error goes to a log, a terminal, and
/// whatever a person pastes into an issue. The repository's rule about
/// secrets has no exception for an error path.
///
/// It used to live in `tests/scrobbling.rs` against `initialize`, which is
/// where the guard used to be. Named destinations moved the refusal here,
/// to the reading of the variable, and the test follows the guard rather
/// than staying where it was and passing for another reason.
#[test]
fn a_refused_destination_is_never_quoted_back_with_its_credentials() {
    // A private host, so the plaintext rule lets this through and the
    // credential rule is what refuses it. That is the path carrying a
    // password.
    let refused = parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "house=http://wf-user:hunter2@10.0.0.2",
        true,
    )
    .expect_err("a destination carrying credentials must be refused");
    let said = format!("{refused:#}");

    // **Named, never quoted.** An earlier version of this loop put the
    // secret and the whole error into the assertion message, and CodeQL was
    // right to call that a cleartext write of a credential — in the one
    // test whose entire subject is that credentials must not be written.
    // What a failure here needs to say is *which* part came back, not the
    // part itself.
    for (part, secret) in [
        ("the password", "hunter2"),
        ("the account", "wf-user"),
        ("the host", "10.0.0.2"),
    ] {
        assert!(
            !said.contains(secret),
            "the startup error quoted {part} back from the destination"
        );
    }
    // And it still says enough to be fixed by the person who typed it:
    // which variable, which name, and what was wrong with it. Printing
    // `said` is safe here and only here — the loop above has just
    // established that it carries none of the three.
    assert!(
        said.contains("credentials"),
        "the error must name the fault: {said}"
    );
    assert!(
        said.contains("house") && said.contains("WAVEFLOW_SCROBBLE_MALOJA_URL"),
        "the error must name the entry that is wrong: {said}"
    );

    // **And a password containing `=` does not become the name.** The first
    // version of the parser split on the first `=` wherever it fell, so
    // this entry became the name `http://wf-user:pa` and the address
    // `ss@10.0.0.2` — and the name is what the refusal quotes back. Half a
    // password in a startup log, from the one paragraph that promises the
    // opposite. Nothing about the shape of this entry is unusual; `=` is an
    // ordinary character in a generated password.
    let refused = parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "http://wf-user:pa=ss@10.0.0.2",
        true,
    )
    .expect_err("a destination carrying credentials must be refused");
    let said = format!("{refused:#}");
    for (part, secret) in [
        ("the password", "pa=ss"),
        ("part of the password", "pa"),
        ("the account", "wf-user"),
        ("the host", "10.0.0.2"),
    ] {
        assert!(
            !said.contains(secret),
            "the startup error quoted {part} back from the destination"
        );
    }
    assert!(
        said.contains("credentials"),
        "the error must still name the fault"
    );
}

/// A mistyped *name* is still reported by name.
///
/// The guard above reads an entry as a bare address whenever the text
/// before the `=` could not be a name. That must not swallow the ordinary
/// typo it exists beside: a name with a space in it carries neither `:` nor
/// `/`, so it is still a name, and the operator is told which one.
#[test]
fn a_mistyped_name_is_still_named_in_the_refusal() {
    let refused = parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "al ice=https://host/alice",
        false,
    )
    .expect_err("a name with a space must be refused");
    let said = format!("{refused:#}");
    assert!(said.contains("al ice"), "{said}");
}

/// Two spellings of one machine are one destination.
///
/// The trailing slash is the one an editor adds without anybody deciding
/// to, and a fingerprint that moved with it would break every link on a
/// server at the first restart after a configuration tidy-up.
#[test]
fn a_trailing_slash_does_not_move_a_destination() {
    let bare = parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "https://host/maloja",
        false,
    )
    .expect("a valid destination");
    let slashed = parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "https://host/maloja/",
        false,
    )
    .expect("a valid destination");
    assert_eq!(bare[0].fingerprint, slashed[0].fingerprint);

    // And the path is *part* of the identity, unlike an origin. Two tenants
    // on one host are two destinations, and giving them one fingerprint
    // would let the identity guard wave through the likeliest move there
    // is.
    let other_tenant = parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "https://host/maloja-two",
        false,
    )
    .expect("a valid destination");
    assert_ne!(bare[0].fingerprint, other_tenant[0].fingerprint);
}

/// A recipient may carry several instances, and neither name nor address
/// may repeat.
#[test]
fn a_recipient_carries_several_instances_each_named_once() {
    let declared = parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "alice=https://host/alice, bob=https://host/bob",
        false,
    )
    .expect("two instances");
    assert_eq!(declared.len(), 2);
    assert_eq!(declared[0].name, "alice");
    assert_eq!(declared[1].name, "bob");

    // A bare URL is the shorthand for one instance called `default`, so the
    // two spellings produce the same row and adding a second instance later
    // does not rename the first.
    let shorthand = parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "https://host/alice",
        false,
    )
    .expect("one instance");
    assert_eq!(shorthand[0].name, DEFAULT_SCROBBLE_DESTINATION);

    // Two names for one machine would give a member two links to keep in
    // step with one profile, and each would look like a separate
    // destination in every counter.
    assert!(parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "alice=https://host/alice, ALICE=https://host/alice/",
        false,
    )
    .is_err());

    // And one name for two machines is not a configuration either.
    assert!(parse_destinations(
        crate::services::ScrobbleProvider::Maloja,
        "WAVEFLOW_SCROBBLE_MALOJA_URL",
        "alice=https://host/alice, alice=https://host/bob",
        false,
    )
    .is_err());
}

/// A name that cannot be a path segment is refused where it is written.
#[test]
fn a_destination_name_is_bounded_by_what_crosses_a_path() {
    for refused in [
        ".",
        "..",
        "al ice",
        "alice/bob",
        "alice%2f",
        "",
        &"a".repeat(65),
    ] {
        assert!(
            parse_destinations(
                crate::services::ScrobbleProvider::Maloja,
                "WAVEFLOW_SCROBBLE_MALOJA_URL",
                &format!("{refused}=https://host/alice"),
                false,
            )
            .is_err(),
            "{refused:?} must not become a path segment"
        );
    }
    for accepted in ["alice", "a-b_c.d", "ALICE2", &"a".repeat(64)] {
        assert!(
            parse_destinations(
                crate::services::ScrobbleProvider::Maloja,
                "WAVEFLOW_SCROBBLE_MALOJA_URL",
                &format!("{accepted}=https://host/alice"),
                false,
            )
            .is_ok(),
            "{accepted:?} must be usable"
        );
    }
}

/// The test fixture waits longer for a cold database than a deployment.
///
/// `Database::open` creates the file and establishes the connection the
/// migrations run on, and a test target boots one whole server per test,
/// so several cold databases are created at once on a machine that is
/// also compiling. Three targets — uploads, media, scrobbling — have each
/// failed that acquire on the Windows runner at different times, naming
/// whichever test was unlucky and looking like three unrelated flakes.
///
/// Pinned because the repair is one literal wide: a later tidy-up that
/// gives the fixture the production budget back restores a lottery whose
/// losing ticket is drawn in a different file every time.
#[test]
fn the_test_fixture_waits_longer_for_a_cold_database_than_a_deployment() {
    let fixture = super::Config::for_data_dir(std::path::PathBuf::from("data"));
    let deployment = Duration::from_millis(DEFAULT_DB_ACQUIRE_TIMEOUT_MS);
    assert!(
        fixture.db_acquire_timeout > deployment,
        "the fixture waits {:?}, a deployment {deployment:?} — a test boots a server per \
         test and must not inherit a budget tuned for one warm server",
        fixture.db_acquire_timeout
    );
}
