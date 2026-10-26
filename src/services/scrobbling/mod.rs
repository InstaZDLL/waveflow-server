//! Sending a listen somewhere else. RFC-010.
//!
//! The whole of this module is arranged around one sentence from decision 5:
//! **the transactional outbox gives atomicity between WaveFlow and its own
//! queue, and it cannot give atomicity between WaveFlow and a third party.**
//! What follows is exactly one side of that frontier — the queue, its states,
//! and the drain that walks it. Nothing here knows what ListenBrainz, Maloja or
//! Last.fm are; an adapter answers with one of five words and the drain acts on
//! the word.

use super::*;

use std::time::Duration;

use futures_util::future::BoxFuture;

mod enqueue;
mod links;
mod outbox;
mod purge;
mod retry;
mod uncertain;

/// A destination the server can be taught to speak to.
///
/// The vocabulary is CHECK-constrained in the schema, so this enum and that
/// constraint are one fact written twice on purpose: the database refuses a
/// value this cannot name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, ToSchema)]
pub enum ScrobbleProvider {
    /// Spelled out per variant rather than derived from the name.
    /// `rename_all = "snake_case"` turns this one into `listen_brainz` and
    /// `LastFm` into `last_fm` — names the `CHECK` constraint refuses and
    /// `FromStr` cannot read, so the API would publish a destination no client
    /// could send back in. One rule per variant, and no interaction between
    /// two. `tests::the_four_spellings_of_a_destination_agree` holds them
    /// together.
    #[serde(rename = "listenbrainz")]
    ListenBrainz,
    #[serde(rename = "maloja")]
    Maloja,
    #[serde(rename = "lastfm")]
    LastFm,
}

impl ScrobbleProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ListenBrainz => "listenbrainz",
            Self::Maloja => "maloja",
            Self::LastFm => "lastfm",
        }
    }
}

impl FromStr for ScrobbleProvider {
    type Err = ServiceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Ok(match value {
            "listenbrainz" => Self::ListenBrainz,
            "maloja" => Self::Maloja,
            "lastfm" => Self::LastFm,
            _ => return Err(ServiceError::Invalid),
        })
    }
}

/// One listen, frozen as it was heard.
///
/// Every field is a copy taken at the moment of the listen. Decision 2: the
/// track is never re-read at drain time, because a correction to its tags would
/// otherwise rewrite history that has already happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrobbleEnvelope {
    pub played_at: i64,
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub duration_ms: Option<i64>,
    pub musicbrainz_recording_id: Option<String>,
}

/// What one attempt at one destination came to.
///
/// **Not an HTTP status.** Decision 6: Last.fm answers `200` carrying an
/// application error in the body, and Maloja reports refusals in its JSON, so a
/// drain that read the status line would take a failure for a success. The
/// adapter reads whatever its destination actually says and answers one of
/// these five words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrobbleVerdict {
    /// The destination has it. Nothing further.
    Accepted,
    /// The destination did not get it, and saying so again later may work.
    ///
    /// `after` is what the destination itself asked for when it said so —
    /// ListenBrainz answers `429` with `X-RateLimit-Reset-In`, in seconds.
    /// Decision 6 promised to honour that and PR #191 had nowhere to put it, so
    /// the verdict was opaque and the header was read by nobody.
    ///
    /// **It is honoured as a floor, never as a replacement.** Waiting *at
    /// least* as long as asked is what honouring means; taking the destination's
    /// twelve seconds in place of our own sixteen-minute backoff would answer a
    /// request to slow down by speeding up.
    Retryable { after: Option<Duration> },
    /// The authorisation is no longer good. The link is marked broken and its
    /// queue finishes, because every further attempt would fail identically.
    AuthBroken,
    /// The destination read the submission and refuses it. No retry can fix a
    /// payload the other end will not have.
    PermanentReject,
    /// **We do not know.** The request may have been recorded before the
    /// connection broke. Decision 5 forbids retrying automatically here: a
    /// duplicate in a public listening history is worse than a gap.
    Ambiguous,
}

/// One destination, as the drain sees it.
///
/// Deliberately dyn-safe and deliberately ignorant of the queue: an adapter is
/// handed a listen and a secret, and answers a verdict. It is constructed with
/// whatever it needs to reach its destination — a base URL among other things —
/// so the queue never carries one. Decision 10: a destination is the operator's
/// setting, never an account's.
pub trait ScrobbleTarget: Send + Sync + 'static {
    fn submit<'a>(
        &'a self,
        envelope: &'a ScrobbleEnvelope,
        secret: &'a str,
    ) -> BoxFuture<'a, ScrobbleVerdict>;
}

/// The registry `initialize` fills and the drain reads.
///
/// Keyed by the **pair**, never by the recipient alone. Decision 10 lets an
/// operator declare several instances of one destination, and a registry keyed
/// on `ScrobbleProvider` would hold one adapter for all of them — pointing at
/// whichever URL happened to be registered last, and sending one member's
/// listens to another member's instance.
pub(super) type ScrobbleTargets =
    Arc<dashmap::DashMap<(ScrobbleProvider, String), Arc<dyn ScrobbleTarget>>>;

// Decision 12: what the API shows is a state, not an echo of the envelope nor
// of the destination's own words. In a `//` because `ToSchema` publishes the
// `///` verbatim as this schema's description, and "decision 12" names nothing
// a client can look up.

/// One instance a member may link to, named without its address.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScrobbleDestinationName {
    pub provider: ScrobbleProvider,
    /// The name to put in the path of `PUT /api/v2/scrobble-links/{provider}/{destination}`.
    pub destination: String,
    /// Whether this server can actually link it right now.
    pub available: bool,
    /// And when it cannot, why.
    ///
    /// Published here rather than left to be discovered: a link that fails
    /// later with no explanation is the silent failure this whole RFC spends
    /// itself preventing. Last.fm is the one recipient that can be declared and
    /// still unusable, because its journey needs an application the operator
    /// registered and an `https` address to bring a person back to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<ScrobbleUnavailable>,
}

/// Why a declared instance cannot be linked — a code, not a sentence.
///
/// This was an English sentence until 2026-09-15, printed verbatim into a web
/// client that ships in two languages: a French reader was told, in English,
/// what to change in their configuration. A reason only helps whoever can read
/// it, so what crosses the wire is the case and the wording belongs to whoever
/// is doing the telling. The two cases are the two things an operator can
/// actually change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScrobbleUnavailable {
    /// No Last.fm application is declared on this server —
    /// `WAVEFLOW_SCROBBLE_LASTFM_API_KEY` and `_SECRET`.
    NoApplicationConfigured,
    /// `WAVEFLOW_PUBLIC_URL` is not an `https` address, so there is nowhere to
    /// bring a browser back to. The command-line journey needs no public
    /// address at all and still works.
    BrowserJourneyNeedsHttps,
}

/// What one link's queue looks like from outside: counters, never content.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScrobbleLinkState {
    pub provider: ScrobbleProvider,
    /// Which instance of it. An account may hold one link per instance, so the
    /// recipient alone no longer names a row here.
    pub destination: String,
    /// `healthy`, `degraded` or `broken`.
    ///
    /// **`healthy` cannot mean "the token is still good".** A valid link with
    /// three thousand listens waiting since this morning is broken in every
    /// sense the person cares about, and that silent failure is the whole
    /// reason a durable queue is visible at all.
    pub health: &'static str,
    /// Waiting, and never attempted.
    pub pending: i64,
    /// Waiting after at least one failure.
    pub retrying: i64,
    /// Ambiguous, and still asking the person to decide. An entry that has been
    /// retried stops being counted here, without being erased.
    pub uncertain: i64,
    pub oldest_pending_at: Option<i64>,
    pub last_success_at: Option<i64>,
    /// A normalised cause — `rate_limited`, `auth_broken` — or nothing.
    pub last_failure: Option<String>,
}

// Decision 12 keeps the envelope out of the API — what this publishes is a
// state, never an echo of what was heard — while decision 13 asks a person to
// choose the fate of one specific listen. A bare UUID is not something anybody
// can choose about, which is what `played_at` is here for.
//
// In a `//` and not a `///` because `ToSchema` publishes the doc block verbatim
// as this schema's description, and "decision 12" has no referent outside
// `docs/rfcs/`.

/// One listen whose fate is unknown, named so that it can be answered.
///
/// Carries no title and no artists — only the entry's own state, and
/// `played_at`, which lets a client match it against a listen it already holds
/// from its history.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UncertainScrobble {
    // A UUID rather than the rowid: a sequential id would tell anyone holding a
    // single entry of their own how many listens this whole server has ever
    // queued. Kept out of the `///` for the same reason as the block above.
    /// The entry's public name, and the only one this API will accept back.
    pub id: Uuid,
    pub provider: ScrobbleProvider,
    /// Which instance of that recipient the listen was queued for.
    ///
    /// The recipient alone does not name it. Decision 10 lets an operator
    /// declare several instances of one destination, and a household server
    /// where everybody self-hosts a Maloja is the ordinary case — so an entry
    /// that said only `maloja` would ask somebody to choose the fate of a
    /// listen without telling them which of their two instances may already
    /// hold it. Decision 13 refuses to make that choice for them; this is what
    /// makes it a choice they can actually make.
    pub destination: String,
    /// When the listen happened, not when it was queued.
    pub played_at: i64,
    pub attempts: i64,
    /// A normalised cause. An entry reaches this listing two ways and each
    /// writes its own: `ambiguous`, when the adapter could not tell whether
    /// the submission arrived, and `interrupted`, when a previous process left
    /// the row claimed and this one recovered it.
    ///
    /// It named `stalled` and `auth_broken` until 2026-09-14, and wrote
    /// neither — `auth_broken` cancels an entry rather than leaving it
    /// asking, and nothing has ever written `stalled` at all. The same
    /// category as the `rate_limited` note further down this file, which was
    /// answered by making the code produce the value; here the value was the
    /// wrong one to promise.
    pub last_failure: Option<String>,
    pub updated_at: i64,
}

/// What one drain pass did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScrobbleDrain {
    pub accepted: usize,
    pub retrying: usize,
    pub abandoned: usize,
    pub rejected: usize,
    pub uncertain: usize,
    pub broken: usize,
    /// Rows a previous process left claimed and never settled, finished as
    /// `uncertain` at the start of this pass. Counted apart from `uncertain`
    /// because they are not something this pass decided — they are what it
    /// found.
    pub recovered: usize,
    /// Rows moved aside because no adapter is registered for their destination.
    /// Not an attempt: a server missing an adapter is a misconfiguration, and
    /// spending the listen's retries on it would destroy the queue the operator
    /// is about to fix.
    ///
    /// This said "left exactly as they were" until a review noticed the path had
    /// called `defer_scrobble` since the day it was written. The row keeps its
    /// `attempts`; it does not keep its place.
    pub unserviced: usize,
    /// Rows moved out of the way because their destination had just asked for
    /// room, or gone silent. Not an attempt either — nothing was sent.
    ///
    /// Counted because without it a pass that defers forty-nine rows compares
    /// equal to `default()` and logs nothing at all, which is a hole in a module
    /// whose whole argument is that a queue which stops must say so.
    pub rested: usize,
}

/// One row the drain is about to act on.
struct DueEntry {
    id: i64,
    link_id: Uuid,
    provider: ScrobbleProvider,
    /// Which instance of it, so the drain picks the adapter this listen was
    /// queued for rather than whichever one shares its recipient.
    destination: String,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
    attempts: i64,
    envelope: ScrobbleEnvelope,
}

/// What one `Retryable` verdict came to.
///
/// Three words rather than a boolean, because there are three outcomes and the
/// third is easy to miss: the row may no longer have been this pass's to write.
/// Folding that into either of the other two would report an attempt that was
/// never recorded — see [`DomainServices::settle_scrobble`].
enum Rescheduled {
    /// Pushed out to a later attempt.
    Later,
    /// Out of attempts, and settled as abandoned.
    Abandoned,
    /// Already settled elsewhere. Nothing to count.
    NotOurs,
}

#[cfg(test)]
mod tests;
