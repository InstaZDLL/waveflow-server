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
}

impl LastFm {
    pub fn new(
        client: reqwest::Client,
        base: &url::Url,
        application: LastFmApplication,
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
            let answer = response.json::<ScrobbleAnswer>().await.ok();
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
            let response = self
                .client
                .post(self.endpoint.clone())
                .form(&params)
                .send()
                .await
                .map_err(|_| ServiceError::Unavailable)?;
            if !response.status().is_success() {
                // The status, and never the body. **This response is the one
                // exception decision 10's "log what came back" rule must carve
                // out**: it carries the session key itself on success, and on
                // failure it carries the request token back in its error text.
                tracing::warn!(status = %response.status(), "last.fm refused the session exchange");
                return Err(ServiceError::Invalid);
            }
            // Read here, and only here. Every other answer from a destination is
            // judged by its status line — but a session key is the point of this
            // call, and there is nowhere else to get it.
            let body: SessionResponse = response
                .json()
                .await
                .map_err(|_| ServiceError::Unavailable)?;
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
mod tests {
    use super::*;

    fn an_envelope() -> ScrobbleEnvelope {
        ScrobbleEnvelope {
            played_at: 1_700_000_123_456,
            title: "Zenith".to_owned(),
            artists: vec!["Nova Kern".to_owned(), "Juno Vale".to_owned()],
            album: Some("Convergence".to_owned()),
            album_artist: Some("Nova Kern".to_owned()),
            duration_ms: Some(245_678),
            musicbrainz_recording_id: Some("d2b9c1f0-0000-4000-8000-000000000001".to_owned()),
        }
    }

    /// The signature is Last.fm's own recipe, checked against a value computed
    /// by hand from it.
    ///
    /// Sorted by name, name and value concatenated with nothing between them,
    /// the shared secret appended, MD5 of the UTF-8 bytes. Every one of those is
    /// a place to be wrong in a way no test that only signed and compared
    /// against itself would ever notice — which is why the expectation here is a
    /// literal rather than a second call to `signature`.
    #[test]
    fn the_signature_follows_the_recipe_last_fm_publishes() {
        use md5::Digest as _;
        let params = [
            ("method", "auth.getSession".to_owned()),
            ("api_key", "the-key".to_owned()),
            ("token", "the-token".to_owned()),
        ];
        // "api_keythe-keymethodauth.getSessiontokenthe-token" + secret.
        let expected = hex::encode(md5::Md5::digest(
            b"api_keythe-keymethodauth.getSessiontokenthe-tokenthe-secret",
        ));
        assert_eq!(signature(&params, "the-secret"), expected);
    }

    /// `format` and `callback` are outside the signature.
    ///
    /// Last.fm's rule, and it is not cosmetic: including either produces a
    /// signature they refuse, and the failure would arrive as an authentication
    /// error that looks exactly like a bad session key.
    #[test]
    fn the_shape_of_the_answer_is_not_part_of_the_signature() {
        let bare = [("method", "track.scrobble".to_owned())];
        let dressed = [
            ("method", "track.scrobble".to_owned()),
            ("format", "json".to_owned()),
            ("callback", "https://example.test/back".to_owned()),
        ];
        assert_eq!(
            signature(&bare, "the-secret"),
            signature(&dressed, "the-secret")
        );
    }

    /// The order the parameters were built in does not reach the signature.
    #[test]
    fn the_signature_sorts_before_it_concatenates() {
        let one_way = [
            ("track", "Zenith".to_owned()),
            ("artist", "Nova Kern".to_owned()),
        ];
        let the_other = [
            ("artist", "Nova Kern".to_owned()),
            ("track", "Zenith".to_owned()),
        ];
        assert_eq!(
            signature(&one_way, "the-secret"),
            signature(&the_other, "the-secret")
        );
    }

    /// The instants are seconds, and every credit is carried.
    #[test]
    fn the_submission_carries_seconds_and_every_credit() {
        let params = scrobble_params(&an_envelope(), "a-session-key");
        let value = |name: &str| {
            params
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(value("timestamp"), Some("1700000123"));
        assert_eq!(value("duration"), Some("245"));
        assert_eq!(value("artist"), Some("Nova Kern, Juno Vale"));
        assert_eq!(value("track"), Some("Zenith"));
        assert_eq!(value("album"), Some("Convergence"));
        assert_eq!(value("albumArtist"), Some("Nova Kern"));
        assert_eq!(value("sk"), Some("a-session-key"));
    }

    /// What this server does not know is not sent.
    #[test]
    fn what_is_unknown_is_left_out_rather_than_sent_empty() {
        let bare = ScrobbleEnvelope {
            album: None,
            album_artist: None,
            duration_ms: None,
            musicbrainz_recording_id: None,
            ..an_envelope()
        };
        let params = scrobble_params(&bare, "a-session-key");
        for absent in ["album", "albumArtist", "duration", "mbid"] {
            assert!(
                !params.iter().any(|(name, _)| *name == absent),
                "{absent} must not be sent"
            );
        }
    }

    /// The endpoint keeps the trailing slash Last.fm documents.
    ///
    /// `/2.0` answers a redirect, and this client follows none: without the
    /// slash every call would fail, and the adapter would read that failure as
    /// the destination having moved.
    #[test]
    fn the_endpoint_keeps_the_slash_last_fm_answers_on() {
        let base = super::super::validate_destination("https://ws.audioscrobbler.com", false)
            .expect("a valid destination");
        let client = super::super::outbound_client(Duration::from_secs(5)).expect("a client");
        let application = crate::config::LastFmApplication {
            api_key: "a-key".to_owned(),
            secret: "a-secret".to_owned(),
        };
        let target = LastFm::new(client, &base, application).expect("an adapter");
        assert_eq!(
            target.endpoint.as_str(),
            "https://ws.audioscrobbler.com/2.0/"
        );
    }

    /// A status line, and nothing else, decides.
    #[test]
    fn the_status_line_earns_the_verdict() {
        assert!(matches!(
            status_verdict(reqwest::StatusCode::OK, None),
            ScrobbleVerdict::Accepted
        ));
        assert!(matches!(
            status_verdict(reqwest::StatusCode::FORBIDDEN, None),
            ScrobbleVerdict::AuthBroken
        ));
        assert!(matches!(
            status_verdict(reqwest::StatusCode::BAD_REQUEST, None),
            ScrobbleVerdict::PermanentReject
        ));
        assert!(matches!(
            status_verdict(reqwest::StatusCode::TOO_MANY_REQUESTS, None),
            ScrobbleVerdict::Retryable {
                after: Some(Duration::ZERO)
            }
        ));
    }
}
