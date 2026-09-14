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
    /// two. [`tests::the_four_spellings_of_a_destination_agree`] holds them
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
pub(super) type ScrobbleTargets = Arc<dashmap::DashMap<ScrobbleProvider, Arc<dyn ScrobbleTarget>>>;

// Decision 12: what the API shows is a state, not an echo of the envelope nor
// of the destination's own words. In a `//` because `ToSchema` publishes the
// `///` verbatim as this schema's description, and "decision 12" names nothing
// a client can look up.

/// What one link's queue looks like from outside: counters, never content.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ScrobbleLinkState {
    pub provider: ScrobbleProvider,
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
    /// When the listen happened, not when it was queued.
    pub played_at: i64,
    pub attempts: i64,
    /// A normalised cause — `stalled`, `auth_broken` — or nothing.
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

/// The floor of the growing wait between two attempts.
const RETRY_BASE: Duration = Duration::from_secs(60);
/// Its ceiling in milliseconds.
///
/// One number, spelled twice from itself. Converting the `Duration` at each use
/// needed a fallback that could never fire, in a file that argues a few hundred
/// lines below that an unreachable branch is a case a later reader will mistake
/// for one that happens.
const RETRY_CEILING_MS: i64 = 60 * 60 * 1_000;
/// Its ceiling. Past this, waiting longer buys nothing a restart would not.
const RETRY_CEILING: Duration = Duration::from_millis(RETRY_CEILING_MS as u64);
/// How far past the outbound deadline a claimed row must sit before it is read
/// as abandoned rather than in flight.
///
/// A live submission cannot outlast `request_timeout`, so any margin at all
/// would do; a generous one means a paused process — a laptop closing, a
/// container throttled — is never mistaken for a dead one and does not have a
/// listen declared uncertain out from under it.
const STALE_SENDING_MARGIN: Duration = Duration::from_secs(60);

impl DomainServices {
    /// Queues one listen for every destination this account has linked.
    ///
    /// Called from inside the transaction that writes `play_event`, under the
    /// same writer gate — decision 1. A listen therefore cannot be recorded
    /// without being queued, nor queued without being recorded, and the
    /// idempotence `claim_operation` already provides covers both at once: a
    /// replayed scrobble rolls the transaction back and so writes neither.
    ///
    /// Silent when the account has linked nothing, which is every account until
    /// somebody says otherwise.
    pub(super) async fn enqueue_scrobble_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        play_event_id: i64,
        track_id: Uuid,
        played_at: i64,
    ) -> Result<(), ServiceError> {
        let links = sqlx::query_scalar::<_, String>(
            "SELECT id FROM scrobble_link WHERE user_id=? AND status='active' ORDER BY provider",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        if links.is_empty() {
            return Ok(());
        }
        let Some(envelope) = self
            .scrobble_envelope_on(&mut *connection, track_id, played_at)
            .await?
        else {
            return Ok(());
        };
        let artists_json =
            serde_json::to_string(&envelope.artists).map_err(|_| ServiceError::Invalid)?;
        let now = now_ms();
        for link in links {
            sqlx::query(
                "INSERT INTO scrobble_outbox (public_id, link_id, play_event_id, played_at, title, \
                 artists_json, album, album_artist, duration_ms, musicbrainz_recording_id, \
                 state, next_attempt_at, created_at, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?)",
            )
            // One per row rather than one per listen: the two destinations of a
            // single listen are two entries, and a person acts on them one at a
            // time.
            .bind(Uuid::new_v4().to_string())
            .bind(&link)
            .bind(play_event_id)
            .bind(envelope.played_at)
            .bind(&envelope.title)
            .bind(&artists_json)
            .bind(envelope.album.as_deref())
            .bind(envelope.album_artist.as_deref())
            .bind(envelope.duration_ms)
            .bind(envelope.musicbrainz_recording_id.as_deref())
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&mut *connection)
            .await?;
        }
        Ok(())
    }

    /// The listen as it was heard, or `None` when it is not worth sending.
    ///
    /// Decision 7: a track the server cannot name is never submitted. Without a
    /// title or without a single credited artist the submission is unusable at
    /// the far end and skews the statistics it lands in, so it is dropped here
    /// rather than queued to be refused later.
    async fn scrobble_envelope_on(
        &self,
        connection: &mut SqliteConnection,
        track_id: Uuid,
        played_at: i64,
    ) -> Result<Option<ScrobbleEnvelope>, ServiceError> {
        // The corrected values, through the same `COALESCE` every projection
        // reads: what was heard is what the catalogue was showing at the time.
        let Some(row) = sqlx::query(
            "SELECT COALESCE(ovr.title, t.title) AS title, t.album_title, \
                    alb.album_artist_name, t.duration_ms, \
                    COALESCE(ovr.musicbrainz_recording_id, t.musicbrainz_recording_id) \
                      AS musicbrainz_recording_id \
             FROM track t \
             LEFT JOIN album alb ON alb.id=t.album_id \
             LEFT JOIN track_override ovr ON ovr.track_id=t.id \
             WHERE t.id=?",
        )
        .bind(track_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        else {
            return Ok(None);
        };
        let title: String = row.try_get("title")?;
        if title.trim().is_empty() {
            return Ok(None);
        }
        let artists = sqlx::query_scalar::<_, String>(
            "SELECT ar.name FROM track_participant tp JOIN artist ar ON ar.id=tp.artist_id \
             WHERE tp.track_id=? AND tp.role='artist' ORDER BY tp.position",
        )
        .bind(track_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        if artists.is_empty() {
            return Ok(None);
        }
        Ok(Some(ScrobbleEnvelope {
            played_at,
            title,
            artists,
            album: row.try_get("album_title")?,
            album_artist: row.try_get("album_artist_name")?,
            duration_ms: row.try_get("duration_ms")?,
            musicbrainz_recording_id: row.try_get("musicbrainz_recording_id")?,
        }))
    }

    /// Teaches the drain how to reach one destination.
    ///
    /// Registered after `initialize` rather than built into the services, which
    /// is what lets a test drive the whole queue against a double and lets an
    /// operator run a server with no outbound adapter at all.
    pub fn register_scrobble_target(
        &self,
        provider: ScrobbleProvider,
        target: Arc<dyn ScrobbleTarget>,
    ) {
        self.scrobble_targets.insert(provider, target);
    }

    /// Authorises this account to submit to one destination, in a new
    /// generation.
    ///
    /// Any live generation for the same pair is unlinked first, and its queue
    /// finishes with it — decision 4. The two happen under one transaction, so
    /// there is no instant at which a waiting listen belongs to no
    /// authorisation.
    pub async fn link_scrobble(
        &self,
        user_id: Uuid,
        provider: ScrobbleProvider,
        secret: &str,
    ) -> Result<Uuid, ServiceError> {
        let secret = secret.trim();
        if secret.is_empty() || secret.len() > 512 {
            return Err(ServiceError::Invalid);
        }
        // A secret carrying a control character can never be spelled as an HTTP
        // header, so it will never work against any destination — this is a
        // property of the credential rather than of whichever adapter carries
        // it, which is why the check belongs here and not there.
        //
        // Refused at the moment it is pasted, because that is the only moment
        // the person can fix it. Discovered instead on a background drain hours
        // later, it would arrive as a broken link nobody could explain, which is
        // the silent failure RFC-010 spends itself preventing. The adapter still
        // guards its own construction: credentials sealed before this check
        // existed are never revalidated.
        if secret.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
            return Err(ServiceError::Invalid);
        }
        let sealed = self.secret_box.encrypt(secret.as_bytes())?;
        let id = Uuid::new_v4();
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        self.unlink_scrobble_on(&mut tx, user_id, provider, now)
            .await?;
        sqlx::query(
            "INSERT INTO scrobble_link (id, user_id, provider, status, credential_nonce, \
             credential_ciphertext, created_at, updated_at) VALUES (?, ?, ?, 'active', ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(provider.as_str())
        .bind(sealed.nonce.as_slice())
        .bind(sealed.ciphertext.as_slice())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Withdraws the authorisation, and finishes what was waiting under it.
    ///
    /// Answers whether there was anything live to withdraw.
    pub async fn unlink_scrobble(
        &self,
        user_id: Uuid,
        provider: ScrobbleProvider,
    ) -> Result<bool, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let unlinked = self
            .unlink_scrobble_on(&mut tx, user_id, provider, now)
            .await?;
        tx.commit().await?;
        Ok(unlinked)
    }

    /// The half of unlinking that both callers share.
    ///
    /// The waiting rows are cancelled rather than deleted: they record listens
    /// that really happened and were really never sent, and a queue that erases
    /// its own losses cannot be asked what it lost.
    ///
    /// **A row already in flight is left alone, deliberately.** It is not
    /// waiting — the request has left — and decision 4 finishes what is
    /// *waiting*. Calling it `cancelled` would be a lie about something that
    /// really was submitted, so it settles as whatever it truly turns out to be
    /// and stays there as history. It cannot be acted on afterwards either:
    /// `retry_uncertain_scrobble` requires a live link, and `scrobble_links`
    /// does not report an unlinked one. This is the same asymmetry the
    /// `AuthBroken` arm had to close, and the opposite answer is the right one
    /// here.
    async fn unlink_scrobble_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        provider: ScrobbleProvider,
        now: i64,
    ) -> Result<bool, ServiceError> {
        let live = sqlx::query_scalar::<_, String>(
            "SELECT id FROM scrobble_link WHERE user_id=? AND provider=? AND status <> 'unlinked'",
        )
        .bind(user_id.to_string())
        .bind(provider.as_str())
        .fetch_optional(&mut *connection)
        .await?;
        let Some(link_id) = live else {
            return Ok(false);
        };
        sqlx::query(
            "UPDATE scrobble_outbox SET state='cancelled', last_failure='unlinked', updated_at=? \
             WHERE link_id=? AND state='pending'",
        )
        .bind(now)
        .bind(&link_id)
        .execute(&mut *connection)
        .await?;
        sqlx::query("UPDATE scrobble_link SET status='unlinked', updated_at=? WHERE id=?")
            .bind(now)
            .bind(&link_id)
            .execute(&mut *connection)
            .await?;
        Ok(true)
    }

    /// Every live link this account holds, and the shape of its queue.
    pub async fn scrobble_links(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<ScrobbleLinkState>, ServiceError> {
        let rows = sqlx::query(
            "SELECT id, provider, status, last_success_at, last_failure FROM scrobble_link \
             WHERE user_id=? AND status <> 'unlinked' ORDER BY provider",
        )
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let mut states = Vec::with_capacity(rows.len());
        let now = now_ms();
        for row in rows {
            let link_id: String = row.try_get("id")?;
            let provider = ScrobbleProvider::from_str(row.try_get("provider")?)?;
            // `SUM(CASE …)` rather than `COUNT(*) FILTER`: the aggregate filter
            // needs a SQLite newer than the floor this crate builds against,
            // and one query answering four counts is the point either way.
            //
            // The uncertain count excludes an entry a person has already
            // retried. It is still true and still readable; it has simply
            // stopped asking for a decision, and a counter that kept naming it
            // would ask for the same one forever.
            //
            // Read on `retried_at` rather than on a row naming this one through
            // `retry_of`: the retry ends `sent`, so retention will eventually
            // take it, and a count deduced from its survival would put this
            // link back to `degraded` for a listen already answered.
            let counts = sqlx::query(
                "SELECT \
                   SUM(CASE WHEN state='pending' AND attempts=0 THEN 1 ELSE 0 END) AS pending, \
                   SUM(CASE WHEN state='pending' AND attempts>0 THEN 1 ELSE 0 END) AS retrying, \
                   SUM(CASE WHEN state='uncertain' AND retried_at IS NULL \
                     THEN 1 ELSE 0 END) AS uncertain, \
                   MIN(CASE WHEN state='pending' THEN created_at END) AS oldest_pending_at \
                 FROM scrobble_outbox WHERE link_id=?",
            )
            .bind(&link_id)
            .fetch_one(self.db.pool())
            .await?;
            // `SUM` over no rows is NULL, which is nought here: a link nobody
            // has played under yet has an empty queue, not an unknown one.
            let pending: i64 = counts.try_get::<Option<i64>, _>("pending")?.unwrap_or(0);
            let retrying: i64 = counts.try_get::<Option<i64>, _>("retrying")?.unwrap_or(0);
            let uncertain: i64 = counts.try_get::<Option<i64>, _>("uncertain")?.unwrap_or(0);
            let oldest_pending_at: Option<i64> = counts.try_get("oldest_pending_at")?;
            let broken: String = row.try_get("status")?;
            states.push(ScrobbleLinkState {
                provider,
                health: link_health(
                    &broken,
                    uncertain,
                    oldest_pending_at,
                    now,
                    self.scrobbling.stale_after,
                ),
                pending,
                retrying,
                uncertain,
                oldest_pending_at,
                last_success_at: row.try_get("last_success_at")?,
                last_failure: row.try_get("last_failure")?,
            });
        }
        Ok(states)
    }

    /// Every ambiguous entry still asking this account for a decision.
    ///
    /// **Without this, the two gestures decision 13 grants cannot be aimed.**
    /// `discard_uncertain_scrobble` and `retry_uncertain_scrobble` both name an
    /// entry by its `public_id`, and until now nothing published one: a person
    /// was asked to choose, and given no way to learn what about.
    ///
    /// **The exclusions are the counter's, deliberately.** An entry already
    /// retried has stopped asking, so [`Self::scrobble_links`] stops counting
    /// it; a list that went on naming it would ask for the same decision
    /// forever, and the count beside it would disagree. An unlinked generation
    /// drops out for the same reason from the other side: there is no
    /// authorisation left to answer under at all. Its rows stay in the table
    /// and stay true; they have simply stopped being a question.
    ///
    /// **A `broken` link is still listed, and that is not an oversight.** Its
    /// rows ask something answerable — [`Self::discard_uncertain_scrobble`]
    /// works on them, and preferring the gap is a decision.
    /// [`Self::retry_uncertain_scrobble`] requires `status='active'` and will
    /// refuse them for as long as the token stays bad, exactly as
    /// [`Self::due_scrobbles`] refuses to drain under one. The two gestures have
    /// different preconditions here, on purpose.
    ///
    /// An earlier version of this paragraph said the list excluded whatever
    /// retry refuses. That was true of `unlinked` and false of `broken`, which
    /// is the shape of claim this file keeps having to correct.
    /// `an_entry_under_a_broken_link_can_be_discarded_but_not_retried` holds the
    /// asymmetry still.
    ///
    /// Newest first: an ambiguous listen from this afternoon is the one a person
    /// can still remember playing.
    pub async fn uncertain_scrobbles(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<UncertainScrobble>, ServiceError> {
        // Tenancy in the query rather than in a caller, as everywhere else here:
        // the join to `scrobble_link` is what makes another account's entry
        // unnameable rather than merely unreturned.
        let rows = sqlx::query(
            "SELECT o.public_id, l.provider, o.played_at, o.attempts, \
                    o.last_failure, o.updated_at \
             FROM scrobble_outbox o JOIN scrobble_link l ON l.id = o.link_id \
             WHERE l.user_id=? AND l.status <> 'unlinked' AND o.state='uncertain' \
               AND o.retried_at IS NULL \
             ORDER BY o.played_at DESC, o.id DESC",
        )
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(UncertainScrobble {
                    // Stored as text and parsed back rather than trusted: a
                    // value this cannot read is a row this server wrote wrong,
                    // and answering with it would publish nonsense as an id.
                    id: Uuid::parse_str(row.try_get("public_id")?)
                        .map_err(|_| ServiceError::Invalid)?,
                    provider: ScrobbleProvider::from_str(row.try_get("provider")?)?,
                    played_at: row.try_get("played_at")?,
                    attempts: row.try_get("attempts")?,
                    last_failure: row.try_get("last_failure")?,
                    updated_at: row.try_get("updated_at")?,
                })
            })
            .collect()
    }

    /// Throws away one ambiguous entry. The person prefers the gap.
    ///
    /// Named by its `public_id`, never by its rowid: the sequential one would
    /// tell anyone holding a single entry of their own how many listens this
    /// whole server has queued.
    ///
    /// An entry already retried is refused here too, and on the same column the
    /// other three readers use: it has stopped asking, so there is nothing left
    /// to prefer a gap to.
    pub async fn discard_uncertain_scrobble(
        &self,
        user_id: Uuid,
        entry: Uuid,
    ) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let changed = sqlx::query(
            "UPDATE scrobble_outbox SET state='discarded', updated_at=? \
             WHERE public_id=? AND state='uncertain' AND retried_at IS NULL \
             AND link_id IN \
               (SELECT id FROM scrobble_link WHERE user_id=? AND status <> 'unlinked')",
        )
        .bind(now_ms())
        .bind(entry.to_string())
        .bind(user_id.to_string())
        .execute(self.db.pool())
        .await?;
        if changed.rows_affected() == 0 {
            return Err(ServiceError::NotFound);
        }
        Ok(())
    }

    /// Sends one ambiguous entry again, knowing it may already be there.
    ///
    /// **Retrying is not reactivating.** The ambiguous attempt stays in the
    /// record exactly as it happened and a *new* row is queued beside it,
    /// pointing back at it. Reopening the original would falsify the only trace
    /// that explains why the destination may hold this listen twice —
    /// decision 13.
    ///
    /// The original stops being counted as uncertain, because it no longer asks
    /// the person for anything; it has not stopped being true.
    ///
    /// **The joker is spent on the entry, not on its descendant.** Whether this
    /// gesture has already been made is `retried_at`, a fact the row carries,
    /// and not the survival of a row naming it through `retry_of`. A retry ends
    /// `sent`, so retention will take it, and every reader deducing the answer
    /// from a join would then hand this listen a second joker.
    pub async fn retry_uncertain_scrobble(
        &self,
        user_id: Uuid,
        entry: Uuid,
    ) -> Result<Uuid, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        // The conditional UPDATE comes first because it is the arbitration:
        // spending the joker and refusing a second one are the same write, so
        // two simultaneous calls cannot both find the entry unanswered.
        //
        // The link has to be live in the same breath: a retry under a withdrawn
        // authorisation would submit to whichever profile happens to be linked
        // now, which is the very substitution generations exist to prevent.
        //
        // `updated_at` is deliberately left alone. It is what
        // `uncertain_scrobbles` publishes as the moment this listen became
        // ambiguous, and answering it did not make it ambiguous again.
        let claimed = sqlx::query(
            "UPDATE scrobble_outbox SET retried_at=? \
             WHERE public_id=? AND state='uncertain' AND retried_at IS NULL \
               AND link_id IN \
                 (SELECT id FROM scrobble_link WHERE user_id=? AND status='active')",
        )
        .bind(now)
        .bind(entry.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        if claimed.rows_affected() == 0 {
            return Err(ServiceError::NotFound);
        }
        // Resolved after the claim rather than before it: `retry_of`, the
        // ordering and the jitter all speak in rowids, and `public_id` is
        // unique, so this names the row the UPDATE just took and no other.
        let rowid =
            sqlx::query_scalar::<_, i64>("SELECT id FROM scrobble_outbox WHERE public_id=?")
                .bind(entry.to_string())
                .fetch_one(&mut *tx)
                .await?;
        let public_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO scrobble_outbox (public_id, link_id, play_event_id, retry_of, played_at, \
             title, artists_json, album, album_artist, duration_ms, musicbrainz_recording_id, \
             state, next_attempt_at, created_at, updated_at) \
             SELECT ?, link_id, play_event_id, id, played_at, title, artists_json, album, \
                    album_artist, duration_ms, musicbrainz_recording_id, 'pending', ?, ?, ? \
             FROM scrobble_outbox WHERE id=?",
        )
        .bind(public_id.to_string())
        .bind(now)
        .bind(now)
        .bind(now)
        .bind(rowid)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(public_id)
    }

    /// Walks the queue on a timer. The shape the six other background tasks
    /// use: a pass at boot, then one per interval.
    pub fn spawn_scrobble_drain(&self) {
        let services = self.clone();
        // Never zero, because `tokio::time::interval` panics on a zero period —
        // here, inside a `tokio::spawn`ed task, where the unwind takes the queue
        // with it and says nothing. The same shape as the `clamp` panic three
        // commits ago, and the same answer.
        //
        // `parse_positive_env` refuses a zero interval from the environment, so
        // this is unreachable from a configured server. It is reachable from a
        // `Config` built in process, which is how every test builds one, and
        // this is the only one of the seven background tasks whose period comes
        // from an assignable field rather than a constant or a validated
        // `Option`.
        let interval = services
            .scrobbling
            .drain_interval
            .max(Duration::from_millis(1));
        tokio::spawn(async move {
            services.drain_scrobbles_now().await;
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.drain_scrobbles_now().await;
            }
        });
    }

    async fn drain_scrobbles_now(&self) {
        match self.drain_scrobble_outbox().await {
            Ok(drained) if drained == ScrobbleDrain::default() => {}
            Ok(drained) => tracing::info!(
                accepted = drained.accepted,
                retrying = drained.retrying,
                abandoned = drained.abandoned,
                rejected = drained.rejected,
                uncertain = drained.uncertain,
                broken = drained.broken,
                unserviced = drained.unserviced,
                rested = drained.rested,
                recovered = drained.recovered,
                "scrobble queue drained"
            ),
            Err(error) => tracing::warn!(%error, "could not drain the scrobble queue"),
        }
    }

    /// One pass. Public so a test can run it rather than wait out an interval.
    ///
    /// **No writer gate is held across a submission.** The batch is read, each
    /// entry is submitted with nothing locked, and only the verdict is written
    /// back under the gate. Holding the process-wide gate across a third
    /// party's latency would let a destination that has stopped answering block
    /// every other write on the server.
    ///
    /// One listen per request, deliberately. Decision 11: Last.fm would take
    /// fifty at a time, and a batch of fifty that comes back `Ambiguous` makes
    /// fifty listens uncertain at once — which decision 5 then forbids retrying.
    pub async fn drain_scrobble_outbox(&self) -> Result<ScrobbleDrain, ServiceError> {
        // Before anything else: whatever a previous process left mid-flight.
        // Those rows are claimed, so nothing below would look at them, and left
        // alone they would sit `sending` forever.
        let mut drained = ScrobbleDrain {
            recovered: usize::try_from(self.recover_stale_sending().await?).unwrap_or(0),
            ..Default::default()
        };
        // Links this pass has just found broken.
        //
        // The batch is chosen before the first verdict comes back, so without
        // this every entry queued behind a broken authorisation would still be
        // submitted once — each failing identically, which is exactly what
        // `AuthBroken` has already established. `mark_link_broken` has finished
        // their rows in the database; this is what stops the requests.
        let mut broken_links: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        // Links that have already gone quiet on this pass.
        //
        // The same argument as `broken_links`, applied to the path that did not
        // get it. Without this, a destination that accepts connections and never
        // answers costs the deadline *per entry*: fifty rows at thirty seconds
        // is a pass of twenty-five minutes, and — far worse — fifty `uncertain`
        // rows, each of which is a decision decision 13 lets a person make
        // exactly once. One outage would become a heap of irreversible manual
        // choices, which is the accident the RFC refuses when it declines to
        // offer a "retry everything" button, arriving from the other end.
        //
        // The entry that actually timed out stays `Ambiguous` — it really was
        // sent. The ones behind it are not touched at all: they were never
        // emitted, so they stay `pending` and come back next pass, when one
        // further timeout will cost one further entry and no more.
        // Two reasons reach this set and they end in the same gesture: stop
        // offering this link rows for the rest of the pass.
        //
        // A destination that went silent, and one that answered `429` with a
        // delay. The second was missing until a review found it: the batch is
        // chosen before the first verdict, so being told to slow down was
        // followed by forty-nine more submissions on the same pass, each
        // earning its own refusal and spending its own attempt. Answering
        // "please wait" by sending the rest of the batch is the opposite of
        // honouring it, and decision 6 is explicit that the delay is honoured.
        // Carrying how long each one asked for, because skipping is not enough.
        //
        // A skipped row keeps its old `next_attempt_at`, already in the past, so
        // it is due again the instant the next pass runs — and it sorts *ahead*
        // of another link's newer rows. One row of the resting link drains per
        // pass while the rest hold the head of the queue, and a link with a
        // large backlog starves every other one for hours or for good. The file
        // already makes this argument for the no-adapter case, twenty lines
        // below; the resting path needed the same answer.
        //
        // Deferring by the delay that was actually asked is not a slower retry
        // than skipping — a skipped row returns in one drain interval anyway —
        // it is a *more* honouring one: right now the other rows sit due and are
        // re-offered once a minute despite an hour having been requested.
        let mut resting_links: std::collections::HashMap<Uuid, i64> =
            std::collections::HashMap::new();
        // The floor keeps a `429` that named no duration from deferring by
        // nothing at all; the ceiling is the one the queue already refuses to
        // let a third party exceed.
        // The ceiling first, because the floor falls back to it.
        //
        // `unwrap_or(i64::MAX)` on the floor was the same park-a-row-forever
        // hole this queue already refuses a destination, arriving by the other
        // door — configuration. An interval this platform cannot represent in
        // milliseconds is not a reason to defer a listen past the end of time.
        let rest_ceiling = RETRY_CEILING_MS;
        // At least a millisecond, which closes the literal zero and no more
        // than that.
        //
        // An earlier version of this comment claimed it kept a rested row from
        // going "straight back at the head of the queue". That is not true and
        // a review said so: `due_scrobbles` selects `next_attempt_at <= now`,
        // so nought and one are both due on the very next pass. What this
        // actually prevents is a zero deferral being written at all.
        //
        // `parse_positive_env` refuses a zero interval from the environment; a
        // `Config` built in process can carry one, and the tests build theirs
        // that way. The sharper hazard of that value is not here but in
        // `spawn_scrobble_drain`, where `tokio::time::interval` panics on a
        // zero period — guarded there.
        let rest_floor = i64::try_from(self.scrobbling.drain_interval.as_millis())
            .unwrap_or(rest_ceiling)
            .max(1);
        for entry in self.due_scrobbles(now_ms()).await? {
            if broken_links.contains(&entry.link_id) {
                // Its waiting rows are already `cancelled`, so there is nothing
                // left to defer and no place for them to be in the way of.
                continue;
            }
            if let Some(rest) = resting_links.get(&entry.link_id).copied() {
                if self.defer_scrobble(&entry, rest).await? {
                    drained.rested += 1;
                }
                continue;
            }
            let Some(target) = self
                .scrobble_targets
                .get(&entry.provider)
                .map(|found| Arc::clone(found.value()))
            else {
                // Stepped aside rather than left at the head of the queue.
                //
                // No attempt is spent — a server with no adapter for this
                // destination is misconfigured, which is not a failed delivery
                // — but the row must not keep its place either. `due_scrobbles`
                // orders by `next_attempt_at` under a fixed batch, and a row
                // nothing can service never advances on its own, so without
                // this it would fill every pass forever and a destination that
                // *does* have an adapter would never be reached. That is
                // starvation rather than slowness, and it is the reason this
                // defers instead of merely counting.
                if self.defer_scrobble(&entry, rest_floor).await? {
                    drained.unserviced += 1;
                }
                continue;
            };
            let secret = match self.secret_box.decrypt(&entry.nonce, &entry.ciphertext) {
                Ok(secret) => secret,
                // A secret that will not open is a broken link, not a broken
                // listen: it means this database and this `instance.key` are
                // not the pair that sealed it, and no amount of retrying
                // decrypts it.
                Err(_) => {
                    self.mark_link_broken(entry.link_id, "credential_unreadable")
                        .await?;
                    broken_links.insert(entry.link_id);
                    drained.broken += 1;
                    continue;
                }
            };
            let secret = match String::from_utf8(secret) {
                Ok(secret) => secret,
                Err(_) => {
                    self.mark_link_broken(entry.link_id, "credential_unreadable")
                        .await?;
                    broken_links.insert(entry.link_id);
                    drained.broken += 1;
                    continue;
                }
            };
            // Taken out of `pending` before a single byte is emitted. A pass
            // that loses the race leaves the row to whoever won it rather than
            // sending a second copy — see `claim_scrobble` for why this also
            // matters to a process that never races anyone.
            if !self.claim_scrobble(&entry).await? {
                continue;
            }
            // Bounded, because nothing else bounds it. A destination that
            // accepts the connection and then never answers would otherwise
            // hold this background task for the life of the process — and the
            // stopped queue that results is the very thing decision 12's
            // `degraded` exists to report, arriving by the one route that also
            // stops `degraded` from ever being recomputed.
            //
            // An expired wait is `Ambiguous`, never `Retryable`. The request
            // left; what failed to come back is the answer, which is exactly
            // the case decision 5 calls indistinguishable — the destination may
            // already hold this listen, so the server does not get to decide to
            // send it again. An adapter that knows better, because its own
            // connection failed before anything was sent, answers `Retryable`
            // itself and never reaches this deadline: the finer judgement
            // belongs where the knowledge is, and this is only the backstop.
            let verdict = match tokio::time::timeout(
                self.scrobbling.request_timeout,
                target.submit(&entry.envelope, &secret),
            )
            .await
            {
                Ok(verdict) => verdict,
                // The entry and its destination, and nothing of the envelope
                // or of the secret.
                Err(_) => {
                    tracing::warn!(
                        entry = entry.id,
                        provider = entry.provider.as_str(),
                        "a scrobble submission passed its deadline and is now uncertain"
                    );
                    // Nothing else queued for this destination is offered on
                    // this pass; see `resting_links` above. No delay was named
                    // here — silence asks for nothing — so the floor stands.
                    resting_links.insert(entry.link_id, rest_floor);
                    ScrobbleVerdict::Ambiguous
                }
            };
            match verdict {
                // Every arm counts only what it actually wrote. A verdict whose
                // row had already been settled elsewhere is a verdict about a
                // row this pass no longer owns, and reporting it would describe
                // a write that did not happen — see `settle_scrobble`.
                ScrobbleVerdict::Accepted => {
                    if self.settle_scrobble(&entry, "sent", None).await? {
                        drained.accepted += 1;
                    }
                }
                ScrobbleVerdict::Retryable { after } => {
                    // Only when the destination actually asked. A refused
                    // connection or a 5xx is a fault, not a request for room,
                    // and resting the whole link on one of those would slow a
                    // recovery nobody asked to slow.
                    if let Some(after) = after {
                        // `min` then `max`, never `clamp`: `Ord::clamp` panics
                        // when its minimum exceeds its maximum, and nothing
                        // stops an operator setting `WAVEFLOW_SCROBBLE_DRAIN_INTERVAL_SECS`
                        // above the hour — `parse_positive_env` bounds it below
                        // zero and nowhere above. That panic would unwind inside
                        // a spawned task and stop the queue for the life of the
                        // process: the silent stop this RFC exists to prevent,
                        // reachable by one plausible setting.
                        //
                        // The order is also the right answer when the floor does
                        // exceed the ceiling: if a pass runs every two hours,
                        // deferring by one would have the row offered again
                        // before the next pass anyway, so the floor should win.
                        let asked = i64::try_from(after.as_millis())
                            .unwrap_or(i64::MAX)
                            .min(rest_ceiling)
                            .max(rest_floor);
                        resting_links.insert(entry.link_id, asked);
                    }
                    match self.reschedule_scrobble(&entry, after).await? {
                        Rescheduled::Later => drained.retrying += 1,
                        Rescheduled::Abandoned => drained.abandoned += 1,
                        Rescheduled::NotOurs => {}
                    }
                }
                ScrobbleVerdict::PermanentReject => {
                    if self
                        .settle_scrobble(&entry, "rejected", Some("rejected"))
                        .await?
                    {
                        drained.rejected += 1;
                    }
                }
                ScrobbleVerdict::Ambiguous => {
                    if self
                        .settle_scrobble(&entry, "uncertain", Some("ambiguous"))
                        .await?
                    {
                        drained.uncertain += 1;
                    }
                }
                ScrobbleVerdict::AuthBroken => {
                    // This row is claimed, so the sweep inside
                    // `mark_link_broken` — which finishes the link's *waiting*
                    // rows — does not reach it. Finished here, on the same
                    // terms as the ones queued behind it.
                    self.settle_scrobble(&entry, "cancelled", Some("auth_broken"))
                        .await?;
                    // Unconditional, unlike the counters above: a refused
                    // authorisation is a fact about the link, not about whether
                    // this particular row was still ours to write.
                    self.mark_link_broken(entry.link_id, "auth_broken").await?;
                    broken_links.insert(entry.link_id);
                    drained.broken += 1;
                }
            }
        }
        Ok(drained)
    }

    /// Takes one row out of `pending` before anything is emitted.
    ///
    /// Answers whether this pass got it. The `UPDATE … WHERE state='pending'`
    /// is the whole mechanism: two passes reaching for the same row both run
    /// it, SQLite serialises them, and exactly one sees a row affected. The
    /// loser leaves it alone instead of sending a second copy. `drain_scrobble_outbox`
    /// is public, so a manual or test pass really can run beside the background
    /// one.
    ///
    /// It also closes a window that has nothing to do with racing. Between a
    /// submission leaving and its verdict being committed the process can stop,
    /// and a row left `pending` there is simply sent again at the next boot —
    /// the duplicate this whole module exists to avoid, arriving without anyone
    /// choosing it. Left `sending`, it is recoverable as what it actually is:
    /// unknown.
    async fn claim_scrobble(&self, entry: &DueEntry) -> Result<bool, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let claimed = sqlx::query(
            "UPDATE scrobble_outbox SET state='sending', updated_at=? \
             WHERE id=? AND state='pending'",
        )
        .bind(now)
        .bind(entry.id)
        .execute(self.db.pool())
        .await?;
        Ok(claimed.rows_affected() == 1)
    }

    /// Finishes the rows a stopped process left mid-flight.
    ///
    /// A row is `sending` only between its claim and its verdict, and a live
    /// submission cannot outlast `request_timeout`. Anything still `sending`
    /// well past that belongs to a process that stopped between emitting and
    /// recording, so nobody knows whether the destination took it — which is
    /// `uncertain` by decision 5, and emphatically not `pending`. Returning it
    /// to the queue would be the server choosing a duplicate on someone's
    /// behalf, which is the one choice it never gets to make.
    async fn recover_stale_sending(&self) -> Result<u64, ServiceError> {
        let now = now_ms();
        let stale_after = self
            .scrobbling
            .request_timeout
            .saturating_add(STALE_SENDING_MARGIN);
        let cutoff = now.saturating_sub(i64::try_from(stale_after.as_millis()).unwrap_or(i64::MAX));
        let _writer = self.db.writer_guard().await;
        let recovered = sqlx::query(
            "UPDATE scrobble_outbox SET state='uncertain', last_failure='interrupted', \
             updated_at=? WHERE state='sending' AND updated_at < ?",
        )
        .bind(now)
        .bind(cutoff)
        .execute(self.db.pool())
        .await?;
        Ok(recovered.rows_affected())
    }

    /// The rows that are due, under a link that is still good.
    async fn due_scrobbles(&self, now: i64) -> Result<Vec<DueEntry>, ServiceError> {
        let rows = sqlx::query(
            "SELECT o.id, o.link_id, o.attempts, o.played_at, o.title, o.artists_json, o.album, \
                    o.album_artist, o.duration_ms, o.musicbrainz_recording_id, \
                    l.provider, l.credential_nonce, l.credential_ciphertext \
             FROM scrobble_outbox o JOIN scrobble_link l ON l.id=o.link_id \
             WHERE o.state='pending' AND o.next_attempt_at <= ? AND l.status='active' \
             ORDER BY o.next_attempt_at, o.id LIMIT ?",
        )
        .bind(now)
        .bind(i64::try_from(self.scrobbling.batch).unwrap_or(i64::MAX))
        .fetch_all(self.db.pool())
        .await?;
        let mut entries = Vec::with_capacity(rows.len());
        for row in rows {
            let artists: Vec<String> = serde_json::from_str(row.try_get("artists_json")?)
                .map_err(|_| ServiceError::Invalid)?;
            entries.push(DueEntry {
                id: row.try_get("id")?,
                link_id: parse_uuid(row.try_get("link_id")?)?,
                provider: ScrobbleProvider::from_str(row.try_get("provider")?)?,
                nonce: row.try_get("credential_nonce")?,
                ciphertext: row.try_get("credential_ciphertext")?,
                attempts: row.try_get("attempts")?,
                envelope: ScrobbleEnvelope {
                    played_at: row.try_get("played_at")?,
                    title: row.try_get("title")?,
                    artists,
                    album: row.try_get("album")?,
                    album_artist: row.try_get("album_artist")?,
                    duration_ms: row.try_get("duration_ms")?,
                    musicbrainz_recording_id: row.try_get("musicbrainz_recording_id")?,
                },
            });
        }
        Ok(entries)
    }

    /// Writes one terminal state, and the link's last success with it.
    ///
    /// If this fails after a submission has already left, the pass aborts with
    /// the row still `sending`. That is the right resting place rather than an
    /// oversight: `recover_stale_sending` reads it as `uncertain` later, which
    /// is exactly what a listen that was emitted and never confirmed is. It
    /// must not be "repaired" into `pending` — that would send it again.
    /// Answers whether the row really moved.
    ///
    /// It can fail to, and the window is narrow but real. `recover_stale_sending`
    /// computes its cutoff *before* taking the writer gate, while this has to
    /// wait for that gate before writing — a wait a long scan can stretch past
    /// the margin. A concurrent pass may by then have read the row as abandoned
    /// and settled it `uncertain`, and the drain is public, so a manual or test
    /// pass really can run beside the background one.
    ///
    /// Where the row ends up is defensible either way: `uncertain` is the honest
    /// verdict for a listen whose fate was lost track of. What must not follow
    /// is the *reporting* carrying on regardless — counting an accepted
    /// submission that was never recorded, or stamping `last_success_at` for it.
    /// Decision 12 says what the API shows is a state and not a guess, and a
    /// counter describing a write that did not happen is a guess.
    async fn settle_scrobble(
        &self,
        entry: &DueEntry,
        state: &str,
        failure: Option<&str>,
    ) -> Result<bool, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let moved = sqlx::query(
            "UPDATE scrobble_outbox SET state=?, attempts=attempts+1, last_failure=?, \
             updated_at=? WHERE id=? AND state='sending'",
        )
        .bind(state)
        .bind(failure)
        .bind(now)
        .bind(entry.id)
        .execute(&mut *tx)
        .await?;
        if moved.rows_affected() == 0 {
            // The link updates below are rolled back with it: a success this
            // row cannot claim must not leave `last_success_at` behind.
            tx.rollback().await?;
            tracing::warn!(
                entry = entry.id,
                verdict = state,
                "a scrobble verdict arrived after its row had been settled elsewhere"
            );
            return Ok(false);
        }
        if state == "sent" {
            sqlx::query(
                "UPDATE scrobble_link SET last_success_at=?, last_failure=NULL, updated_at=? \
                 WHERE id=?",
            )
            .bind(now)
            .bind(now)
            .bind(entry.link_id.to_string())
            .execute(&mut *tx)
            .await?;
        } else if let Some(failure) = failure {
            sqlx::query("UPDATE scrobble_link SET last_failure=?, updated_at=? WHERE id=?")
                .bind(failure)
                .bind(now)
                .bind(entry.link_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Moves one entry out of the way without spending an attempt.
    ///
    /// For the row nothing can carry yet: the destination is linked and the
    /// listen is real, but this process knows no adapter for it. Deferring by
    /// one drain interval is what keeps it from starving the destinations that
    /// can be reached, and keeping `attempts` where it is means the row loses
    /// nothing when the operator supplies the missing adapter.
    ///
    /// Refusing the link instead would be the other way to prevent this, and it
    /// is the wrong one: a durable queue exists precisely so that a listen
    /// survives until the thing that carries it arrives.
    /// `wait` is how far out to push it: one drain interval for a row nothing
    /// can carry, and the delay a destination asked for when the link is resting
    /// under a rate limit. Never an attempt — this only moves a row out of the
    /// way, and `attempts` is what says something was tried.
    /// Answers whether the row really moved, on the same terms as every other
    /// writer here: a pass counts what it wrote and not what it attempted, and a
    /// row a concurrent pass has already taken is not this one's to report.
    async fn defer_scrobble(&self, entry: &DueEntry, wait: i64) -> Result<bool, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let moved = sqlx::query(
            "UPDATE scrobble_outbox SET next_attempt_at=?, updated_at=? \
             WHERE id=? AND state='pending'",
        )
        .bind(now.saturating_add(wait))
        .bind(now)
        .bind(entry.id)
        .execute(self.db.pool())
        .await?;
        if moved.rows_affected() != 1 {
            // Said aloud, like every other "this row was not ours" path here.
            // A pass that lost each of its deferrals to a concurrent one would
            // otherwise count nothing and log nothing, which reads exactly like
            // a pass with nothing to do.
            //
            // The tense matters and the first draft had it wrong: this fires on
            // the branch where the row was *not* moved aside, so saying it was
            // would report the very action that did not happen.
            tracing::warn!(
                entry = entry.id,
                "a row could not be moved aside; it had been settled elsewhere"
            );
            return Ok(false);
        }
        Ok(true)
    }

    /// Pushes one entry out to its next attempt, or gives up on it.
    ///
    /// A queue that never empties is a fault and not a state — decision 6 — so
    /// the attempts are bounded and what is abandoned is counted.
    ///
    /// Answers [`Rescheduled::NotOurs`] when the row had already been settled
    /// elsewhere, on the same terms and for the same reason as
    /// [`Self::settle_scrobble`].
    async fn reschedule_scrobble(
        &self,
        entry: &DueEntry,
        after: Option<Duration>,
    ) -> Result<Rescheduled, ServiceError> {
        let attempts = entry.attempts.saturating_add(1);
        let exhausted =
            u64::try_from(attempts).unwrap_or(u64::MAX) >= u64::from(self.scrobbling.max_attempts);
        if exhausted {
            return Ok(
                if self
                    .settle_scrobble(entry, "abandoned", Some("attempts_exhausted"))
                    .await?
                {
                    Rescheduled::Abandoned
                } else {
                    Rescheduled::NotOurs
                },
            );
        }
        let now = now_ms();
        // The longer of the two, so a destination asking for room gets at least
        // what it asked for and our own backoff is never shortened by it — but
        // never longer than the backoff's own ceiling either.
        //
        // `after` is the only value in this whole scheduling path that a third
        // party chooses, and without the clamp a single
        // `x-ratelimit-reset-in: 999999999999` parks the row at a moment it
        // never reaches. That is decision 10's concern arriving from the
        // unexpected side: not somebody making this server call a URL, but
        // somebody making it stop. One hour rather than a larger number of its
        // own, because `RETRY_CEILING` is already documented as the point past
        // which waiting buys nothing — a destination should not get to buy what
        // our own backoff calls worthless — and because the attempt cap spans
        // about a day, so a longer clamp would let one answer eat the entire
        // budget and decide the listen dies.
        //
        // Clamping is not ignoring: past the ceiling this waits an hour, asks
        // again, is refused again, and waits again. What it costs is one
        // attempt per refusal, which is what the cap is for.
        let ceiling = RETRY_CEILING_MS;
        let asked = after
            .map_or(0, |after| {
                i64::try_from(after.as_millis()).unwrap_or(i64::MAX)
            })
            .min(ceiling);
        // The spread is applied *after* the two are compared, not folded into
        // either of them. That is the whole reason `retry_base` and
        // `retry_spread` are separate: a single function returning base plus
        // spread, compared against `asked` with `max`, drops the spread
        // precisely when `asked` wins — and every rate-limited row in the batch
        // then comes back at the same millisecond, in a herd, at a destination
        // that had just asked for room.
        let base = retry_base(attempts).max(asked);
        let wait = base.saturating_add(retry_spread(base, entry.id));
        let _writer = self.db.writer_guard().await;
        // Two causes rather than one, because `ScrobbleLinkState::last_failure`
        // is documented as carrying a *normalised* cause and named
        // `rate_limited` among them — while nothing ever wrote it. A comment
        // asserting a value the code never produces is the category this branch
        // set out to be rid of, so the code now produces it.
        let cause = if after.is_some() {
            "rate_limited"
        } else {
            "retryable"
        };
        let mut tx = self.db.pool().begin().await?;
        let moved = sqlx::query(
            "UPDATE scrobble_outbox SET state='pending', attempts=?, next_attempt_at=?, \
             last_failure=?, updated_at=? WHERE id=? AND state='sending'",
        )
        .bind(attempts)
        .bind(now.saturating_add(wait))
        .bind(cause)
        .bind(now)
        .bind(entry.id)
        .execute(&mut *tx)
        .await?;
        // The link's own column as well, and this is the half that was missing.
        //
        // `ScrobbleLinkState::last_failure` is read from `scrobble_link`, never
        // from the row — so writing the cause onto the outbox entry alone left
        // decision 12's normalised `rate_limited` unreachable by anything that
        // reports it. An earlier commit message in this branch claimed that gap
        // was closed. It was not; this closes it.
        //
        // Only when the row really moved, on the same terms as every counter in
        // the drain: a verdict about a row this pass no longer owns must not
        // rewrite the link's state either.
        if moved.rows_affected() == 1 {
            sqlx::query("UPDATE scrobble_link SET last_failure=?, updated_at=? WHERE id=?")
                .bind(cause)
                .bind(now)
                .bind(entry.link_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        // Rolled back rather than committed empty, on the same terms as
        // `settle_scrobble`: a verdict about a row this pass no longer owns
        // writes nothing, and an empty transaction that commits is a write a
        // reader has to reason about before discovering it is not one.
        //
        // An earlier commit message in this branch said this was already done.
        // It was not — the sentence was written from the plan instead of from
        // the diff, which is the third time that has happened here, and naming
        // it is cheaper than the review round it cost.
        if moved.rows_affected() == 0 {
            tx.rollback().await?;
        } else {
            tx.commit().await?;
        }
        Ok(if moved.rows_affected() == 0 {
            tracing::warn!(
                entry = entry.id,
                "a retryable verdict arrived after its row had been settled elsewhere"
            );
            Rescheduled::NotOurs
        } else {
            Rescheduled::Later
        })
    }

    /// Marks one link broken and finishes its queue.
    ///
    /// Every waiting row under it would fail identically, so leaving them
    /// pending would be asking the same refused question a few thousand times.
    async fn mark_link_broken(&self, link_id: Uuid, cause: &str) -> Result<(), ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        sqlx::query(
            "UPDATE scrobble_link SET status='broken', last_failure=?, updated_at=? WHERE id=?",
        )
        .bind(cause)
        .bind(now)
        .bind(link_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE scrobble_outbox SET state='cancelled', last_failure=?, updated_at=? \
             WHERE link_id=? AND state='pending'",
        )
        .bind(cause)
        .bind(now)
        .bind(link_id.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Where a link stands, from its status and the shape of its queue.
///
/// Kept apart from the query so the rule can be read and exercised on its own:
/// the whole point of decision 12 is that this is a judgement and not a column.
fn link_health(
    status: &str,
    uncertain: i64,
    oldest_pending_at: Option<i64>,
    now: i64,
    stale_after: Duration,
) -> &'static str {
    if status == "broken" {
        return "broken";
    }
    if uncertain > 0 {
        return "degraded";
    }
    let stale_ms = i64::try_from(stale_after.as_millis()).unwrap_or(i64::MAX);
    if oldest_pending_at.is_some_and(|oldest| now.saturating_sub(oldest) > stale_ms) {
        return "degraded";
    }
    "healthy"
}

/// The growing half: doubling from a minute, capped at an hour.
///
/// Apart from the spread so that a caller with its own floor to apply — a
/// destination that asked for room — can take the larger of the two *bases* and
/// still get a spread on the result. Folded together, the spread is lost in
/// exactly the case that most needs it.
fn retry_base(attempts: i64) -> i64 {
    let step = u32::try_from(attempts.saturating_sub(1)).unwrap_or(u32::MAX);
    let base = RETRY_BASE
        .saturating_mul(2u32.saturating_pow(step.min(16)))
        .min(RETRY_CEILING);
    i64::try_from(base.as_millis()).unwrap_or(i64::MAX)
}

/// The spreading half: up to a quarter of the wait, added.
///
/// The id is **mixed** before it is folded into the interval, and that is the
/// whole of what makes this worth having. Rowids are consecutive, so
/// `entry_id % spread` put two neighbouring rows one millisecond apart inside a
/// fifteen-minute window — a spread that existed, passed its test, and did
/// nothing. A review caught it; the test could not, because `assert_ne!` tells
/// "absent" from "present" and never "present and useless".
///
/// Multiplied by the odd golden-ratio constant and read from the high bits,
/// because those are the bits a multiplicative hash actually scrambles — the low
/// ones keep the input's own structure, which is the structure being escaped.
/// Still no randomness: two rows must land apart, not unpredictably.
fn retry_spread(base_ms: i64, entry_id: i64) -> i64 {
    let spread = base_ms / 4;
    if spread <= 0 {
        return 0;
    }
    let mixed = entry_id.unsigned_abs().wrapping_mul(0x9E37_79B9_7F4A_7C15);
    // The shift leaves a value below 2^32, so both conversions are exact and
    // there is no fallback branch — nothing unreachable for a later reader to
    // mistake for a case that happens.
    i64::from((mixed >> 32) as u32) % spread
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire name, the database name, `as_str` and `FromStr` are one fact.
    ///
    /// They were four separate facts until a review noticed that
    /// `rename_all = "snake_case"` serialised `ListenBrainz` as
    /// `listen_brainz`: a link list would have published a destination that
    /// `FromStr` refuses and the `CHECK` constraint has never heard of, so a
    /// client could read back a value it could not send in. Nothing had noticed
    /// because the surface that serialises it is not written yet — which is
    /// precisely when this costs nothing to hold still.
    #[test]
    fn the_four_spellings_of_a_destination_agree() {
        for provider in [
            ScrobbleProvider::ListenBrainz,
            ScrobbleProvider::Maloja,
            ScrobbleProvider::LastFm,
        ] {
            let name = provider.as_str();
            assert_eq!(
                serde_json::to_string(&provider).unwrap(),
                format!("\"{name}\""),
                "what the API publishes must be what the database stores"
            );
            assert_eq!(ScrobbleProvider::from_str(name).unwrap(), provider);
        }
    }

    #[test]
    fn a_link_that_answers_is_still_degraded_while_its_queue_does_not_move() {
        let hour = Duration::from_secs(3600);
        let now = 10_000_000;

        // Nothing waiting, nothing ambiguous.
        assert_eq!(link_health("active", 0, None, now, hour), "healthy");

        // Waiting, but not for long.
        assert_eq!(
            link_health("active", 0, Some(now - 60_000), now, hour),
            "healthy"
        );

        // This is the silent failure a durable queue exists to make visible: the
        // token is good, the destination answers, and nothing has moved for
        // hours.
        assert_eq!(
            link_health("active", 0, Some(now - 6 * 3_600_000), now, hour),
            "degraded"
        );

        // One ambiguous entry is enough: it is waiting on a person, not on a
        // network, and nothing will move it on its own.
        assert_eq!(link_health("active", 1, None, now, hour), "degraded");

        // A broken authorisation outranks both.
        assert_eq!(
            link_health("broken", 3, Some(now - 6 * 3_600_000), now, hour),
            "broken"
        );
    }

    /// The ordinary wait, composed the way the drain composes it.
    ///
    /// The two halves are separate in production so that a caller with a floor
    /// of its own — a destination that asked for room — can take the larger of
    /// the two *bases* and still get a spread on the result. A test that wants
    /// the plain wait therefore has to put them back together rather than call
    /// a third function that exists only for it to call.
    fn wait(attempts: i64, entry_id: i64) -> i64 {
        let base = retry_base(attempts);
        base.saturating_add(retry_spread(base, entry_id))
    }

    /// Neighbouring rows must land *far* apart, not merely at different
    /// milliseconds.
    ///
    /// This is the property an integration test could not hold. Rowids are
    /// consecutive, so the unmixed spread put four rows at 1, 2, 3 and 4
    /// milliseconds inside a fifteen-minute window — distinct, and a herd all
    /// the same. `assert_ne!` tells "absent" from "present" and never "present
    /// and useless", so it certified a property the code did not deliver until
    /// a review read the arithmetic.
    ///
    /// A pure function, tested purely: dragging the drain, a database and an
    /// HTTP server through this proved less and cost more.
    #[test]
    fn the_spread_scatters_neighbouring_rows_across_the_interval() {
        let base = 3_600_000;
        let mut offsets: Vec<i64> = (1..=4).map(|id| retry_spread(base, id)).collect();
        for offset in &offsets {
            assert!(
                (0..base / 4).contains(offset),
                "the spread never leaves its quarter of the wait"
            );
        }
        offsets.sort_unstable();
        let smallest = offsets
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .min()
            .expect("four offsets have three gaps");
        assert!(
            smallest >= 1_000,
            "neighbouring rows land {smallest}ms apart, which is still a herd"
        );
    }

    /// The attempt cap and the schedule beside it describe one span of time, so
    /// what the cap is *for* is checked here rather than asserted in prose.
    ///
    /// A review found the comment on that constant claiming "most of a day"
    /// above a number that delivered two hours and three minutes. The sentence
    /// was the honest statement of the intention, so the number moved to meet
    /// it — and this exists so the two cannot drift apart again, which prose
    /// alone has already failed to prevent once.
    #[test]
    fn the_default_attempt_cap_carries_a_listen_across_a_day_of_outage() {
        let cap = i64::from(crate::config::DEFAULT_SCROBBLE_MAX_ATTEMPTS);
        // One submission per attempt, and one wait between each pair of them,
        // so a cap of `n` spends `n - 1` waits. Without jitter: the spread only
        // ever adds.
        let covered: i64 = (1..cap).map(|attempt| wait(attempt, 0)).sum();
        let hours = covered / 3_600_000;
        assert!(
            (23..=26).contains(&hours),
            "the default cap covers {hours}h of outage, which is not the day its comment claims"
        );
    }

    #[test]
    fn the_wait_grows_to_an_hour_and_two_rows_never_return_together() {
        let minute = 60_000;
        // The first retry waits about a minute, the later ones about an hour,
        // and never more.
        assert!((minute..minute + minute / 4).contains(&wait(1, 0)));
        assert!(wait(40, 0) <= 3_600_000 + 3_600_000 / 4);
        assert!(wait(40, 0) >= 3_600_000);
        // Monotonic while it grows.
        assert!(wait(3, 0) > wait(1, 0));
        // And two entries that failed in the same instant come back apart.
        assert_ne!(wait(2, 7), wait(2, 8));
    }
}
