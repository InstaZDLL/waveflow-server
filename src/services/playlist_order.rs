//! Moving one entry of a playlist to another position.
//!
//! The verb RFC-020 left open. Removal by position and appending were all the
//! surfaces had, so a listener who wanted a track earlier had to remove it and
//! add it back at the end, and a client that reorders had nothing to call.

use super::playlists::{playlist_track_ids_on, Reach};
use super::*;

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
        let intent = MutationIntent::new("move", &format!("playlist:{id}"), &intent_payload);
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
        if from >= shown || to >= shown {
            return Err(ServiceError::Invalid);
        }
        if expected.is_some_and(|expected| current.songs[from].id != expected) {
            return Err(ServiceError::Conflict);
        }
        let mut ids = playlist_track_ids_on(&mut tx, user_id, id).await?;
        let visible: std::collections::HashSet<Uuid> =
            current.songs.iter().map(|song| song.id).collect();
        let order = moved(&ids, &visible, from, to).ok_or(ServiceError::Invalid)?;
        ids = order;
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
        // from its whole track list, and a move is a new order of it.
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
mod tests {
    use super::*;

    fn ids(count: usize) -> Vec<Uuid> {
        (0..count).map(|_| Uuid::new_v4()).collect()
    }

    #[test]
    fn a_move_lands_where_it_was_asked_to_be_shown() {
        let tracks = ids(4);
        let visible = tracks.iter().copied().collect();
        let forward = moved(&tracks, &visible, 0, 2).unwrap();
        assert_eq!(forward, [tracks[1], tracks[2], tracks[0], tracks[3]]);
        let backward = moved(&tracks, &visible, 3, 0).unwrap();
        assert_eq!(backward, [tracks[3], tracks[0], tracks[1], tracks[2]]);
        let last = moved(&tracks, &visible, 1, 3).unwrap();
        assert_eq!(last, [tracks[0], tracks[2], tracks[3], tracks[1]]);
        assert_eq!(moved(&tracks, &visible, 2, 2).unwrap(), tracks);
        assert!(moved(&tracks, &visible, 4, 0).is_none());
        assert!(moved(&tracks, &visible, 0, 4).is_none());
    }

    /// A hidden entry keeps its place among the others, and the positions
    /// are the shown ones: moving shown 0 to shown 1 passes the hidden entry
    /// between them by, rather than landing on it.
    #[test]
    fn a_hidden_entry_neither_counts_nor_moves() {
        let tracks = ids(4);
        let hidden = tracks[1];
        let visible = tracks
            .iter()
            .copied()
            .filter(|track| *track != hidden)
            .collect();
        // Shown: 0, 2, 3.
        let order = moved(&tracks, &visible, 0, 1).unwrap();
        assert_eq!(order, [hidden, tracks[2], tracks[0], tracks[3]]);
        let to_end = moved(&tracks, &visible, 0, 2).unwrap();
        assert_eq!(to_end, [hidden, tracks[2], tracks[3], tracks[0]]);
        let to_front = moved(&tracks, &visible, 2, 0).unwrap();
        assert_eq!(to_front, [tracks[3], tracks[0], hidden, tracks[2]]);
    }

    /// The same track twice: the copy at the named position moves, and the
    /// other stays.
    #[test]
    fn a_repeat_moves_one_copy() {
        let tracks = ids(2);
        let stored = vec![tracks[0], tracks[1], tracks[0]];
        let visible = tracks.iter().copied().collect();
        let order = moved(&stored, &visible, 2, 0).unwrap();
        assert_eq!(order, [tracks[0], tracks[0], tracks[1]]);
        let order = moved(&stored, &visible, 0, 2).unwrap();
        assert_eq!(order, [tracks[1], tracks[0], tracks[0]]);
    }
}
