//! Links between an account and a destination: the published destinations,
//! linking and unlinking, the link states, and reconciling links against the
//! destinations the operator declares.

use super::*;

impl DomainServices {
    /// Teaches the drain how to reach one destination.
    ///
    /// Registered after `initialize` rather than built into the services, which
    /// is what lets a test drive the whole queue against a double and lets an
    /// operator run a server with no outbound adapter at all.
    pub fn register_scrobble_target(
        &self,
        provider: ScrobbleProvider,
        destination: &str,
        target: Arc<dyn ScrobbleTarget>,
    ) {
        self.scrobble_targets
            .insert((provider, destination.to_owned()), target);
    }

    /// Every instance a member may name, by recipient.
    ///
    /// Decision 10's barrier from the inside: an account picks among these and
    /// describes no URL, so something has to publish the names. The addresses
    /// are deliberately not here — a member never needs one, and a link never
    /// holds one either.
    pub fn scrobble_destinations(&self) -> Vec<ScrobbleDestinationName> {
        let mut named: Vec<ScrobbleDestinationName> = self
            .scrobble_destinations
            .keys()
            .map(|(provider, name)| {
                let unavailable = (*provider == ScrobbleProvider::LastFm)
                    .then_some(self.lastfm_unavailable)
                    .flatten();
                ScrobbleDestinationName {
                    provider: *provider,
                    destination: name.clone(),
                    available: unavailable.is_none(),
                    unavailable,
                }
            })
            .collect();
        // A `HashMap` has no order and an API that reorders itself between two
        // identical calls is one nobody can diff.
        named.sort_by(|left, right| {
            (left.provider.as_str(), &left.destination)
                .cmp(&(right.provider.as_str(), &right.destination))
        });
        named
    }

    /// Authorises this account to submit to one destination, in a new
    /// generation.
    ///
    /// Any live generation for the same **triple** — account, recipient,
    /// instance — is unlinked first, and its queue finishes with it — decision
    /// 4. The two happen under one transaction, so there is no instant at which
    /// a waiting listen belongs to no authorisation.
    ///
    /// **The instance is part of what is replaced, and that is not a detail.**
    /// While this unlinked by recipient alone, linking `maloja/bob` withdrew
    /// `maloja/alice` and cancelled her queue: the feature would have destroyed
    /// itself on its second use.
    ///
    /// An instance the operator never declared answers `NotFound`. A member
    /// picks among the names the server publishes; they do not describe one.
    pub async fn link_scrobble(
        &self,
        user_id: Uuid,
        provider: ScrobbleProvider,
        destination: &str,
        secret: &str,
    ) -> Result<Uuid, ServiceError> {
        let fingerprint = self
            .scrobble_destinations
            .get(&(provider, destination.to_owned()))
            .ok_or(ServiceError::NotFound)?
            .clone();
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
        let mut tx = self.db.begin_write(&_writer).await?;
        self.unlink_scrobble_on(&mut tx, user_id, provider, destination, now)
            .await?;
        sqlx::query(
            "INSERT INTO scrobble_link (id, user_id, provider, destination, \
             destination_fingerprint, status, credential_nonce, credential_ciphertext, \
             created_at, updated_at) VALUES (?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(provider.as_str())
        .bind(destination)
        .bind(&fingerprint)
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
        destination: &str,
    ) -> Result<bool, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        let unlinked = self
            .unlink_scrobble_on(&mut tx, user_id, provider, destination, now)
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
    ///
    /// **Scoped to one instance, and that is load-bearing.** It withdraws "this
    /// account's live authorisation at this *instance*", not "at this
    /// recipient". Keyed by recipient alone — which is how it was written while
    /// there could only be one — `link_scrobble("maloja", "bob")` would have
    /// withdrawn `maloja/alice` and cancelled her queue, so naming a second
    /// instance destroyed the first. The index, the registry and this clause
    /// are one change in three places: correcting two of the three gives a
    /// server that accepts two instances and erases one.
    ///
    /// A legacy row whose destination `initialize` has not filled yet matches
    /// nothing here, which is right: nothing can be linked before the catch-up
    /// has run, because `initialize` runs before anything is served.
    async fn unlink_scrobble_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        provider: ScrobbleProvider,
        destination: &str,
        now: i64,
    ) -> Result<bool, ServiceError> {
        let live = sqlx::query_scalar::<_, String>(
            "SELECT id FROM scrobble_link \
             WHERE user_id=? AND provider=? AND destination=? AND status <> 'unlinked'",
        )
        .bind(user_id.to_string())
        .bind(provider.as_str())
        .bind(destination)
        .fetch_optional(&mut *connection)
        .await?;
        let Some(link_id) = live else {
            return Ok(false);
        };
        sqlx::query(
            "UPDATE scrobble_outbox SET state='cancelled', last_failure='unlinked', updated_at=?, terminal_at=? \
             WHERE link_id=? AND state='pending'",
        )
        .bind(now)
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
            "SELECT id, provider, destination, status, last_success_at, last_failure \
             FROM scrobble_link \
             WHERE user_id=? AND status <> 'unlinked' ORDER BY provider, destination",
        )
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let mut states = Vec::with_capacity(rows.len());
        let now = now_ms();
        for row in rows {
            let link_id: String = row.try_get("id")?;
            let provider = ScrobbleProvider::from_str(row.try_get("provider")?)?;
            // Never null by the time anything reads this: `initialize` fills
            // every legacy row before the server answers, and breaks the ones it
            // cannot fill. Defaulted rather than unwrapped all the same — a
            // listing is not the place to discover a boot went wrong.
            let destination: Option<String> = row.try_get("destination")?;
            // `SUM(CASE …)` rather than `COUNT(*) FILTER`, because one query
            // answering four counts is the point. An earlier note here gave a
            // second reason — that the aggregate filter wanted a SQLite newer
            // than this crate builds against — and it was simply untrue: the
            // library is bundled, at 3.51, and the schema has asked for 3.37
            // since the first `STRICT` table.
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
                destination: destination.unwrap_or_default(),
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

    /// Fills in what old links never carried, then checks every link still
    /// points where it was made to point.
    ///
    /// Called once by `initialize`, under the writer gate, **before**
    /// `spawn_scrobble_drain`. The order is half the rule: reconciling while
    /// the drain already runs leaves a window — short, and quite sufficient —
    /// in which listens leave for yesterday's destination. Nothing else needs
    /// guarding, because once a link is `broken` the queueing writes nothing
    /// more for it.
    ///
    /// **The catch-up asserts; it cannot verify.** Nothing in the database says
    /// which URL was configured yesterday, so writing today's fingerprint onto
    /// a legacy link declares that today's instance is the one it meant. That
    /// is true while there is only one possible answer, which is the case when
    /// this ships. Where a recipient has legacy links *and* the configuration
    /// already declares several instances for it, there is no answer, and the
    /// server refuses to start rather than pick one: guessing here would send
    /// waiting listens to the wrong profile with nothing to show for it. The
    /// way out needs no new machinery — boot once with a single instance
    /// declared, the one those links meant, and add the others next boot.
    ///
    /// **None of that concerns a fresh install.** The refusal is about an
    /// ambiguous catch-up, and with no legacy link there is nothing to catch
    /// up: an empty database starts with as many instances as the operator
    /// declares, the first time. Not saying so would have made the feature
    /// unreachable to everyone with nothing to migrate.
    ///
    /// **A recipient with links and no declared instance at all** — a URL
    /// emptied out of the configuration, which is already a way of switching it
    /// off — has nothing to write. Those links are treated as a vanished
    /// destination: broken, with their queue finished, rather than left holding
    /// two empty columns the next reconciliation could not read.
    ///
    /// The deployment consequence, said rather than left to be discovered:
    /// whoever changes an instance's URL *and then* updates will have the
    /// catch-up ratify the change, because the guard compares against what was
    /// written and nothing was. Update first, change the URL after.
    pub async fn reconcile_scrobble_links(&self) -> Result<(), ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;

        let legacy = sqlx::query(
            "SELECT id, provider FROM scrobble_link \
             WHERE destination IS NULL AND status <> 'unlinked'",
        )
        .fetch_all(&mut *tx)
        .await?;
        for row in &legacy {
            let provider = ScrobbleProvider::from_str(row.try_get("provider")?)?;
            let declared: Vec<(&String, &String)> = self
                .scrobble_destinations
                .iter()
                .filter(|((declared, _), _)| *declared == provider)
                .map(|((_, name), fingerprint)| (name, fingerprint))
                .collect();
            match declared.as_slice() {
                [(name, fingerprint)] => {
                    sqlx::query(
                        "UPDATE scrobble_link SET destination=?, destination_fingerprint=?, \
                         updated_at=? WHERE id=?",
                    )
                    .bind(name.as_str())
                    .bind(fingerprint.as_str())
                    .bind(now)
                    .bind(row.try_get::<String, _>("id")?)
                    .execute(&mut *tx)
                    .await?;
                }
                [] => {
                    // Named before it is broken, and named `default`.
                    //
                    // Not a guess: a link made under #191–#193 carried no name
                    // because its recipient had exactly one instance, and
                    // `DEFAULT_SCROBBLE_DESTINATION` is what a bare URL in the
                    // configuration is still called. The link is broken either
                    // way and nothing is remapped — what the name buys is that
                    // the row can be spoken about.
                    //
                    // Left NULL, it came out of `scrobble_links` and
                    // `uncertain_scrobbles` as the empty string, so the API
                    // published a pair with no instance in it. Worse, the row
                    // could not be withdrawn: `unlink_scrobble_on` matches
                    // `destination=?`, and no comparison is ever true of NULL,
                    // so the listing showed a broken link with a blank name
                    // that no gesture could remove.
                    //
                    // The fingerprint stays NULL. It exists to be compared
                    // against a declared URL, there is none, and a broken link
                    // is never reconciled again — the second pass below reads
                    // `status='active'` only.
                    sqlx::query("UPDATE scrobble_link SET destination=?, updated_at=? WHERE id=?")
                        .bind(crate::config::DEFAULT_SCROBBLE_DESTINATION)
                        .bind(now)
                        .bind(row.try_get::<String, _>("id")?)
                        .execute(&mut *tx)
                        .await?;
                    self.break_link_on(&mut tx, row.try_get("id")?, "destination_gone", now)
                        .await?;
                }
                _ => {
                    // Refused rather than guessed, and refused loudly: a
                    // startup that picked one would hand somebody else's
                    // instance a queue of listens and say nothing.
                    return Err(ServiceError::Conflict);
                }
            }
        }

        // And then every link, legacy or not, against what is declared now.
        let links = sqlx::query(
            "SELECT id, provider, destination, destination_fingerprint FROM scrobble_link \
             WHERE status='active'",
        )
        .fetch_all(&mut *tx)
        .await?;
        for row in &links {
            let provider = ScrobbleProvider::from_str(row.try_get("provider")?)?;
            let destination: Option<String> = row.try_get("destination")?;
            let fingerprint: Option<String> = row.try_get("destination_fingerprint")?;
            let matches = destination
                .and_then(|destination| self.scrobble_destinations.get(&(provider, destination)))
                .is_some_and(|declared| Some(declared) == fingerprint.as_ref());
            if !matches {
                // One cause for both halves. A name that disappeared and a name
                // pointing somewhere else are the same event for the account:
                // the machine this authorisation was for is not reachable under
                // it any more, and nothing is silently remapped onto a sibling.
                self.break_link_on(&mut tx, row.try_get("id")?, "destination_gone", now)
                    .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Breaks one link and finishes what was waiting under it.
    ///
    /// The queue has to finish, not wait. A `broken` link whose rows stay
    /// `pending` is a queue nothing will ever empty: `pending` escapes
    /// retention, so the table keeps those rows for ever, and
    /// `oldest_pending_at` holds the link `degraded` for ever with them — the
    /// same defect this slice corrects twice elsewhere. Unlinking already does
    /// exactly this, for exactly this reason.
    async fn break_link_on(
        &self,
        connection: &mut SqliteConnection,
        link_id: String,
        cause: &str,
        now: i64,
    ) -> Result<(), ServiceError> {
        sqlx::query(
            "UPDATE scrobble_link SET status='broken', last_failure=?, updated_at=? WHERE id=?",
        )
        .bind(cause)
        .bind(now)
        .bind(&link_id)
        .execute(&mut *connection)
        .await?;
        sqlx::query(
            "UPDATE scrobble_outbox SET state='cancelled', last_failure=?, updated_at=?, \
             terminal_at=? WHERE link_id=? AND state='pending'",
        )
        .bind(cause)
        .bind(now)
        .bind(now)
        .bind(&link_id)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }
}

/// Where a link stands, from its status and the shape of its queue.
///
/// Kept apart from the query so the rule can be read and exercised on its own:
/// the whole point of decision 12 is that this is a judgement and not a column.
pub(super) fn link_health(
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
