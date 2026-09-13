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
    #[error("the destination must not carry credentials, a query or a fragment")]
    NotABareOrigin,
    #[error("the outbound client could not be built")]
    Unbuildable,
}

/// The largest response body an adapter will read.
///
/// Every destination here answers a short JSON object. The cap exists because
/// nothing else bounds what a host at the other end chooses to send, and an
/// adapter reading an unbounded body is an adapter one hostile — or merely
/// broken — destination can exhaust.
pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// The client every adapter shares.
///
/// Three properties, each of them decision 10 written down:
///
/// - **No redirect is followed.** `reqwest` follows up to ten by default, and
///   the tenth can be anywhere. A destination that answers `302` to an internal
///   address would otherwise have this server fetch it and hand back what it
///   found, which is the shape of every request-forgery there is.
/// - **The wait is bounded**, by the same value the drain uses for its own
///   backstop, so an adapter cannot outlast the deadline that is watching it.
/// - **No proxy is read from the environment.** An operator's `HTTP_PROXY`,
///   set for something else entirely, must not silently become the route every
///   listen takes.
pub fn outbound_client(timeout: Duration) -> Result<reqwest::Client, OutboundError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .connect_timeout(timeout)
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
/// A bare origin only — no credentials, no query, no fragment. A path is
/// allowed, because a self-hosted instance may well live under one.
pub fn validate_destination(raw: &str, allow_plaintext: bool) -> Result<url::Url, OutboundError> {
    let parsed = url::Url::parse(raw.trim()).map_err(|_| OutboundError::Unparseable)?;
    match parsed.scheme() {
        "https" => {}
        "http" if allow_plaintext => {}
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
