//! The HTTP client every outbound request in this server is made with.
//!
//! Two subsystems call out: scrobbling (RFC-010), to destinations an operator
//! names, and metadata enrichment (RFC-011), to providers whose addresses are
//! constants. They share this client so the rules below are written once and
//! cannot drift apart — RFC-011 decision 9 adopts RFC-010 decision 10 rather
//! than restating it, and a second builder would be the place it quietly
//! stopped being true.
//!
//! The same goes for what comes back: [`read_bounded_body`] is how a body from
//! another host is read, under a ceiling, whoever asked for it (RFC-023
//! decision 1).

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

/// The most an answer from another host may weigh when nothing narrower
/// applies (RFC-023 decision 1).
///
/// The figure the metadata clients of `waveflow-core` already hold every
/// provider to (`metadata::MAX_RESPONSE_BYTES`). The largest real answer this
/// server reads — a Last.fm session, a Maloja acknowledgement — is a few
/// hundred bytes, so the ceiling never trims a genuine one; what it stops is a
/// destination, broken or hostile, that answers with a body this process would
/// otherwise hold whole.
pub const MAX_ANSWER_BYTES: usize = 2 * 1024 * 1024;

/// Why [`read_bounded_body`] produced no body.
///
/// **Carries nothing from the request or the answer.** A `reqwest::Error`
/// prints the URL it was raised on, and a Last.fm URL carries the operator's
/// key; an answer quotes the submission, and on the authorisation path the
/// token. A caller logs one of these two words and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BodyError {
    /// The connection failed, or the body broke off, before it was whole.
    #[error("the answer could not be read")]
    Unread,
    /// The answer announced, or reached, more than the ceiling it was read
    /// under, and the rest of it was not read.
    #[error("the answer is over the size ceiling")]
    TooLarge,
}

/// A response body read whole, unless it outgrows `limit` bytes.
///
/// Every body this server reads from another host goes through here or through
/// a reader that does the same (`waveflow-core`'s metadata clients, the
/// portrait fetcher's image read). Reusable by anything that fetches: it knows
/// nothing about who asked.
///
/// - **An announced giant is refused unread.** `Content-Length` is checked
///   before the first byte, so a destination that says what it is about to send
///   costs nothing.
/// - **The running count covers the rest**: an answer that announces nothing,
///   or lies, is cut off as soon as it crosses the line, and the connection is
///   dropped with the response.
/// - **Size only, not time.** A body that trickles in stays within the
///   caller's own deadline, which every caller has (RFC-010 decision 10); a
///   second clock here would race it, as `client` explains.
pub async fn read_bounded_body(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, BodyError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(BodyError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| BodyError::Unread)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(BodyError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
pub(crate) mod tests;
