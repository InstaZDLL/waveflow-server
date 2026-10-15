//! The HTTP client every outbound request in this server is made with.
//!
//! Two subsystems call out: scrobbling (RFC-010), to destinations an operator
//! names, and metadata enrichment (RFC-011), to providers whose addresses are
//! constants. They share this client so the rules below are written once and
//! cannot drift apart — RFC-011 decision 9 adopts RFC-010 decision 10 rather
//! than restating it, and a second builder would be the place it quietly
//! stopped being true.

use std::time::Duration;

/// How much of the budget a connection may spend before the rest of the request
/// has had any of it.
///
/// A third, so a destination that accepts slowly still leaves room for the
/// request itself rather than consuming the whole deadline in the handshake.
const CONNECT_SHARE: u32 = 3;

/// The client.
///
/// Each property below is RFC-010 decision 10 written down:
///
/// - **No redirect is followed.** `reqwest` follows up to ten by default, and
///   the tenth can be anywhere. A destination that answers `302` to an internal
///   address would otherwise have this server fetch it and hand back what it
///   found, which is the shape of every request-forgery there is.
/// - **No proxy is read from the environment.** An operator's `HTTP_PROXY`,
///   set for something else entirely, must not silently become the route every
///   request takes.
/// - **Only the connection is bounded here.** The request as a whole is bounded
///   once, by the caller's own `tokio::time::timeout`, and deliberately not a
///   second time by this client.
///
/// That last one was two equal deadlines racing until a review noticed. Both
/// ended at `Ambiguous`, so the scrobbling queue looked the same either way —
/// but only the drain's own deadline records the link as stalled, and that
/// record is what stops one silent destination from spending an entry per row.
/// Which of the two fired was a coin toss, so which protection applied was a
/// coin toss.
pub fn client(timeout: Duration) -> Result<reqwest::Client, reqwest::Error> {
    builder(timeout).build()
}

/// The same rules, left open for one caller to add to: the portrait fetcher
/// pins the address it checked (`ClientBuilder::resolve`), which is a property
/// of one connection rather than of every client.
pub(crate) fn builder(timeout: Duration) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(timeout / CONNECT_SHARE)
        .no_proxy()
        .user_agent(concat!("WaveFlowServer/", env!("CARGO_PKG_VERSION")))
}
