//! ListenBrainz, the first destination this server learns to speak to.
//!
//! First by RFC-010 decision 11, and for a reason worth repeating here: it asks
//! the architecture for nothing. A token per person, JSON, a permanent
//! submission and a `playing_now` its own documentation calls temporary — which
//! is decision 3 exactly. It self-hosts too, so it exercises decision 10's
//! bounded outbound surface from the first day rather than the third.
//!
//! Nothing in this file knows the queue exists. It is handed a listen and a
//! secret and answers one of five words; what happens to the row afterwards is
//! the drain's business and none of its own.

use std::time::Duration;

use futures_util::future::BoxFuture;
use serde::Serialize;

use crate::services::{ScrobbleEnvelope, ScrobbleTarget, ScrobbleVerdict};

use super::{endpoint, OutboundError, MAX_RESPONSE_BYTES};

/// How the destination names us in its own logs.
const CLIENT_NAME: &str = "WaveFlow Server";

/// The header ListenBrainz answers a `429` with: seconds until the window
/// reopens. Its own documentation recommends this one over `X-RateLimit-Reset`
/// because it carries no absolute time and so survives a clock that disagrees.
const RESET_IN_HEADER: &str = "x-ratelimit-reset-in";

pub struct ListenBrainz {
    client: reqwest::Client,
    submit: url::Url,
}

impl ListenBrainz {
    /// The submission endpoint is derived once, at construction, so a
    /// misconfigured destination is a boot-time failure rather than a verdict
    /// on somebody's listen.
    pub fn new(client: reqwest::Client, base: &url::Url) -> Result<Self, OutboundError> {
        Ok(Self {
            client,
            submit: endpoint(base, "1/submit-listens")?,
        })
    }
}

/// The submission body, in ListenBrainz's own vocabulary.
///
/// `listen_type` is `single` rather than `import`: decision 11 sends one listen
/// per request, so there is never a second element to justify the batch form.
#[derive(Debug, Serialize)]
struct Submission<'a> {
    listen_type: &'static str,
    payload: [Listen<'a>; 1],
}

#[derive(Debug, Serialize)]
struct Listen<'a> {
    /// **Seconds**, not milliseconds. The envelope carries epoch milliseconds
    /// like every other timestamp in this server, and handing those over
    /// unconverted would date every listen about fifty thousand years from now
    /// — a mistake a listening history keeps for good.
    listened_at: i64,
    track_metadata: TrackMetadata<'a>,
}

#[derive(Debug, Serialize)]
struct TrackMetadata<'a> {
    artist_name: String,
    track_name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_name: Option<&'a str>,
    additional_info: AdditionalInfo<'a>,
}

#[derive(Debug, Serialize)]
struct AdditionalInfo<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recording_mbid: Option<&'a str>,
    submission_client: &'static str,
    submission_client_version: &'static str,
}

/// The envelope, in the shape the destination reads.
///
/// Two conversions are the whole of it, and both are places to be wrong
/// quietly rather than loudly — which is why each has a test naming it.
fn submission(envelope: &ScrobbleEnvelope) -> Submission<'_> {
    Submission {
        listen_type: "single",
        payload: [Listen {
            listened_at: played_at_seconds(envelope.played_at),
            track_metadata: TrackMetadata {
                artist_name: credited_artists(&envelope.artists),
                track_name: &envelope.title,
                release_name: envelope.album.as_deref(),
                additional_info: AdditionalInfo {
                    duration_ms: envelope.duration_ms,
                    recording_mbid: envelope.musicbrainz_recording_id.as_deref(),
                    submission_client: CLIENT_NAME,
                    submission_client_version: env!("CARGO_PKG_VERSION"),
                },
            },
        }],
    }
}

/// Epoch milliseconds to epoch seconds.
///
/// `div_euclid` rather than `/`: integer division truncates towards zero, so a
/// timestamp before 1970 would round the wrong way. No listen has one, and that
/// is exactly the sort of confidence this rounds correctly instead of relying
/// on.
fn played_at_seconds(played_at_ms: i64) -> i64 {
    played_at_ms.div_euclid(1_000)
}

/// Every credited artist, in tag order, as the one string the destination takes.
///
/// ListenBrainz has a single `artist_name` field and matches it against
/// MusicBrainz afterwards. Sending only the first credit would match more often
/// and would quietly drop a collaborator from somebody's history — the envelope
/// records what was heard, and a guest who was on the track was heard. Joining
/// keeps the listen true and leaves the matching to the end that owns it.
fn credited_artists(artists: &[String]) -> String {
    artists.join(", ")
}

impl ScrobbleTarget for ListenBrainz {
    fn submit<'a>(
        &'a self,
        envelope: &'a ScrobbleEnvelope,
        secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict> {
        Box::pin(async move {
            let sent = self
                .client
                .post(self.submit.clone())
                // The scheme ListenBrainz documents, and not `Bearer`.
                .header(reqwest::header::AUTHORIZATION, format!("Token {secret}"))
                .json(&submission(envelope))
                .send()
                .await;
            let response = match sent {
                Ok(response) => response,
                Err(error) => return transport_verdict(&error),
            };
            let status = response.status();
            let retry_after = reset_in(&response);
            // Read for the log alone, bounded, and never returned to anyone:
            // decision 10 says what the API shows is a state and not an echo of
            // somebody else's words.
            let detail = bounded_body(response).await;
            status_verdict(status, retry_after, &detail)
        })
    }
}

/// What a failure to get an answer at all means.
///
/// A connect failure is the one case where nothing left this machine, so it is
/// the one case that may be retried freely. Everything else — a timeout, a
/// broken body, a response that would not decode — happened *after* the request
/// was on the wire, and decision 5 calls that indistinguishable from a success
/// whose acknowledgement was lost.
fn transport_verdict(error: &reqwest::Error) -> ScrobbleVerdict {
    if error.is_connect() {
        tracing::warn!("listenbrainz refused the connection");
        ScrobbleVerdict::Retryable { after: None }
    } else {
        tracing::warn!("listenbrainz did not answer a request that had already left");
        ScrobbleVerdict::Ambiguous
    }
}

/// The verdict a status line earns.
fn status_verdict(
    status: reqwest::StatusCode,
    retry_after: Option<Duration>,
    detail: &str,
) -> ScrobbleVerdict {
    if status.is_success() {
        return ScrobbleVerdict::Accepted;
    }
    match status.as_u16() {
        // The token is no good. Every listen behind this one would fail the
        // same way, which is what `AuthBroken` exists to stop.
        401 | 403 => {
            tracing::warn!(%status, "listenbrainz refused the token");
            ScrobbleVerdict::AuthBroken
        }
        // Read, understood, refused. No retry fixes a payload the far end will
        // not have.
        400 | 413 | 422 => {
            tracing::warn!(%status, detail, "listenbrainz refused the submission");
            ScrobbleVerdict::PermanentReject
        }
        429 => {
            tracing::info!(?retry_after, "listenbrainz asked for room");
            ScrobbleVerdict::Retryable { after: retry_after }
        }
        // Everything else, 5xx and the surprises alike — including a redirect,
        // which this client does not follow and which means the operator's
        // destination has moved. None of them say anything about the listen, so
        // it waits; the bounded attempts give an operator about a day to notice
        // through the link's own `degraded`, and the queue gives up after that
        // rather than pretending forever.
        _ => {
            tracing::warn!(%status, detail, "listenbrainz answered unusably");
            ScrobbleVerdict::Retryable { after: None }
        }
    }
}

/// How long the destination asked us to wait, in whole seconds.
fn reset_in(response: &reqwest::Response) -> Option<Duration> {
    response
        .headers()
        .get(RESET_IN_HEADER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_secs)
}

/// At most [`MAX_RESPONSE_BYTES`] of whatever came back, for the log.
///
/// Streamed rather than `text()`: nothing bounds what a host at the other end
/// chooses to send, and an adapter that reads it all is one broken destination
/// away from exhausting this process.
async fn bounded_body(mut response: reqwest::Response) -> String {
    let mut collected: Vec<u8> = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        let room = MAX_RESPONSE_BYTES.saturating_sub(collected.len());
        if room == 0 {
            break;
        }
        collected.extend_from_slice(&chunk[..chunk.len().min(room)]);
    }
    String::from_utf8_lossy(&collected).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope() -> ScrobbleEnvelope {
        ScrobbleEnvelope {
            // 2026-09-13T11:32:42Z, and the millisecond remainder is there on
            // purpose: it must not survive the conversion.
            played_at: 1_789_648_362_750,
            title: "Never Gonna Give You Up".into(),
            artists: vec!["Rick Astley".into(), "The Guest".into()],
            album: Some("Whenever You Need Somebody".into()),
            album_artist: Some("Rick Astley".into()),
            duration_ms: Some(222_000),
            musicbrainz_recording_id: Some("98255a8c-017a-4bc7-8dd6-1fa36124572b".into()),
        }
    }

    /// The one conversion a listening history would keep forever if it were
    /// wrong.
    #[test]
    fn a_listen_is_dated_in_seconds_and_not_milliseconds() {
        assert_eq!(played_at_seconds(1_789_648_362_750), 1_789_648_362);
        // Truncation towards zero would answer 0 here and -1 is correct; no
        // listen predates 1970, which is why this is worth pinning rather than
        // assuming.
        assert_eq!(played_at_seconds(-1), -1);

        let body = serde_json::to_value(submission(&envelope())).unwrap();
        assert_eq!(body["payload"][0]["listened_at"], 1_789_648_362);
    }

    #[test]
    fn the_body_is_the_shape_listenbrainz_documents() {
        let body = serde_json::to_value(submission(&envelope())).unwrap();
        assert_eq!(body["listen_type"], "single");
        assert_eq!(body["payload"].as_array().unwrap().len(), 1);

        let metadata = &body["payload"][0]["track_metadata"];
        assert_eq!(metadata["track_name"], "Never Gonna Give You Up");
        assert_eq!(metadata["release_name"], "Whenever You Need Somebody");
        // Every credit, in tag order: the envelope records what was heard, and
        // a guest on the track was heard.
        assert_eq!(metadata["artist_name"], "Rick Astley, The Guest");
        assert_eq!(metadata["additional_info"]["duration_ms"], 222_000);
        assert_eq!(
            metadata["additional_info"]["recording_mbid"],
            "98255a8c-017a-4bc7-8dd6-1fa36124572b"
        );
    }

    /// An absent field is absent, rather than present and null: the destination
    /// reads `additional_info` as a set of hints, and a null hint is a claim.
    #[test]
    fn what_the_listen_does_not_know_is_left_out_entirely() {
        let bare = ScrobbleEnvelope {
            album: None,
            duration_ms: None,
            musicbrainz_recording_id: None,
            ..envelope()
        };
        let body = serde_json::to_value(submission(&bare)).unwrap();
        let metadata = &body["payload"][0]["track_metadata"];
        assert!(metadata.get("release_name").is_none());
        assert!(metadata["additional_info"].get("duration_ms").is_none());
        assert!(metadata["additional_info"].get("recording_mbid").is_none());
        // And what it always knows stays.
        assert_eq!(
            metadata["additional_info"]["submission_client"],
            CLIENT_NAME
        );
    }

    #[test]
    fn each_answer_earns_the_verdict_the_queue_can_act_on() {
        use reqwest::StatusCode;
        let no_wait = None;

        assert_eq!(
            status_verdict(StatusCode::OK, no_wait, ""),
            ScrobbleVerdict::Accepted
        );
        // A bad token breaks the link rather than the listen.
        assert_eq!(
            status_verdict(StatusCode::UNAUTHORIZED, no_wait, ""),
            ScrobbleVerdict::AuthBroken
        );
        // A refused payload is terminal: no retry can fix it.
        assert_eq!(
            status_verdict(StatusCode::BAD_REQUEST, no_wait, ""),
            ScrobbleVerdict::PermanentReject
        );
        // And a rate limit carries the destination's own answer back.
        assert_eq!(
            status_verdict(
                StatusCode::TOO_MANY_REQUESTS,
                Some(Duration::from_secs(12)),
                ""
            ),
            ScrobbleVerdict::Retryable {
                after: Some(Duration::from_secs(12))
            }
        );
        // A server fault says nothing about the listen.
        assert_eq!(
            status_verdict(StatusCode::BAD_GATEWAY, no_wait, ""),
            ScrobbleVerdict::Retryable { after: None }
        );
        // A redirect is not followed, so it reaches here: the operator's
        // destination moved, and the listen waits for them to notice.
        assert_eq!(
            status_verdict(StatusCode::FOUND, no_wait, ""),
            ScrobbleVerdict::Retryable { after: None }
        );
    }
}
