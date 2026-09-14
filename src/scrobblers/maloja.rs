//! Maloja, the destination the envelope pours into without losing anything.
//!
//! Second by RFC-010 decision 11, and the reason is the shape of its API rather
//! than its popularity: `newscrobble` takes **lists** of artists and album
//! artists, so every credit this server recorded arrives as a credit. The
//! ListenBrainz adapter has to join them into one string and let the far end
//! re-match — honest, and lossy. Here nothing is lost.
//!
//! It is also the destination decision 10's revision exists for. Maloja
//! self-hosts by nature; on a family server everybody has their own instance,
//! which is what made one URL field per recipient the wrong shape.
//!
//! Like its neighbour, nothing in this file knows the queue exists.

use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::Serialize;

use crate::services::{ScrobbleEnvelope, ScrobbleTarget, ScrobbleVerdict};

use super::{endpoint, OutboundError};

pub struct Maloja {
    client: reqwest::Client,
    submit: url::Url,
}

impl Maloja {
    /// The submission endpoint is derived once, at construction, so a
    /// misconfigured destination is a boot-time failure rather than a verdict
    /// on somebody's listen.
    ///
    /// `mlj_1` is Maloja's own versioned prefix, and it is spelled out here
    /// rather than discovered: this server does not probe a destination to find
    /// out what it speaks, because a probe is one more outbound request whose
    /// answer decides where a credential goes.
    pub fn new(client: reqwest::Client, base: &url::Url) -> Result<Self, OutboundError> {
        Ok(Self {
            client,
            submit: endpoint(base, "apis/mlj_1/newscrobble")?,
        })
    }
}

/// The submission body, in Maloja's own vocabulary.
///
/// **The key travels in the body, not in a header.** Maloja has accepted it
/// there since its first API version, and `Authorization: Bearer` only since
/// 3.2 — a self-hosted destination is whatever version its operator installed,
/// and picking the newer spelling would refuse older instances for no gain.
///
/// That is why this type spells its own `Debug` out below rather than deriving
/// it: the body *is* a credential, and the derived one would print it the first
/// time anybody adds a `tracing::debug!` to this file.
#[derive(Serialize)]
struct Submission<'a> {
    key: &'a str,
    title: &'a str,
    /// Every credit, as credits. This is the field that makes Maloja lossless
    /// for this server's envelope.
    artists: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    album: Option<&'a str>,
    /// A list because Maloja's is, though this server records one album artist.
    /// Sending `[]` and sending nothing are different things to a destination
    /// that matches on it, so an absent one is absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    albumartists: Option<[&'a str; 1]>,
    /// **Seconds**, like `time`. Maloja's `length` is how long the track is;
    /// its `duration` is how long it was listened to, which this server does
    /// not record — decision 3 queues a listen, not a measurement of one — so
    /// `duration` is not sent at all rather than guessed from `length`.
    #[serde(skip_serializing_if = "Option::is_none")]
    length: Option<i64>,
    /// **Seconds**, not milliseconds. The envelope carries epoch milliseconds
    /// like every other timestamp in this server, and handing those over
    /// unconverted would date every listen about fifty thousand years from now
    /// — a mistake a listening history keeps for good.
    time: i64,
}

/// Written by hand so the guarantee is structural rather than circumstantial.
///
/// The same reasoning `LinkScrobbleRequest` carries, and more pressing here:
/// this struct is the one place in the outbound half where a member's secret
/// sits inside a value that a formatter would otherwise print whole.
impl std::fmt::Debug for Submission<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Submission")
            .field("key", &"[redacted]")
            .field("title", &self.title)
            .field("time", &self.time)
            .finish_non_exhaustive()
    }
}

/// The envelope, in the shape the destination reads.
///
/// The MusicBrainz recording id has nowhere to go: Maloja matches on names and
/// takes no identifier. Said here rather than left as a field somebody looks
/// for later and assumes was forgotten.
fn submission<'a>(envelope: &'a ScrobbleEnvelope, secret: &'a str) -> Submission<'a> {
    Submission {
        key: secret,
        title: &envelope.title,
        artists: &envelope.artists,
        album: envelope.album.as_deref(),
        albumartists: envelope.album_artist.as_deref().map(|artist| [artist]),
        length: envelope
            .duration_ms
            .map(|duration| duration.div_euclid(1_000)),
        time: envelope.played_at.div_euclid(1_000),
    }
}

impl ScrobbleTarget for Maloja {
    fn submit<'a>(
        &'a self,
        envelope: &'a ScrobbleEnvelope,
        secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async move {
            let sent = self
                .client
                .post(self.submit.clone())
                .json(&submission(envelope, secret))
                .send()
                .await;
            let response = match sent {
                Ok(response) => response,
                Err(error) => return transport_verdict(&error),
            };
            let status = response.status();
            let retry_after = super::retry_after(&response);
            // **The body is never read**, exactly as for ListenBrainz:
            // decision 12 says what this server reports is a state and not an
            // echo of the destination's own words, and a body that is never
            // read is the tightest bound there is on one.
            //
            // It costs something real here and it is still the right trade.
            // Maloja answers `200` with `{"status": "failure", …}` for some
            // refusals a status line does not distinguish, so those are read as
            // accepted. The alternative is parsing a third party's error
            // vocabulary into this server's five words and keeping the mapping
            // true across versions of a destination nobody here controls — and
            // getting *that* wrong turns a refusal into `Ambiguous`, which
            // decision 13 makes a person answer, one listen at a time.
            drop(response);
            status_verdict(status, retry_after)
        })
    }
}

/// What a failure to get an answer at all means.
///
/// Identical to the ListenBrainz reading, and deliberately not shared with it:
/// the two adapters answer for two destinations, and a common helper would make
/// the next one's different failure shape look like a bug in this one. What is
/// shared is the *rule* — decision 5 — written out in both places.
///
/// **Two failures happen before anything leaves this machine**, and they are the
/// two that may be answered freely: a refused connection, and a request this
/// client could not even assemble. Everything else — a timeout, a broken body, a
/// response that would not decode — happened *after* the request was on the
/// wire, and decision 5 calls that indistinguishable from a success whose
/// acknowledgement was lost.
fn transport_verdict(error: &reqwest::Error) -> ScrobbleVerdict {
    if error.is_builder() {
        tracing::warn!("the maloja request could not be assembled");
        ScrobbleVerdict::Retryable { after: None }
    } else if error.is_connect() {
        tracing::warn!("maloja refused the connection");
        ScrobbleVerdict::Retryable { after: None }
    } else {
        tracing::warn!("maloja did not answer a request that had already left");
        ScrobbleVerdict::Ambiguous
    }
}

/// The verdict a status line earns — and the status line alone.
fn status_verdict(status: reqwest::StatusCode, retry_after: Option<Duration>) -> ScrobbleVerdict {
    if status.is_success() {
        return ScrobbleVerdict::Accepted;
    }
    match status.as_u16() {
        // The key is no good. Every listen behind this one would fail the same
        // way, which is what `AuthBroken` exists to stop.
        401 | 403 => {
            tracing::warn!(%status, "maloja refused the key");
            ScrobbleVerdict::AuthBroken
        }
        // Read, understood, refused. No retry fixes a payload the far end will
        // not have.
        400 | 413 | 422 => {
            tracing::warn!(%status, "maloja refused the submission");
            ScrobbleVerdict::PermanentReject
        }
        429 => {
            tracing::info!(?retry_after, "maloja asked for room");
            // `Some` even when no header was readable: the **status** says room
            // was asked for, the header only says how much. Zero rather than an
            // invented number — the queue takes the larger of its own backoff
            // and this.
            //
            // Maloja documents no rate limit of its own, and this arm is here
            // for what sits in front of it: a reverse proxy answering for a
            // self-hosted instance is the ordinary deployment, and it is the
            // thing that emits `429` and `Retry-After`.
            ScrobbleVerdict::Retryable {
                after: Some(retry_after.unwrap_or(Duration::ZERO)),
            }
        }
        // Everything else, 5xx and the surprises alike — including a redirect,
        // which this client does not follow and which means the operator's
        // destination has moved. None of them say anything about the listen, so
        // it waits.
        _ => {
            tracing::warn!(%status, "maloja answered unusably");
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

    /// Every credit arrives as a credit.
    ///
    /// This is the whole reason decision 11 puts Maloja second. A body that
    /// joined the artists would work, and would quietly turn a duo into a band
    /// nobody has heard of in somebody's statistics.
    #[test]
    fn the_envelope_pours_in_without_losing_a_credit() {
        let envelope = an_envelope();
        let body = serde_json::to_value(submission(&envelope, "a-key")).unwrap();
        assert_eq!(
            body["artists"],
            serde_json::json!(["Nova Kern", "Juno Vale"])
        );
        assert_eq!(body["albumartists"], serde_json::json!(["Nova Kern"]));
        assert_eq!(body["album"], "Convergence");
        assert_eq!(body["title"], "Zenith");
    }

    /// Both instants are seconds, and both are truncated the same way.
    ///
    /// Milliseconds handed over unconverted would date every listen about fifty
    /// thousand years from now, and a listening history keeps that for good.
    #[test]
    fn the_instants_are_seconds_and_not_milliseconds() {
        let envelope = an_envelope();
        let body = serde_json::to_value(submission(&envelope, "a-key")).unwrap();
        assert_eq!(body["time"], 1_700_000_123);
        assert_eq!(body["length"], 245);

        // `div_euclid`, so an instant before 1970 rounds down rather than
        // towards zero. No listen has one; the point is not to depend on that.
        let ancient = ScrobbleEnvelope {
            played_at: -1_500,
            ..an_envelope()
        };
        let body = serde_json::to_value(submission(&ancient, "a-key")).unwrap();
        assert_eq!(body["time"], -2);
    }

    /// What this server does not know is not sent.
    ///
    /// An absent album and an absent duration are absent, not empty: a
    /// destination matching on `album` treats `""` as a title, and Maloja's
    /// `duration` is a listened length this server never measured.
    #[test]
    fn what_is_unknown_is_left_out_rather_than_sent_empty() {
        let bare = ScrobbleEnvelope {
            album: None,
            album_artist: None,
            duration_ms: None,
            ..an_envelope()
        };
        let body = serde_json::to_value(submission(&bare, "a-key")).unwrap();
        let object = body.as_object().unwrap();
        for absent in ["album", "albumartists", "length", "duration"] {
            assert!(!object.contains_key(absent), "{absent} must not be sent");
        }
        // And what is always known is always sent.
        for present in ["key", "title", "artists", "time"] {
            assert!(object.contains_key(present), "{present} must be sent");
        }
    }

    /// The body carries a secret, so it does not survive being formatted.
    ///
    /// The failure message deliberately carries neither the key nor the
    /// formatted output: if this guard were removed, that output would *be* the
    /// credential, and a panic message is a log line.
    #[test]
    fn a_submission_does_not_print_its_key() {
        let envelope = an_envelope();
        let shown = format!("{:?}", submission(&envelope, "a-key-nobody-should-read"));
        assert!(
            !shown.contains("a-key-nobody-should-read"),
            "the debug output carried the key"
        );
        assert!(shown.contains("[redacted]"), "the key was not replaced");
    }

    /// A status line, and nothing else, decides.
    #[test]
    fn the_status_line_earns_the_verdict() {
        assert!(matches!(
            status_verdict(reqwest::StatusCode::OK, None),
            ScrobbleVerdict::Accepted
        ));
        for refused in [401, 403] {
            assert!(
                matches!(
                    status_verdict(reqwest::StatusCode::from_u16(refused).unwrap(), None),
                    ScrobbleVerdict::AuthBroken
                ),
                "{refused}"
            );
        }
        for refused in [400, 413, 422] {
            assert!(
                matches!(
                    status_verdict(reqwest::StatusCode::from_u16(refused).unwrap(), None),
                    ScrobbleVerdict::PermanentReject
                ),
                "{refused}"
            );
        }
        // A rate limit with no readable header still rests the link: the status
        // says room was asked for, the header only says how much.
        assert!(matches!(
            status_verdict(reqwest::StatusCode::TOO_MANY_REQUESTS, None),
            ScrobbleVerdict::Retryable {
                after: Some(Duration::ZERO)
            }
        ));
        // A redirect is not followed, and means the destination moved.
        assert!(matches!(
            status_verdict(reqwest::StatusCode::FOUND, None),
            ScrobbleVerdict::Retryable { after: None }
        ));
    }
}
