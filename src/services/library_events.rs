//! Reading a library's change feed.
//!
//! The counterpart of the RFC-003 journal for the half of the server that had
//! no feed at all. Written by scans in `catalog.rs`; read here.

use super::*;
use crate::sync::MAX_SYNC_LIMIT;

/// How often the feed is trimmed.
///
/// Retention is measured in days, so a pass a day is the coarsest interval that
/// still honours it: the oldest surviving event is never more than a day past
/// the bound. The other sweepers in this crate have the same shape.
const PURGE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// What one pass of the retention purge removed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EventPurge {
    /// Rows cut, across every library.
    pub events_removed: u64,
    /// Libraries that lost at least one.
    pub libraries_trimmed: usize,
    /// Devices whose acknowledged cursor now sits below the watermark.
    ///
    /// They have been sent back to the catalogue snapshot. Counted rather than
    /// prevented — RFC-007 decision 8 says the acknowledgement informs and does
    /// not decide, or one forgotten phone would stop a shared library from ever
    /// being trimmed.
    pub devices_stranded: usize,
}

impl DomainServices {
    /// Trims every library's feed to what RFC-007 decision 7 says it keeps.
    ///
    /// Boot first, then a pass a day. The other sweepers in this crate have the
    /// same shape, and for the same reason: a server that has been down should
    /// not wait a full interval to catch up on what it owes.
    pub fn spawn_library_event_purge(&self) {
        let services = self.clone();
        tokio::spawn(async move {
            services.purge_now().await;
            let mut ticker = tokio::time::interval(PURGE_INTERVAL);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.purge_now().await;
            }
        });
    }

    async fn purge_now(&self) {
        match self.purge_library_events(now_ms()).await {
            Ok(purged) if purged == EventPurge::default() => {}
            Ok(purged) => tracing::info!(
                events = purged.events_removed,
                libraries = purged.libraries_trimmed,
                "library event feeds trimmed"
            ),
            Err(error) => tracing::warn!(%error, "could not trim the library event feeds"),
        }
    }

    /// One pass. Public so a test can run it rather than wait a day for it.
    ///
    /// Two bounds, and the floor wins. An age alone lets a library that rescans
    /// daily grow without limit; a count alone cuts the head off a quiet one
    /// whose ten thousand events cover two years.
    ///
    /// `now_ms` is a parameter for the same reason `sweep_expired_sessions`
    /// takes one: the bound is exclusive, so a caller that cannot name the
    /// instant cannot place an event *on* it — it can only put one a few
    /// milliseconds either side and hope. A test written that way passes or
    /// fails on how long the lines between it and here took to run.
    pub async fn purge_library_events(&self, now_ms: i64) -> Result<EventPurge, ServiceError> {
        let retention = self.library_event_retention;
        // Whole days in milliseconds, and the multiplication is checked: a
        // configured value large enough to overflow means "keep everything",
        // which is what a cutoff of zero produces anyway.
        let cutoff = i64::from(retention.days)
            .checked_mul(24 * 60 * 60 * 1000)
            .and_then(|window| now_ms.checked_sub(window));
        let Some(cutoff) = cutoff else {
            return Ok(EventPurge::default());
        };

        let libraries: Vec<String> = sqlx::query_scalar("SELECT id FROM library")
            .fetch_all(self.db.pool())
            .await?;
        let mut purged = EventPurge::default();
        for library_id in libraries {
            // One transaction per library rather than one for all of them: the
            // writer gate is process-wide, and holding it across every feed on
            // a server with fifty libraries would stall every other mutation
            // for the whole pass.
            let removed = self.purge_one_library(&library_id, cutoff).await?;
            if removed > 0 {
                purged.events_removed += removed;
                purged.libraries_trimmed += 1;
                // What the cut cost, and to whom. This is the whole of what
                // decision 8's table is for: the ack does not hold the purge
                // back, so the only useful thing it can do is say which devices
                // the purge has just sent back to the snapshot.
                let stranded = self.devices_left_behind(&library_id).await?;
                if stranded > 0 {
                    tracing::info!(
                        library = %library_id,
                        devices = stranded,
                        "trimming this feed sent devices back to the catalogue"
                    );
                    purged.devices_stranded += stranded;
                }
            }
        }
        Ok(purged)
    }

    /// How many devices this library's watermark has just overtaken.
    async fn devices_left_behind(&self, library_id: &str) -> Result<usize, ServiceError> {
        let stranded: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM library_event_ack a \
             JOIN library l ON l.id=a.library_id \
             WHERE a.library_id=? AND a.cursor < l.events_purged_through",
        )
        .bind(library_id)
        .fetch_one(self.db.pool())
        .await?;
        Ok(usize::try_from(stranded).unwrap_or(0))
    }

    /// Cuts one library's feed and moves its watermark with it.
    ///
    /// The delete and the watermark are one transaction, and that is the whole
    /// of what makes the expiry answer honest. Written separately, there is a
    /// window where the watermark claims less than has gone — and a client
    /// reading into it is handed a catch-up that looks complete while silently
    /// skipping the gap, which is the failure decision 4 exists to refuse.
    async fn purge_one_library(&self, library_id: &str, cutoff: i64) -> Result<u64, ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;

        // What is eligible, measured before it is gone. `changes()` after the
        // delete would give the count and not the highest cursor, and reading
        // the maximum back afterwards would read a table the delete has already
        // emptied of exactly the rows in question.
        //
        // The floor is expressed as the newest cursor that may be cut: skip the
        // `min_events` newest rows and take the one after them. With fewer rows
        // than the floor this subquery is NULL, `cursor <= NULL` is never true,
        // and the library keeps everything however old — which is the rule.
        let eligible = sqlx::query(
            "SELECT COUNT(*) AS n, MAX(cursor) AS highest FROM library_event \
             WHERE library_id=? AND changed_at < ? AND cursor <= ( \
               SELECT cursor FROM library_event WHERE library_id=? \
               ORDER BY cursor DESC LIMIT 1 OFFSET ?)",
        )
        .bind(library_id)
        .bind(cutoff)
        .bind(library_id)
        .bind(self.library_event_retention.min_events)
        .fetch_one(&mut *tx)
        .await?;
        let removed: i64 = eligible.try_get("n")?;
        let highest: Option<i64> = eligible.try_get("highest")?;
        let (Some(highest), true) = (highest, removed > 0) else {
            return Ok(0);
        };

        sqlx::query(
            "DELETE FROM library_event \
             WHERE library_id=? AND changed_at < ? AND cursor <= ( \
               SELECT cursor FROM library_event WHERE library_id=? \
               ORDER BY cursor DESC LIMIT 1 OFFSET ?)",
        )
        .bind(library_id)
        .bind(cutoff)
        .bind(library_id)
        .bind(self.library_event_retention.min_events)
        .execute(&mut *tx)
        .await?;

        // `MAX` never decreases: a pass that cut an older tail must not lower a
        // watermark an earlier one raised, and two passes racing must not
        // either.
        sqlx::query(
            "UPDATE library SET events_purged_through=MAX(events_purged_through, ?) WHERE id=?",
        )
        .bind(highest)
        .bind(library_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(u64::try_from(removed).unwrap_or(0))
    }

    /// Records how far a device has read one library's feed.
    ///
    /// RFC-007 decision 8. Two checks rather than one, and both are in the
    /// statement: the device must be this account's and unrevoked, and the
    /// account must be a member of the library. `sync_ack` needs only the
    /// first, because the journal is keyed per account and there is no second
    /// scope to escape into; here there is.
    ///
    /// `false` for anything refused — an unknown or revoked device, a library
    /// this account cannot see, a cursor beyond what the feed has written. A
    /// caller who is not a member learns nothing about whether the library
    /// exists, which is the rule everywhere else in this API.
    pub async fn acknowledge_library_events(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        device_id: Uuid,
        cursor: i64,
    ) -> Result<bool, ServiceError> {
        if cursor < 0 {
            return Ok(false);
        }
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;

        // A cursor beyond what the feed has written would let a client mark
        // itself caught up with events that do not exist yet, and then be
        // silently behind when they arrive.
        let latest: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(cursor), 0) FROM library_event WHERE library_id=?",
        )
        .bind(library_id.to_string())
        .fetch_one(&mut *tx)
        .await?;
        if cursor > latest {
            return Ok(false);
        }

        let result = sqlx::query(
            "INSERT INTO library_event_ack (library_id, device_id, cursor, acknowledged_at) \
             SELECT ?, ?, ?, ? WHERE EXISTS ( \
               SELECT 1 FROM device d \
               JOIN library_member m ON m.user_id=d.user_id \
               WHERE d.id=? AND d.user_id=? AND d.revoked_at IS NULL AND m.library_id=? \
             ) ON CONFLICT (library_id, device_id) DO UPDATE SET \
               cursor=MAX(library_event_ack.cursor, excluded.cursor), \
               acknowledged_at=excluded.acknowledged_at",
        )
        .bind(library_id.to_string())
        .bind(device_id.to_string())
        .bind(cursor)
        .bind(now_ms())
        .bind(device_id.to_string())
        .bind(user_id.to_string())
        .bind(library_id.to_string())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        // Never lowered: a client that acknowledges an older cursor after a
        // newer one has raced its own two requests, and the server is not the
        // place to decide which of them is the truth.
        Ok(result.rows_affected() == 1)
    }

    /// One page of a library's changes, for a caller entitled to that library.
    ///
    /// A caller who is not a member gets `NotFound`, not `Forbidden`: a feed
    /// that answered differently for a library that exists and one that does
    /// not would confirm the existence of another tenant's catalogue, which is
    /// the rule every other projection follows.
    pub async fn library_changes(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        after: i64,
        limit: i64,
    ) -> Result<LibraryEventPage, ServiceError> {
        if after < 0 || !(1..=MAX_SYNC_LIMIT).contains(&limit) {
            return Err(ServiceError::Invalid);
        }
        // Every read below runs on one transaction, so the watermark and the
        // events it guards come from the same snapshot.
        //
        // Two statements on the pool take two snapshots, and a retention pass
        // between them defeats the guard exactly where it matters: read the
        // watermark at 0, let a purge cut through cursor 100 and move it, then
        // read events after 5 and hand back a page starting at 101. The client
        // takes that for a successful catch-up and never learns it skipped
        // ninety-four events. Inside one snapshot the pair can only be both
        // before the purge — the events are still there — or both after it,
        // where the watermark refuses.
        //
        // The membership row is read on its own because it is what separates
        // "not a member" from "a member with nothing new". Both answers are
        // safe — an empty page and a 404 confirm equally little about a library
        // the caller cannot see — but a revoked client that only ever saw empty
        // pages would poll forever believing it was up to date.
        //
        // The events query joins `library_member` as well, and the snapshot is
        // precisely what makes that redundant: membership is fixed for the
        // duration, so the check and the join cannot disagree. It stays because
        // the rule is that tenancy lives in the query rather than in a check
        // the query trusts — which is what keeps the read safe if this
        // transaction is ever taken away again. No test covers it, and none
        // can: the case it guards is a revocation landing between two reads
        // that now share one snapshot.
        let mut tx = self.db.pool().begin().await?;
        let watermark: Option<i64> = sqlx::query_scalar(
            "SELECT l.events_purged_through FROM library l \
             JOIN library_member m ON m.library_id = l.id \
             WHERE l.id = ? AND m.user_id = ?",
        )
        .bind(library_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(watermark) = watermark else {
            return Err(ServiceError::NotFound);
        };

        // A cursor below what has been cut away has missed events. Handing back
        // the surviving tail would look like a successful catch-up while
        // silently skipping the gap, so refuse and let the client re-read the
        // catalogue instead.
        //
        // The watermark is what was purged, not what survives: a feed whose
        // oldest row sits at a high cursor has not lost anything, it simply
        // started late, and a floor derived from surviving rows cannot tell the
        // two apart.
        if after < watermark {
            return Err(ServiceError::Conflict);
        }

        let rows = sqlx::query(
            "SELECT e.cursor, e.entity_type, e.entity_id, e.action, e.payload_json, \
                    e.changed_at, e.origin_device_id \
             FROM library_event e \
             JOIN library_member m ON m.library_id = e.library_id \
             WHERE m.user_id = ? AND e.library_id = ? AND e.cursor > ? \
             ORDER BY e.cursor LIMIT ?",
        )
        .bind(user_id.to_string())
        .bind(library_id.to_string())
        .bind(after)
        .bind(limit + 1)
        .fetch_all(&mut *tx)
        .await?;
        // Nothing was written, so this only releases the snapshot; the two
        // early returns above release it the same way by dropping the
        // transaction. Closed here rather than left to the end of the function
        // because a read snapshot held open is a checkpoint the WAL cannot
        // take.
        tx.commit().await?;
        let has_more = rows.len() as i64 > limit;
        let events = rows
            .into_iter()
            .take(limit as usize)
            .map(library_event_from_row)
            .collect::<Result<Vec<_>, _>>()?;
        let next_cursor = events.last().map_or(after, |event| event.cursor);
        Ok(LibraryEventPage {
            events,
            next_cursor,
            has_more,
        })
    }
}

fn library_event_from_row(row: sqlx::sqlite::SqliteRow) -> Result<LibraryEvent, sqlx::Error> {
    Ok(LibraryEvent {
        cursor: row.try_get("cursor")?,
        entity_type: row.try_get("entity_type")?,
        entity_id: parse_uuid(row.try_get("entity_id")?)?,
        action: row.try_get("action")?,
        payload: serde_json::from_str(row.try_get::<String, _>("payload_json")?.as_str())
            .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
        changed_at: row.try_get("changed_at")?,
        origin_device_id: row
            .try_get::<Option<String>, _>("origin_device_id")?
            .map(parse_uuid)
            .transpose()?,
    })
}
