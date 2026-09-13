//! The outbound half of RFC-010: the adapters that actually speak to somebody.
//!
//! Apart from `src/services/` on purpose. The domain owns the queue and knows
//! five words; these know one destination each and nothing about the queue.
//! That line is what decision 6 exists to draw, and putting an adapter beside
//! the drain would rub it out within a release.
//!
//! **This is the only place in the server that makes an outbound request.**
//! Decision 10 calls that the real risk of the whole RFC — a server that calls
//! a URL is a server somebody can make call a URL — so the rules live here,
//! once, rather than in each adapter.

use std::time::Duration;

pub mod listenbrainz;

/// Why a destination was refused before anything was sent to it.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum OutboundError {
    #[error("the destination is not a valid URL")]
    Unparseable,
    #[error("the destination must be https")]
    NotHttps,
    #[error("plaintext is only allowed to a host on the operator's own network")]
    PlaintextToPublicHost,
    #[error("the destination must not carry credentials, a query or a fragment")]
    NotABareOrigin,
    #[error("the outbound client could not be built")]
    Unbuildable,
}

/// How much of the budget a connection may spend before the rest of the request
/// has had any of it.
///
/// A third, so a destination that accepts slowly still leaves room for the
/// submission itself rather than consuming the whole deadline in the handshake.
const CONNECT_SHARE: u32 = 3;

/// The client every adapter shares.
///
/// Each property below is decision 10 written down:
///
/// - **No redirect is followed.** `reqwest` follows up to ten by default, and
///   the tenth can be anywhere. A destination that answers `302` to an internal
///   address would otherwise have this server fetch it and hand back what it
///   found, which is the shape of every request-forgery there is.
/// - **No proxy is read from the environment.** An operator's `HTTP_PROXY`,
///   set for something else entirely, must not silently become the route every
///   listen takes.
/// - **Only the connection is bounded here.** The request as a whole is bounded
///   once, by the drain's own `tokio::time::timeout`, and deliberately not a
///   second time by this client.
///
/// That last one was two equal deadlines racing until a review noticed. Both
/// ended at `Ambiguous`, so the queue looked the same either way — but only the
/// drain's own deadline records the link as stalled, and that record is what
/// stops one silent destination from spending an entry per row. Which of the two
/// fired was a coin toss, so which protection applied was a coin toss.
pub fn outbound_client(timeout: Duration) -> Result<reqwest::Client, OutboundError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(timeout / CONNECT_SHARE)
        .no_proxy()
        .user_agent(concat!("WaveFlowServer/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| OutboundError::Unbuildable)
}

/// Checks a destination an operator configured, before it is ever called.
///
/// `allow_plaintext` is the one escape, and it belongs to the operator rather
/// than to a member: Maloja and ListenBrainz self-host, and a server talking to
/// a container on its own network over plain HTTP is a reasonable thing that
/// this rule must not forbid. What it must forbid is a *default* that quietly
/// sends a personal token in clear across somebody's internet.
///
/// **And the escape is now no wider than that justification.** A review pointed
/// out that `submit` sends `Authorization: Token …` on the first request, so
/// plaintext to a *public* host is cleartext transmission of somebody's
/// credential — CWE-319. Decision 10 had always read "sauf pour une cible
/// explicitement déclarée par l'opérateur **en clair sur son propre réseau**",
/// and the paragraph above said the same; neither was ever checked, so the flag
/// bought plaintext to anywhere. The reviewer asked for HTTP to be refused
/// outright, which would remove the self-hosted case the decision exists to
/// permit. Making the code enforce the sentence it already claimed is the
/// narrower and truer fix.
///
/// Judged on the literal, never resolved: a name that resolves privately but
/// does not look private is refused, and the operator writes the address
/// instead. Guessing from DNS would make this answer depend on what a resolver
/// said at boot.
///
/// A bare origin only — no credentials, no query, no fragment. A path is
/// allowed, because a self-hosted instance may well live under one.
pub fn validate_destination(raw: &str, allow_plaintext: bool) -> Result<url::Url, OutboundError> {
    let parsed = url::Url::parse(raw.trim()).map_err(|_| OutboundError::Unparseable)?;
    match parsed.scheme() {
        "https" => {}
        "http" if allow_plaintext => {
            if !is_on_our_own_network(parsed.host()) {
                return Err(OutboundError::PlaintextToPublicHost);
            }
        }
        _ => return Err(OutboundError::NotHttps),
    }
    if !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.host().is_none()
    {
        return Err(OutboundError::NotABareOrigin);
    }
    Ok(parsed)
}

/// Whether a host is plausibly on the operator's own network.
///
/// The addresses are the ones that cannot be routed across the internet:
/// loopback, the three private IPv4 ranges, link-local, and their IPv6
/// equivalents — unique-local `fc00::/7` and link-local `fe80::/10`, spelled out
/// by hand because `Ipv6Addr`'s own predicates for those are still unstable.
/// **An IPv4-mapped address is unwrapped and judged as the address it names**,
/// so `[::ffff:10.0.0.1]` and `10.0.0.1` get the same answer. A review found
/// them getting opposite ones: the refusal was safe, but it told an operator
/// their own network was public, which is the worst kind of true-sounding lie
/// for a boot failure to tell.
///
/// The deprecated IPv4-*compatible* spelling is a different thing and stays
/// refused: `to_ipv4_mapped` answers only for `::ffff:a.b.c.d`, so `[::10.0.0.1]`
/// falls through to the segment tests and fails them. It is now the one IPv6
/// spelling of a private address that disagrees with its IPv4 twin, and it is
/// left that way deliberately — the form is deprecated, nothing writes it on
/// purpose, and refusing is the safe direction. Said here because the paragraph
/// above, read alone, promises more agreement than there is.
///
/// The names are `localhost` and the mDNS and intranet suffixes — `.local`,
/// `.internal`, `.home.arpa` — and nothing else.
///
/// **A bare single label was accepted here and is not any more.** The argument
/// for it was that `http://maloja` is what a container is called on a Docker
/// network, which is true. The argument against it is stronger, and a review had
/// to make it twice before it landed: **a single label is completed by the
/// resolver's own search list.** `http://maloja` on a host configured with
/// `search corp.example.com` names a public address, and the literal does not
/// say which. That is precisely the DNS dependence the paragraph below refuses —
/// it was sitting inside the rule that refuses it. `com` and `ai` are single
/// labels too, and they answer publicly.
///
/// An operator on a container network writes the address, or a name under one of
/// the suffixes above. That is a smaller cost than a personal token in clear to
/// wherever a search domain happened to point.
///
/// A name made only of dots trims to the empty string, which contains no dot and
/// so read as ours. Refused — nobody's network is called that, and accepting it
/// was an accident of how the trimming is written rather than a decision.
///
/// A public name is refused even if it happens to resolve to `10.0.0.2` today.
/// That is the deliberate half: a check that asked a resolver would answer
/// differently depending on when it was asked, and a DNS answer is not a thing
/// to hang a credential on.
fn is_on_our_own_network(host: Option<url::Host<&str>>) -> bool {
    fn unroutable(address: std::net::Ipv4Addr) -> bool {
        address.is_loopback() || address.is_private() || address.is_link_local()
    }
    match host {
        Some(url::Host::Ipv4(address)) => unroutable(address),
        Some(url::Host::Ipv6(address)) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return unroutable(mapped);
            }
            let leading = address.segments()[0];
            address.is_loopback() || (leading & 0xfe00) == 0xfc00 || (leading & 0xffc0) == 0xfe80
        }
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            if name.is_empty() {
                return false;
            }
            name == "localhost"
                || [".localhost", ".local", ".internal", ".home.arpa"]
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
        }
        None => false,
    }
}

/// Joins a path onto a configured destination without letting the path escape.
///
/// `Url::join` treats a leading `/` as "replace the whole path", which would
/// quietly undo an operator's `https://host/listenbrainz` prefix, and a `..`
/// segment can climb out of it. Neither is reachable from user input today —
/// every caller passes a literal — and that is exactly why it is worth closing
/// now, while the only paths are ones we wrote.
pub fn endpoint(base: &url::Url, path: &str) -> Result<url::Url, OutboundError> {
    let mut joined = base.clone();
    {
        let mut segments = joined
            .path_segments_mut()
            .map_err(|()| OutboundError::NotABareOrigin)?;
        // Drops the empty segment a trailing slash leaves behind, so
        // `https://host/` and `https://host` produce the same endpoint.
        segments.pop_if_empty();
        for segment in path.split('/').filter(|segment| !segment.is_empty()) {
            if segment == ".." || segment == "." {
                return Err(OutboundError::NotABareOrigin);
            }
            segments.push(segment);
        }
    }
    Ok(joined)
}

#[cfg(test)]
mod tests {
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
}
