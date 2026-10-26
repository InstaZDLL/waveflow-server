//! Moving and removing one entry of a playlist, by the position shown.
//!
//! The verb RFC-020 left open. Removal by position and appending were all the
//! surfaces had, so a listener who wanted a track earlier had to remove it and
//! add it back at the end, and a client that reorders had nothing to call.

use super::playlists::{playlist_track_ids_on, Reach};
use super::*;

/// One edit of one entry, named by the position the caller was shown.
#[derive(Debug, Clone, Copy)]
enum EntryEdit {
    /// To be shown at this position once moved.
    MoveTo(usize),
    Remove,
}

impl DomainServices {
    /// Moves the entry shown at `from` so that it is shown at `to`.
    ///
    /// **Positions are the ones the caller was shown**, as for removal
    /// (RFC-020 decision 1): the stored list without the tracks this account
    /// cannot read. Those hidden entries keep their place among the others,
    /// for when their track comes back; only the moved entry changes place.
    ///
    /// **`expected` guards a stale view.** With repeats allowed, a position is
    /// the only way to name an entry, and a position read before somebody
    /// else's edit can name a different one by the time it arrives. A caller
    /// that sends the track it believes sits at `from` is refused with a
    /// conflict rather than having the wrong entry moved.
    ///
    /// The owner's playlists only: another account's public playlist answers
    /// as one that does not exist.
    pub async fn move_playlist_entry_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        from: usize,
        to: usize,
        expected: Option<Uuid>,
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let mut intent_payload = serde_json::json!({ "from": from, "to": to });
        if let Some(expected) = expected {
            intent_payload["track_id"] = serde_json::json!(expected);
        }
        self.edit_entry(
            user_id,
            id,
            from,
            expected,
            EntryEdit::MoveTo(to),
            MutationIntent::new("move", &format!("playlist:{id}"), &intent_payload),
            context,
        )
        .await
    }

    /// Removes the entry shown at `position`, guarded like a move: when
    /// `expected` is not the track shown there, the removal is refused with a
    /// conflict rather than taking another entry of a list edited elsewhere.
    /// Hidden entries stay, for when their track comes back.
    pub async fn remove_playlist_entry_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        position: usize,
        expected: Option<Uuid>,
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let mut intent_payload = serde_json::json!({ "position": position });
        if let Some(expected) = expected {
            intent_payload["track_id"] = serde_json::json!(expected);
        }
        self.edit_entry(
            user_id,
            id,
            position,
            expected,
            EntryEdit::Remove,
            MutationIntent::new("remove-entry", &format!("playlist:{id}"), &intent_payload),
            context,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn edit_entry(
        &self,
        user_id: Uuid,
        id: Uuid,
        at: usize,
        expected: Option<Uuid>,
        edit: EntryEdit,
        intent: MutationIntent,
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        let current = self.playlist_on(&mut tx, user_id, id, Reach::Owned).await?;
        let shown = current.songs.len();
        let within = match edit {
            EntryEdit::MoveTo(to) => at < shown && to < shown,
            EntryEdit::Remove => at < shown,
        };
        if !within {
            return Err(ServiceError::Invalid);
        }
        if expected.is_some_and(|expected| current.songs[at].id != expected) {
            return Err(ServiceError::Conflict);
        }
        let stored = playlist_track_ids_on(&mut tx, user_id, id).await?;
        let visible: std::collections::HashSet<Uuid> =
            current.songs.iter().map(|song| song.id).collect();
        let ids = match edit {
            EntryEdit::MoveTo(to) => moved(&stored, &visible, at, to),
            EntryEdit::Remove => removed(&stored, &visible, at),
        }
        .ok_or(ServiceError::Invalid)?;
        let changed_at = now_ms();
        sqlx::query("UPDATE playlist SET updated_at=? WHERE id=? AND owner_user_id=?")
            .bind(changed_at)
            .bind(id.to_string())
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM playlist_track WHERE playlist_id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        replace_playlist_tracks(&mut tx, id, &ids, changed_at).await?;
        // The same payload an update journals: a device mirrors a playlist
        // from its whole track list, and an edit is a new state of it.
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": current.name,
                    "comment": current.comment,
                    "public": current.public,
                    "track_ids": ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }
}

/// The stored list without the entry shown at `position`, every hidden entry
/// kept. `None` when the position is not one of the shown entries.
fn removed(
    stored: &[Uuid],
    visible: &std::collections::HashSet<Uuid>,
    position: usize,
) -> Option<Vec<Uuid>> {
    let at = stored
        .iter()
        .enumerate()
        .filter(|(_, track)| visible.contains(track))
        .map(|(index, _)| index)
        .nth(position)?;
    let mut ids = stored.to_vec();
    ids.remove(at);
    Some(ids)
}

/// The stored list with the entry shown at `from` moved so that it is shown
/// at `to`, every hidden entry left where it was among the others.
///
/// `None` when either position is not one of the shown entries.
fn moved(
    stored: &[Uuid],
    visible: &std::collections::HashSet<Uuid>,
    from: usize,
    to: usize,
) -> Option<Vec<Uuid>> {
    let shown: Vec<usize> = stored
        .iter()
        .enumerate()
        .filter(|(_, track)| visible.contains(track))
        .map(|(position, _)| position)
        .collect();
    let source = *shown.get(from)?;
    shown.get(to)?;
    let mut ids = stored.to_vec();
    let entry = ids.remove(source);
    // The shown positions once the entry is out: everything after it moved
    // up by one.
    let remaining: Vec<usize> = shown
        .iter()
        .filter(|position| **position != source)
        .map(|position| {
            if *position > source {
                position - 1
            } else {
                *position
            }
        })
        .collect();
    // Before the entry now shown at `to`, or after the last shown one when it
    // goes to the end.
    let at = match remaining.get(to) {
        Some(position) => *position,
        None => remaining.last().map_or(source, |last| last + 1),
    };
    ids.insert(at, entry);
    Some(ids)
}

#[cfg(test)]
mod tests;
