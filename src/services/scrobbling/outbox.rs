//! The drain: claiming the entries that are due, handing each to its
//! destination's adapter, and settling it by the verdict.

use super::purge::retention_instant;
use super::retry::RETRY_CEILING_MS;
use super::*;

/// How far past the outbound deadline a claimed row must sit before it is read
/// as abandoned rather than in flight.
///
/// A live submission cannot outlast `request_timeout`, so any margin at all
/// would do; a generous one means a paused process — a laptop closing, a
/// container throttled — is never mistaken for a dead one and does not have a
/// listen declared uncertain out from under it.
const STALE_SENDING_MARGIN: Duration = Duration::from_secs(60);

impl DomainServices {
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
                .get(&(entry.provider, entry.destination.clone()))
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
                    l.provider, l.destination, l.credential_nonce, l.credential_ciphertext \
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
                destination: row
                    .try_get::<Option<String>, _>("destination")?
                    .unwrap_or_default(),
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
    pub(super) async fn settle_scrobble(
        &self,
        entry: &DueEntry,
        state: &str,
        failure: Option<&str>,
    ) -> Result<bool, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        let moved = sqlx::query(
            "UPDATE scrobble_outbox SET state=?, attempts=attempts+1, last_failure=?, \
             updated_at=?, terminal_at=? WHERE id=? AND state='sending'",
        )
        .bind(state)
        .bind(failure)
        .bind(now)
        .bind(retention_instant(state, now))
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

    /// Marks one link broken and finishes its queue.
    ///
    /// Every waiting row under it would fail identically, so leaving them
    /// pending would be asking the same refused question a few thousand times.
    async fn mark_link_broken(&self, link_id: Uuid, cause: &str) -> Result<(), ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        sqlx::query(
            "UPDATE scrobble_link SET status='broken', last_failure=?, updated_at=? WHERE id=?",
        )
        .bind(cause)
        .bind(now)
        .bind(link_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE scrobble_outbox SET state='cancelled', last_failure=?, updated_at=?, terminal_at=? \
             WHERE link_id=? AND state='pending'",
        )
        .bind(cause)
        .bind(now)
        .bind(now)
        .bind(link_id.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }
}
