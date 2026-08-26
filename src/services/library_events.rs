//! Reading a library's change feed.
//!
//! The counterpart of the RFC-003 journal for the half of the server that had
//! no feed at all. Written by scans in `catalog.rs`; read here.

use super::*;
use crate::sync::MAX_SYNC_LIMIT;

impl DomainServices {
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
            "SELECT e.cursor, e.entity_type, e.entity_id, e.action, e.payload_json, e.changed_at \
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
    })
}
