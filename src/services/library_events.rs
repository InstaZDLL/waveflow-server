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
        // Membership is read here and nowhere cached: losing access stops
        // delivery on the next request, retroactively, with no subscriber list
        // to keep in step.
        let member: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM library_member WHERE library_id=? AND user_id=?")
                .bind(library_id.to_string())
                .bind(user_id.to_string())
                .fetch_optional(self.db.pool())
                .await?;
        if member.is_none() {
            return Err(ServiceError::NotFound);
        }

        // A feed that starts above `after + 1` has dropped events this caller
        // never saw. Returning the surviving tail would look like a successful
        // catch-up while silently skipping the gap, so refuse instead and let
        // the client re-read the catalogue.
        //
        // The floor is the feed's own and not this library's, for the reason
        // the journal states about its own: `cursor` is one sequence across
        // every library, so a library's MIN only marks where it first wrote. A
        // library created on a busy instance would otherwise be told that a
        // cursor it has never advanced past had expired.
        let floor: Option<i64> = sqlx::query_scalar("SELECT MIN(cursor) FROM library_event")
            .fetch_one(self.db.pool())
            .await?;
        if floor.is_some_and(|floor| after < floor - 1) {
            return Err(ServiceError::Conflict);
        }

        let rows = sqlx::query(
            "SELECT cursor, entity_type, entity_id, action, payload_json, changed_at \
             FROM library_event WHERE library_id=? AND cursor>? ORDER BY cursor LIMIT ?",
        )
        .bind(library_id.to_string())
        .bind(after)
        .bind(limit + 1)
        .fetch_all(self.db.pool())
        .await?;
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
