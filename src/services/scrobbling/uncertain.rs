//! The listens whose fate is unknown, and the person's answer about each
//! (decision 13).

use super::*;

impl DomainServices {
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
            "SELECT o.public_id, l.provider, l.destination, o.played_at, o.attempts, \
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
                    // Defaulted for the reason `scrobble_links` defaults it:
                    // `initialize` fills every legacy row before the server
                    // answers anything, and a listing is not where a boot that
                    // went wrong should be discovered.
                    destination: row
                        .try_get::<Option<String>, _>("destination")?
                        .unwrap_or_default(),
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
        // One instant, named once and written twice: `updated_at` and
        // `terminal_at` describe the same write, and two `now_ms()` calls
        // would make them disagree by whatever the gate took.
        let discarded_at = now_ms();
        let _writer = self.db.writer_guard().await;
        let changed = sqlx::query(
            "UPDATE scrobble_outbox SET state='discarded', updated_at=?, terminal_at=? \
             WHERE public_id=? AND state='uncertain' AND retried_at IS NULL \
             AND link_id IN \
               (SELECT id FROM scrobble_link WHERE user_id=? AND status <> 'unlinked')",
        )
        .bind(discarded_at)
        .bind(discarded_at)
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
    ///
    /// **And it checks where it would go.** A live link is not enough: the
    /// instance that link names must still be declared, and its URL must still
    /// be the one the link was made against. Without that, the most dangerous
    /// entry in the whole design — a listen its owner accepts risking twice —
    /// would be the one that leaves for a machine nobody chose. The check is
    /// inside the same transaction as the UPDATE and precedes it.
    pub async fn retry_uncertain_scrobble(
        &self,
        user_id: Uuid,
        entry: Uuid,
    ) -> Result<Uuid, ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        // Read inside the transaction, before the claim, so the identity this
        // answers for is the identity the row is claimed under. Outside it, a
        // reconciliation could move between the two.
        let addressed = sqlx::query(
            "SELECT l.provider, l.destination, l.destination_fingerprint \
             FROM scrobble_outbox o JOIN scrobble_link l ON l.id=o.link_id \
             WHERE o.public_id=? AND l.user_id=? AND l.status='active'",
        )
        .bind(entry.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(addressed) = addressed else {
            return Err(ServiceError::NotFound);
        };
        let provider = ScrobbleProvider::from_str(addressed.try_get("provider")?)?;
        let destination: Option<String> = addressed.try_get("destination")?;
        let fingerprint: Option<String> = addressed.try_get("destination_fingerprint")?;
        // A 404 rather than a distinct answer, like every other refusal on this
        // path: "already answered", "not yours" and "the destination moved" are
        // one reply, and the entry simply stops being findable.
        let still_there = destination
            .and_then(|destination| self.scrobble_destinations.get(&(provider, destination)))
            .is_some_and(|declared| Some(declared) == fingerprint.as_ref());
        if !still_there {
            return Err(ServiceError::NotFound);
        }
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
        //
        // `RETURNING` rather than a second lookup by `public_id`: `retry_of`,
        // the ordering and the jitter all speak in rowids, and this hands back
        // the rowid of the row the UPDATE itself took. A `SELECT` afterwards
        // would name the same row today and would be a separate claim about
        // which row that is.
        let claimed = sqlx::query_scalar::<_, i64>(
            "UPDATE scrobble_outbox SET retried_at=? \
             WHERE public_id=? AND state='uncertain' AND retried_at IS NULL \
               AND link_id IN \
                 (SELECT id FROM scrobble_link WHERE user_id=? AND status='active') \
             RETURNING id",
        )
        .bind(now)
        .bind(entry.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(rowid) = claimed else {
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
}
