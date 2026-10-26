//! Last.fm, the destination that authorises rather than pastes.
//!
//! Third by RFC-010 decision 11, and last for a reason: it is the only one of
//! the three that hands out no secret a person can paste. What a member holds
//! at the end is a *session key*, obtained through the journey in
//! `src/services/lastfm.rs`, and this file only knows what to do with one.
//!
//! Two things make this adapter unlike its neighbours:
//!
//! - **Every call is signed.** The application's shared secret never leaves
//!   this server, and the signature is what proves a request came from it. The
//!   secret is therefore in the signature and never in the body.
//! - **The application belongs to the deployment.** Decision 4 forbids shipping
//!   any provider credential in an AGPL binary, so an operator registers their
//!   own and Last.fm is simply unavailable until they do.
//!
//! Like its neighbours, nothing here knows the queue exists.

use std::time::Duration;

use futures_util::future::BoxFuture;

use crate::config::LastFmApplication;
use crate::services::{ScrobbleEnvelope, ScrobbleTarget, ScrobbleVerdict};

use super::{endpoint, OutboundError};

pub struct LastFm {
    client: reqwest::Client,
    endpoint: url::Url,
    application: LastFmApplication,
    /// The whole of one authorisation call, its answer included.
    ///
    /// A listen is bounded by the drain's own deadline. The two calls of the
    /// authorisation journey are made for a person waiting on a page or a
    /// terminal, and nothing above them bounded them at all (RFC-023
    /// decision 1). It is the `WAVEFLOW_SCROBBLE_REQUEST_TIMEOUT_SECS` the
    /// drain uses.
    deadline: Duration,
}

impl LastFm {
    pub fn new(
        client: reqwest::Client,
        base: &url::Url,
        application: LastFmApplication,
        deadline: Duration,
    ) -> Result<Self, OutboundError> {
        // **The trailing slash is kept, and it is not decoration.** Last.fm
        // documents its endpoint as `/2.0/`, answers `/2.0` with a redirect,
        // and `outbound_client` follows none — so dropping it would make every
        // call fail with an answer this adapter reads as "the destination
        // moved". `endpoint` pops the empty segment a trailing slash leaves
        // behind, which is right for the two destinations that do not want one,
        // so it is put back here rather than by weakening that rule.
        let mut submit = endpoint(base, "2.0")?;
        submit
            .path_segments_mut()
            .map_err(|()| OutboundError::NotABareOrigin)?
            .push("");
        Ok(Self {
            client,
            endpoint: submit,
            application,
            deadline,
        })
    }

    /// One signed authorisation call, and the narrow answer it is read into.
    ///
    /// Bounded twice, which is the whole of RFC-023 decision 1 for this path:
    /// in time by [`Self::deadline`], and in size by
    /// [`crate::remote::outbound::read_bounded_body`]. Either refusal is
    /// `Unavailable`, a failure of the way to Last.fm rather than of what the
    /// person brought.
    ///
    /// **The status, and never the body, on a refusal.** This answer is the
    /// one exception decision 10's "log what came back" rule must carve out:
    /// it carries the session key itself on success, and on failure it quotes
    /// the request token or the application key back in its error text.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        params: &[(&str, String)],
        what: &'static str,
    ) -> Result<T, crate::services::ServiceError> {
        use crate::services::ServiceError;
        let answer = async {
            let response = self
                .client
                .post(self.endpoint.clone())
                .form(params)
                .send()
                .await
                .map_err(|_| {
                    tracing::warn!(call = what, "last.fm was not reached");
                    ServiceError::Unavailable
                })?;
            let status = response.status();
            if !status.is_success() {
                tracing::warn!(call = what, %status, "last.fm refused the call");
                return Err(ServiceError::Invalid);
            }
            let body = crate::remote::outbound::read_bounded_body(
                response,
                crate::remote::outbound::MAX_ANSWER_BYTES,
            )
            .await
            .map_err(|error| {
                tracing::warn!(call = what, %error, "last.fm's answer was not read");
                ServiceError::Unavailable
            })?;
            serde_json::from_slice::<T>(&body).map_err(|_| {
                tracing::warn!(call = what, "last.fm's answer is not the expected JSON");
                ServiceError::Unavailable
            })
        };
        tokio::time::timeout(self.deadline, answer)
            .await
            .unwrap_or_else(|_| {
                tracing::warn!(call = what, "last.fm did not answer in time");
                Err(ServiceError::Unavailable)
            })
    }
}

/// The signature Last.fm requires on every authenticated call.
///
/// Its own rule, spelled out: sort the parameters by name, concatenate name and
/// value with nothing between them, append the shared secret, and take the MD5
/// of the UTF-8 bytes. `format` and `callback` are excluded — they are not part
/// of the call, only of how the answer is shaped.
///
/// **MD5 because Last.fm says MD5.** It is not a security choice this server
/// gets to make: a signature computed any other way is one they reject. What it
/// does mean is that the digest carries no weight here beyond matching theirs,
/// and nothing else in this crate may take it as a precedent.
pub(super) fn signature(params: &[(&str, String)], secret: &str) -> String {
    use md5::Digest as _;
    let mut sorted: Vec<&(&str, String)> = params
        .iter()
        .filter(|(name, _)| *name != "format" && *name != "callback")
        .collect();
    sorted.sort_by(|left, right| left.0.cmp(right.0));
    let mut hasher = md5::Md5::new();
    for (name, value) in sorted {
        hasher.update(name.as_bytes());
        hasher.update(value.as_bytes());
    }
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// One listen, in Last.fm's own vocabulary.
///
/// `track.scrobble` takes indexed arrays for a batch — `artist[0]`, `track[0]`
/// and so on — and the unindexed spelling for a single listen. Decision 11
/// sends one listen per request, so there is never an index to write.
///
/// The album artist and the recording identifier are sent when the envelope has
/// them: Last.fm matches on names and the MusicBrainz id is what disambiguates
/// two recordings that share one.
fn scrobble_params(envelope: &ScrobbleEnvelope, session_key: &str) -> Vec<(&'static str, String)> {
    let mut params = vec![
        ("method", "track.scrobble".to_owned()),
        // Every credit, in tag order, as the one string Last.fm takes. Sending
        // only the first would match more often and would quietly drop a
        // collaborator from somebody's history — the envelope records what was
        // heard, and a guest who was on the track was heard.
        ("artist", envelope.artists.join(", ")),
        ("track", envelope.title.clone()),
        // **Seconds**, not milliseconds. The envelope carries epoch
        // milliseconds like every other timestamp in this server, and handing
        // those over unconverted would date every listen about fifty thousand
        // years from now — a mistake a listening history keeps for good.
        (
            "timestamp",
            envelope.played_at.div_euclid(1_000).to_string(),
        ),
        ("sk", session_key.to_owned()),
    ];
    if let Some(album) = &envelope.album {
        params.push(("album", album.clone()));
    }
    if let Some(album_artist) = &envelope.album_artist {
        params.push(("albumArtist", album_artist.clone()));
    }
    if let Some(duration_ms) = envelope.duration_ms {
        params.push(("duration", duration_ms.div_euclid(1_000).to_string()));
    }
    if let Some(mbid) = &envelope.musicbrainz_recording_id {
        params.push(("mbid", mbid.clone()));
    }
    params
}

impl ScrobbleTarget for LastFm {
    fn submit<'a>(
        &'a self,
        envelope: &'a ScrobbleEnvelope,
        secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async move {
            let mut params = scrobble_params(envelope, secret);
            params.push(("api_key", self.application.api_key.clone()));
            let signed = signature(&params, &self.application.secret);
            params.push(("api_sig", signed));
            params.push(("format", "json".to_owned()));

            let sent = self
                .client
                .post(self.endpoint.clone())
                .form(&params)
                .send()
                .await;
            let response = match sent {
                Ok(response) => response,
                Err(error) => return transport_verdict(&error),
            };
            let status = response.status();
            let retry_after = super::retry_after(&response);
            // **The status line is not enough here, and this reverses a
            // decision taken three commits ago.** That one said the body is
            // never read, on the strength of decision 12 — what this server
            // publishes is a state, never an echo. Decision 12 governs what
            // reaches a *member*; it says nothing about what an adapter may
            // read to form a verdict, and reading the numbers below publishes
            // nothing.
            //
            // What the earlier reasoning missed is the cost. Last.fm answers
            // `200` carrying `"error": 9` for a session key that has stopped
            // working, so a link whose credential died would record every
            // subsequent listen as `sent` while nothing was recorded anywhere
            // — a silent gap, which is the exact failure this RFC spends itself
            // making visible, and `AuthBroken` is the state it exists to reach.
            // The same answer carries `accepted`/`ignored` counts, and an
            // ignored listen is one the far end read and refused.
            //
            // The narrowness is the safeguard: a numeric code and two counts,
            // nothing textual. A body that will not parse leaves the status
            // line deciding, so a malformed answer cannot turn a success into
            // the one verdict a person has to answer.
            //
            // **And the body only ever refines a success.** A failing status
            // line already says everything the body could, and letting the
            // body answer for it costs two real things: a `429` carrying
            // `"error": 29` would return this adapter's own zero wait instead
            // of the `Retry-After` the header gave — defeating the whole point
            // of reading that header — and a `5xx` whose body happened to
            // parse as accepted would be read as a listen that arrived.
            //
            // **Read under a ceiling** (RFC-023 decision 1): an answer over it,
            // or one that breaks off, refines nothing, and the status line
            // decides as it does for a body that will not parse.
            if !status.is_success() {
                return status_verdict(status, retry_after);
            }
            let answer = match crate::remote::outbound::read_bounded_body(
                response,
                crate::remote::outbound::MAX_ANSWER_BYTES,
            )
            .await
            {
                Ok(body) => serde_json::from_slice::<ScrobbleAnswer>(&body).ok(),
                Err(error) => {
                    tracing::warn!(%error, "last.fm's answer was not read; its status line decides");
                    None
                }
            };
            match answer.as_ref().and_then(ScrobbleAnswer::verdict) {
                Some(verdict) => verdict,
                None => status_verdict(status, retry_after),
            }
        })
    }
}

/// Just enough of a `track.scrobble` answer to tell a refusal from a success.
///
/// **Numbers only.** `ignoredMessage` and `message` quote what was submitted
/// back, and decision 12 keeps a destination's own words out of this server
/// entirely — a field that could hold them is a field something would
/// eventually log.
#[derive(serde::Deserialize)]
struct ScrobbleAnswer {
    error: Option<u16>,
    scrobbles: Option<Scrobbles>,
}

#[derive(serde::Deserialize)]
struct Scrobbles {
    #[serde(rename = "@attr")]
    attr: Option<ScrobbleCounts>,
}

/// Last.fm has spelled these as numbers and as strings across versions of its
/// own answer, so both are read and anything else counts as absent.
#[derive(serde::Deserialize)]
struct ScrobbleCounts {
    accepted: Option<serde_json::Value>,
    ignored: Option<serde_json::Value>,
}

fn count(value: Option<&serde_json::Value>) -> Option<u64> {
    match value? {
        serde_json::Value::Number(number) => number.as_u64(),
        serde_json::Value::String(text) => text.parse().ok(),
        _ => None,
    }
}

impl ScrobbleAnswer {
    /// The verdict this answer earns, or `None` to leave it to the status line.
    fn verdict(&self) -> Option<ScrobbleVerdict> {
        if let Some(code) = self.error {
            return Some(error_verdict(code));
        }
        let attr = self.scrobbles.as_ref()?.attr.as_ref()?;
        let accepted = count(attr.accepted.as_ref())?;
        let ignored = count(attr.ignored.as_ref())?;
        if accepted > 0 {
            return Some(ScrobbleVerdict::Accepted);
        }
        if ignored > 0 {
            // Read, understood, refused. No retry fixes a submission the far
            // end will not have — a timestamp it considers too old, a title it
            // refuses to index.
            tracing::warn!("last.fm ignored the submission");
            return Some(ScrobbleVerdict::PermanentReject);
        }
        None
    }
}

/// What one of Last.fm's own error codes means for the queue.
///
/// The codes, and never the message beside them: that message quotes the
/// submission — and on the authorisation path, the request token.
fn error_verdict(code: u16) -> ScrobbleVerdict {
    match code {
        // 4 authentication failed, 9 invalid session key, 14 unauthorised
        // token, 15 expired token, 26 suspended API key. Every listen behind
        // this one would fail identically, which is what `AuthBroken` exists to
        // stop — and the honest answer even when the fault is the operator's
        // key rather than the member's.
        4 | 9 | 14 | 15 | 26 => {
            tracing::warn!(code, "last.fm refused the credentials");
            ScrobbleVerdict::AuthBroken
        }
        // 2 invalid service, 3 invalid method, 5 invalid format, 6 invalid
        // parameters, 7 invalid resource, 13 invalid signature. None of them is
        // fixed by sending the same thing again.
        2 | 3 | 5 | 6 | 7 | 13 => {
            tracing::warn!(code, "last.fm refused the submission");
            ScrobbleVerdict::PermanentReject
        }
        // 29 rate limit exceeded. The queue takes the larger of its own backoff
        // and this, so zero adds nothing to the wait while still saying that
        // room was asked for and no duration named.
        29 => {
            tracing::info!(code, "last.fm asked for room");
            ScrobbleVerdict::Retryable {
                after: Some(Duration::ZERO),
            }
        }
        // 8 operation failed, 11 service offline, 16 temporarily unavailable,
        // and whatever else they add. None says anything about the listen, so
        // it waits.
        _ => {
            tracing::warn!(code, "last.fm answered with an error");
            ScrobbleVerdict::Retryable { after: None }
        }
    }
}

/// The half of the journey that has to leave this machine.
///
/// `auth.getSession` turns the request token Last.fm handed the browser into a
/// session key that lasts until somebody revokes it.
///
/// **This is the web journey, and it does not call `auth.getToken`.** That
/// method belongs to the desktop-application journey, where the token is asked
/// for *before* the page opens; here Last.fm hands it over on the return. The
/// two resemble each other enough to be mixed up, and an implementation that
/// asked for a token and then received a different one would work with the
/// wrong one.
impl crate::services::LastFmSessionExchange for LastFm {
    fn exchange<'a>(
        &'a self,
        token: &'a str,
    ) -> BoxFuture<'a, Result<String, crate::services::ServiceError>> {
        Box::pin(async move {
            use crate::services::ServiceError;
            let mut params = vec![
                ("method", "auth.getSession".to_owned()),
                ("api_key", self.application.api_key.clone()),
                ("token", token.to_owned()),
            ];
            params.push(("api_sig", signature(&params, &self.application.secret)));
            params.push(("format", "json".to_owned()));
            // Read here, and only here. Every other answer from a destination is
            // judged by its status line — but a session key is the point of this
            // call, and there is nowhere else to get it. A refusing status is
            // `Invalid`, as it always was: the token was spent or never approved.
            let body: SessionResponse = self.call(&params, "auth.getSession").await?;
            let key = body.session.map(|session| session.key).unwrap_or_default();
            if key.is_empty() {
                // Last.fm answers `200` with an error object for a spent or
                // expired token. Nothing is logged of it beyond that it
                // happened: the object quotes the token back.
                tracing::warn!("last.fm returned no session key");
                return Err(ServiceError::Invalid);
            }
            Ok(key)
        })
    }

    /// `auth.getToken`, and the address the person has to open with it.
    ///
    /// **This is the method the comment above says belongs to the other
    /// journey, and it is now implemented** — for the CLI, where nothing comes
    /// back to this server. The two must not be mixed up: here the token is
    /// asked for *first* and is worth nothing until its owner approves it,
    /// while on the web Last.fm hands one over on the return, already
    /// approved. Both end at `auth.getSession` above, which is why one trait
    /// carries them.
    ///
    /// The address is built here rather than in the service because the
    /// application key is here — the service's copy of it lives on a structure
    /// that only exists when a public `https` address is configured, which is
    /// the case this journey exists to do without.
    fn request_token<'a>(
        &'a self,
    ) -> BoxFuture<'a, Result<crate::services::LastFmApproval, crate::services::ServiceError>> {
        Box::pin(async move {
            use crate::services::ServiceError;
            let mut params = vec![
                ("method", "auth.getToken".to_owned()),
                ("api_key", self.application.api_key.clone()),
            ];
            params.push(("api_sig", signature(&params, &self.application.secret)));
            params.push(("format", "json".to_owned()));
            // Every failure is `Unavailable` here, a refusing status included:
            // nothing the person brought has been asked yet, so whatever went
            // wrong is on the way to Last.fm or on this server's side of it.
            let body: TokenResponse = self
                .call(&params, "auth.getToken")
                .await
                .map_err(|_| ServiceError::Unavailable)?;
            let token = body.token.unwrap_or_default();
            if token.is_empty() {
                // A `200` carrying an error object — a bad key, or a signature
                // this server computed wrong. Nothing of the body is logged.
                tracing::warn!("last.fm returned no request token");
                return Err(ServiceError::Unavailable);
            }
            let mut authorize_url = url::Url::parse(crate::services::LASTFM_AUTHORIZE_URL)
                .map_err(|_| ServiceError::Unavailable)?;
            authorize_url
                .query_pairs_mut()
                .append_pair("api_key", &self.application.api_key)
                .append_pair("token", &token);
            Ok(crate::services::LastFmApproval {
                authorize_url: authorize_url.to_string(),
                token,
            })
        })
    }
}

/// Just enough of the answer to read the request token out of it.
///
/// Narrow for the reason [`SessionResponse`] is narrow: the error object
/// beside it quotes the application key back, and a type able to hold it is a
/// type something eventually prints.
#[derive(serde::Deserialize)]
struct TokenResponse {
    token: Option<String>,
}

/// Just enough of the answer to read the session key out of it.
///
/// Deliberately not the whole shape: the error object beside it quotes the
/// request token back, and a type that could hold it is a type something would
/// eventually print.
#[derive(serde::Deserialize)]
struct SessionResponse {
    session: Option<Session>,
}

#[derive(serde::Deserialize)]
struct Session {
    key: String,
}

/// What a failure to get an answer at all means.
///
/// The same rule as its two neighbours — decision 5 — written out here rather
/// than shared with them: three adapters answer for three destinations, and a
/// common helper would make the next one's different failure shape look like a
/// bug in this one.
///
/// **Two failures happen before anything leaves this machine**, and they are the
/// two that may be answered freely: a refused connection, and a request this
/// client could not even assemble. Everything else happened *after* the request
/// was on the wire, and decision 5 calls that indistinguishable from a success
/// whose acknowledgement was lost.
fn transport_verdict(error: &reqwest::Error) -> ScrobbleVerdict {
    if error.is_builder() {
        tracing::warn!("the last.fm request could not be assembled");
        ScrobbleVerdict::Retryable { after: None }
    } else if error.is_connect() {
        tracing::warn!("last.fm refused the connection");
        ScrobbleVerdict::Retryable { after: None }
    } else {
        tracing::warn!("last.fm did not answer a request that had already left");
        ScrobbleVerdict::Ambiguous
    }
}

/// The verdict a status line earns — and the status line alone.
fn status_verdict(status: reqwest::StatusCode, retry_after: Option<Duration>) -> ScrobbleVerdict {
    if status.is_success() {
        return ScrobbleVerdict::Accepted;
    }
    match status.as_u16() {
        // The session key is no good, or the application's own is. Every listen
        // behind this one would fail the same way, which is what `AuthBroken`
        // exists to stop — and it is the honest answer even when the fault is
        // the operator's key rather than the member's: the link cannot submit
        // and the account is told, rather than collecting choices nobody can
        // undo.
        401 | 403 => {
            tracing::warn!(%status, "last.fm refused the credentials");
            ScrobbleVerdict::AuthBroken
        }
        400 | 413 | 422 => {
            tracing::warn!(%status, "last.fm refused the submission");
            ScrobbleVerdict::PermanentReject
        }
        429 => {
            tracing::info!(?retry_after, "last.fm asked for room");
            ScrobbleVerdict::Retryable {
                after: Some(retry_after.unwrap_or(Duration::ZERO)),
            }
        }
        _ => {
            tracing::warn!(%status, "last.fm answered unusably");
            ScrobbleVerdict::Retryable { after: None }
        }
    }
}

#[cfg(test)]
mod tests;
