//! Trimming the finished end of the queue once its retention has passed.

use super::*;

/// How often the finished end of the queue is trimmed.
///
/// Retention is measured in days, so a pass a day is the coarsest interval that
/// still honours it: the oldest surviving entry is never more than a day past
/// the bound. The other sweepers in this crate have the same shape.
const SCROBBLE_PURGE_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// How many finished entries one turn of the writer gate takes.
///
/// A thousand is a delete that finishes in milliseconds on any disk this server
/// runs on, and a backlog of a year's listening is a few dozen of them.
const SCROBBLE_PURGE_CHUNK: i64 = 1_000;

impl DomainServices {
    /// Trims the queue to what the RFC's retention section says it keeps.
    ///
    /// The eighth background task, and the same shape as the seven others: a
    /// pass at boot, then one per interval. A server that has been down should
    /// not wait a full day to catch up on what it owes.
    pub fn spawn_scrobble_purge(&self) {
        let services = self.clone();
        tokio::spawn(async move {
            services.purge_scrobbles_now().await;
            let mut ticker = tokio::time::interval(SCROBBLE_PURGE_INTERVAL);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.purge_scrobbles_now().await;
            }
        });
    }

    async fn purge_scrobbles_now(&self) {
        let now = now_ms();
        match self.purge_scrobble_outbox(now).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(entries = removed, "finished scrobble entries trimmed"),
            Err(error) => tracing::warn!(%error, "could not trim the scrobble queue"),
        }
        // The same pass, because it exists anyway — and each journey's own
        // expiry rather than the queue's thirty-day window. Borrowing the wrong
        // one of the two would keep a fifteen-minute journey alive for a month.
        match self.purge_lastfm_authorizations(now).await {
            Ok(0) => {}
            Ok(removed) => {
                tracing::info!(journeys = removed, "expired last.fm authorisations dropped")
            }
            Err(error) => tracing::warn!(%error, "could not drop expired last.fm authorisations"),
        }
    }

    /// One pass. Public so a test can run it rather than wait a day for it.
    ///
    /// **Only rows carrying a terminal instant, and that is the whole guard.**
    /// `pending` and `sending` never have one, and neither does `uncertain` —
    /// answered or not, it waits for a person, and taking it away after a month
    /// would decide in their place. Nothing a link publishes is read from the
    /// rows this takes: the counters and the oldest wait come from `pending`
    /// and `uncertain`, and `last_success_at` is a column of `scrobble_link`.
    ///
    /// No floor, unlike the library event feed: nobody resumes a cursor here.
    ///
    /// `now_ms` is a parameter for the reason `purge_library_events` takes one:
    /// the bound is exclusive, so a caller that cannot name the instant cannot
    /// place a row *on* it, only a few milliseconds either side of it.
    pub async fn purge_scrobble_outbox(&self, now_ms: i64) -> Result<u64, ServiceError> {
        // Whole days in milliseconds, checked: a value large enough to overflow
        // means "keep everything", which is what the absent cutoff below does.
        let cutoff = i64::from(self.scrobbling.retention_days)
            .checked_mul(24 * 60 * 60 * 1000)
            .and_then(|window| now_ms.checked_sub(window));
        let Some(cutoff) = cutoff else {
            return Ok(0);
        };
        let mut removed = 0;
        loop {
            // One chunk per turn of the gate rather than one statement for the
            // whole backlog. The writer gate is process-wide, and a server
            // coming back to a year of finished entries would otherwise hold
            // every other mutation still for the length of one delete.
            let _writer = self.db.writer_guard().await;
            let cut = sqlx::query(
                "DELETE FROM scrobble_outbox WHERE id IN ( \
                   SELECT id FROM scrobble_outbox \
                    WHERE terminal_at IS NOT NULL AND terminal_at < ? \
                    ORDER BY terminal_at LIMIT ?)",
            )
            .bind(cutoff)
            .bind(SCROBBLE_PURGE_CHUNK)
            .execute(self.db.pool())
            .await?;
            drop(_writer);
            removed += cut.rows_affected();
            if cut.rows_affected() < SCROBBLE_PURGE_CHUNK as u64 {
                return Ok(removed);
            }
        }
    }
}

/// The five states retention counts from, named once.
///
/// `sent`, `rejected`, `abandoned`, `cancelled` and `discarded` are finished
/// and eventually go. `uncertain` is terminal for the server and deliberately
/// gets nothing: it is kept for as long as nobody has answered it, and an
/// instant on it would be an invitation to start counting one day. `pending`
/// and `sending` are not terminal at all.
///
/// A function rather than a `CASE` in each statement because there are four
/// writers, and the one that forgot would leave a row no purge could ever see —
/// the same shape of silent survival `retried_at` was added to close.
pub(super) fn retention_instant(state: &str, now: i64) -> Option<i64> {
    matches!(
        state,
        "sent" | "rejected" | "abandoned" | "cancelled" | "discarded"
    )
    .then_some(now)
}
