//! When a retryable entry is tried again: the growing wait, its spread, and
//! the rescheduling that spends an attempt.

use super::*;

/// The floor of the growing wait between two attempts.
const RETRY_BASE: Duration = Duration::from_secs(60);
/// Its ceiling in milliseconds.
///
/// One number, spelled twice from itself. Converting the `Duration` at each use
/// needed a fallback that could never fire, in a file that argues a few hundred
/// lines below that an unreachable branch is a case a later reader will mistake
/// for one that happens.
pub(super) const RETRY_CEILING_MS: i64 = 60 * 60 * 1_000;
/// Its ceiling. Past this, waiting longer buys nothing a restart would not.
const RETRY_CEILING: Duration = Duration::from_millis(RETRY_CEILING_MS as u64);

impl DomainServices {
    /// Pushes one entry out to its next attempt, or gives up on it.
    ///
    /// A queue that never empties is a fault and not a state — decision 6 — so
    /// the attempts are bounded and what is abandoned is counted.
    ///
    /// Answers [`Rescheduled::NotOurs`] when the row had already been settled
    /// elsewhere, on the same terms and for the same reason as
    /// [`Self::settle_scrobble`].
    pub(super) async fn reschedule_scrobble(
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
        let mut tx = self.db.begin_write(&_writer).await?;
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
}

/// The growing half: doubling from a minute, capped at an hour.
///
/// Apart from the spread so that a caller with its own floor to apply — a
/// destination that asked for room — can take the larger of the two *bases* and
/// still get a spread on the result. Folded together, the spread is lost in
/// exactly the case that most needs it.
pub(super) fn retry_base(attempts: i64) -> i64 {
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
pub(super) fn retry_spread(base_ms: i64, entry_id: i64) -> i64 {
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
