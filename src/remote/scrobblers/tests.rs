use super::*;

#[test]
fn a_destination_must_be_https_unless_the_operator_said_otherwise() {
    assert!(validate_destination("https://api.listenbrainz.org", false).is_ok());

    // The default refuses plaintext, because the request carries a personal
    // token and the operator has not said this is their own network.
    assert_eq!(
        validate_destination("http://maloja.local", false),
        Err(OutboundError::NotHttps)
    );
    // And permits it when they have.
    assert!(validate_destination("http://maloja.local", true).is_ok());

    // Nothing else is a destination.
    for refused in ["ftp://host", "file:///etc/passwd", "not a url"] {
        assert!(validate_destination(refused, true).is_err(), "{refused}");
    }
}

/// The escape reaches exactly as far as the sentence that justifies it.
#[test]
fn plaintext_is_allowed_only_towards_the_operators_own_network() {
    // The shapes decision 10 carved it for: a container named on a Docker
    // network, the intranet suffixes, and the addresses that cannot be
    // routed across the internet.
    for own in [
        "http://maloja.local",
        "http://listens.internal",
        "http://listens.home.arpa",
        "http://127.0.0.1:8080",
        "http://10.0.0.2",
        "http://172.16.4.9",
        "http://192.168.1.50",
        "http://169.254.7.7",
        "http://[::1]:8080",
        "http://[fd00::1]",
        "http://[fe80::1]",
        // The same private addresses written the IPv4-mapped way. The doc
        // above promises "their IPv6 equivalents", and an operator who
        // spells one like this was getting a boot failure saying their own
        // network was public.
        "http://[::ffff:127.0.0.1]",
        "http://[::ffff:10.0.0.1]",
        "http://[::ffff:c0a8:1]",
    ] {
        assert!(validate_destination(own, true).is_ok(), "{own}");
    }

    // And never a public host, whatever the flag says: the first request
    // to it would carry `Authorization: Token …` in clear.
    for public in [
        // A bare label is completed by the resolver's search list, so this
        // one names whatever `search` says it does — which the string
        // itself cannot tell us. It was accepted until a review made the
        // point twice.
        "http://maloja",
        "http://api.listenbrainz.org",
        "http://maloja.example.com",
        "http://1.1.1.1",
        "http://[2606:4700::1111]",
        // A trailing dot and a capital spell the same public name.
        "http://API.ListenBrainz.ORG.",
        // And the mapped form of a public address is still public.
        "http://[::ffff:1.1.1.1]",
        // A name that is nothing but dots trims to the empty string, which
        // has no dot in it and was therefore reading as "ours". Nobody's
        // network is called that; accepting it was an accident of the test
        // rather than a decision.
        "http://.",
        "http://..",
    ] {
        assert_eq!(
            validate_destination(public, true),
            Err(OutboundError::PlaintextToPublicHost),
            "{public}"
        );
    }

    // HTTPS reaches all of them, which is the point: the rule is about a
    // credential crossing in clear, not about where it is going.
    assert!(validate_destination("https://api.listenbrainz.org", false).is_ok());
}

#[test]
fn a_destination_carries_no_credentials_query_or_fragment() {
    for refused in [
        "https://user:secret@host",
        "https://host?token=secret",
        "https://host#fragment",
    ] {
        assert_eq!(
            validate_destination(refused, false),
            Err(OutboundError::NotABareOrigin),
            "{refused}"
        );
    }
    // A path is fine: a self-hosted instance may live under one.
    assert!(validate_destination("https://host/listenbrainz", false).is_ok());
}

#[test]
fn an_endpoint_is_appended_and_never_escapes_the_configured_prefix() {
    let base = validate_destination("https://host/listenbrainz", false).unwrap();
    assert_eq!(
        endpoint(&base, "/1/submit-listens").unwrap().as_str(),
        "https://host/listenbrainz/1/submit-listens",
        "a leading slash must not replace the operator's prefix"
    );

    // The same answer whether or not the operator typed a trailing slash.
    let slashed = validate_destination("https://host/listenbrainz/", false).unwrap();
    assert_eq!(
        endpoint(&slashed, "1/submit-listens").unwrap().as_str(),
        endpoint(&base, "1/submit-listens").unwrap().as_str()
    );

    // And no path climbs out of it.
    assert!(endpoint(&base, "../elsewhere").is_err());
}
