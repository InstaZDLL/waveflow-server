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
    Retryable,
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

/// What one link's queue looks like from outside.
///
/// Counters, never content. Decision 12: what the API shows is a state, not an
/// echo of the envelope nor of the destination's own words.
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
    /// Ambiguous, and still asking the person to decide. A retried one stops
    /// being counted here without being erased — see [`DomainServices::retry_uncertain_scrobble`].
    pub uncertain: i64,
    pub oldest_pending_at: Option<i64>,
    pub last_success_at: Option<i64>,
    /// A normalised cause — `rate_limited`, `auth_broken` — or nothing.
    pub last_failure: Option<String>,
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
    /// Rows left exactly as they were because no adapter is registered for
    /// their destination. Not an attempt: a server missing an adapter is a
    /// misconfiguration, and spending the listen's retries on it would destroy
    /// the queue the operator is about to fix.
    pub unserviced: usize,
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
/// Its ceiling. Past this, waiting longer buys nothing a restart would not.
const RETRY_CEILING: Duration = Duration::from_secs(60 * 60);
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
            let counts = sqlx::query(
                "SELECT \
                   SUM(CASE WHEN state='pending' AND attempts=0 THEN 1 ELSE 0 END) AS pending, \
                   SUM(CASE WHEN state='pending' AND attempts>0 THEN 1 ELSE 0 END) AS retrying, \
                   SUM(CASE WHEN state='uncertain' AND id NOT IN \
                     (SELECT retry_of FROM scrobble_outbox WHERE retry_of IS NOT NULL) \
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

    /// Throws away one ambiguous entry. The person prefers the gap.
    ///
    /// Named by its `public_id`, never by its rowid: the sequential one would
    /// tell anyone holding a single entry of their own how many listens this
    /// whole server has queued.
    pub async fn discard_uncertain_scrobble(
        &self,
        user_id: Uuid,
        entry: Uuid,
    ) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let changed = sqlx::query(
            "UPDATE scrobble_outbox SET state='discarded', updated_at=? WHERE public_id=? \
             AND state='uncertain' AND link_id IN (SELECT id FROM scrobble_link WHERE user_id=?)",
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
    pub async fn retry_uncertain_scrobble(
        &self,
        user_id: Uuid,
        entry: Uuid,
    ) -> Result<Uuid, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        // The link has to be live: a retry under a withdrawn authorisation
        // would submit to whichever profile happens to be linked now, which is
        // the very substitution generations exist to prevent.
        //
        // And the entry must not already have been retried. The original stays
        // `uncertain` for good — erasing it would falsify the only trace
        // explaining why a duplicate exists — so nothing in the row itself says
        // it has been answered, and without this clause a second call would
        // queue a second copy, and a third a third. Decision 13 gives that
        // acceptance once, for one listen, deliberately. The unique index on
        // `retry_of` refuses the insert as well; this clause is what turns the
        // refusal into an ordinary 404 instead of a constraint error.
        // The rowid comes back from this lookup rather than from the caller:
        // `retry_of`, the ordering and the jitter all speak in rowids, and the
        // public name is resolved to one exactly here, once.
        let rowid = sqlx::query_scalar::<_, i64>(
            "SELECT o.id FROM scrobble_outbox o JOIN scrobble_link l ON l.id=o.link_id \
             WHERE o.public_id=? AND o.state='uncertain' AND l.user_id=? AND l.status='active' \
               AND NOT EXISTS (SELECT 1 FROM scrobble_outbox r WHERE r.retry_of=o.id)",
        )
        .bind(entry.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(rowid) = rowid else {
            return Err(ServiceError::NotFound);
        };
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
        let interval = services.scrobbling.drain_interval;
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
        let mut stalled_links: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
        for entry in self.due_scrobbles(now_ms()).await? {
            if broken_links.contains(&entry.link_id) || stalled_links.contains(&entry.link_id) {
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
                self.defer_scrobble(&entry).await?;
                drained.unserviced += 1;
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
                    // this pass; see `stalled_links` above.
                    stalled_links.insert(entry.link_id);
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
                ScrobbleVerdict::Retryable => match self.reschedule_scrobble(&entry).await? {
                    Rescheduled::Later => drained.retrying += 1,
                    Rescheduled::Abandoned => drained.abandoned += 1,
                    Rescheduled::NotOurs => {}
                },
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
    async fn defer_scrobble(&self, entry: &DueEntry) -> Result<(), ServiceError> {
        let now = now_ms();
        let wait = i64::try_from(self.scrobbling.drain_interval.as_millis()).unwrap_or(i64::MAX);
        let _writer = self.db.writer_guard().await;
        sqlx::query(
            "UPDATE scrobble_outbox SET next_attempt_at=?, updated_at=? \
             WHERE id=? AND state='pending'",
        )
        .bind(now.saturating_add(wait))
        .bind(now)
        .bind(entry.id)
        .execute(self.db.pool())
        .await?;
        Ok(())
    }

    /// Pushes one entry out to its next attempt, or gives up on it.
    ///
    /// A queue that never empties is a fault and not a state — decision 6 — so
    /// the attempts are bounded and what is abandoned is counted.
    ///
    /// Answers [`Rescheduled::NotOurs`] when the row had already been settled
    /// elsewhere, on the same terms and for the same reason as
    /// [`Self::settle_scrobble`].
    async fn reschedule_scrobble(&self, entry: &DueEntry) -> Result<Rescheduled, ServiceError> {
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
        let wait = retry_delay(attempts, entry.id);
        let _writer = self.db.writer_guard().await;
        let moved = sqlx::query(
            "UPDATE scrobble_outbox SET state='pending', attempts=?, next_attempt_at=?, \
             last_failure='retryable', updated_at=? WHERE id=? AND state='sending'",
        )
        .bind(attempts)
        .bind(now.saturating_add(wait))
        .bind(now)
        .bind(entry.id)
        .execute(self.db.pool())
        .await?;
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

/// How long to wait before attempt number `attempts`.
///
/// Doubling from a minute to an hour, spread by the row's own id. The spread
/// exists so that a queue which failed together does not come back together;
/// it has no reason to be unpredictable, which is why it costs no randomness —
/// two rows that failed in the same second still return at different moments.
fn retry_delay(attempts: i64, entry_id: i64) -> i64 {
    let step = u32::try_from(attempts.saturating_sub(1)).unwrap_or(u32::MAX);
    let base = RETRY_BASE
        .saturating_mul(2u32.saturating_pow(step.min(16)))
        .min(RETRY_CEILING);
    let base_ms = i64::try_from(base.as_millis()).unwrap_or(i64::MAX);
    // Up to a quarter of the wait, either side of nothing.
    let spread = base_ms / 4;
    let jitter = if spread > 0 {
        entry_id.rem_euclid(spread)
    } else {
        0
    };
    base_ms.saturating_add(jitter)
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
        let covered: i64 = (1..cap).map(|attempt| retry_delay(attempt, 0)).sum();
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
        assert!((minute..minute + minute / 4).contains(&retry_delay(1, 0)));
        assert!(retry_delay(40, 0) <= 3_600_000 + 3_600_000 / 4);
        assert!(retry_delay(40, 0) >= 3_600_000);
        // Monotonic while it grows.
        assert!(retry_delay(3, 0) > retry_delay(1, 0));
        // And two entries that failed in the same instant come back apart.
        assert_ne!(retry_delay(2, 7), retry_delay(2, 8));
    }
}
