//! Correcting a track's tags without rewriting its file.

use super::*;

/// Trimmed, with blank read as no correction at all.
fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

impl DomainServices {
    /// Replaces the corrections carried by one track.
    ///
    /// The file is never touched. `full_hash` therefore cannot move, which is
    /// what keeps a client's content-based link valid across an edit — the one
    /// thing rewriting tags into the file would have cost.
    ///
    /// The scanner neither reads nor writes `track_override`, so surviving a
    /// rescan is a property of where the correction lives rather than of
    /// anything remembering to preserve it.
    pub async fn set_track_metadata(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        patch: TrackMetadataPatch,
    ) -> Result<SongItem, ServiceError> {
        // Tenancy and role in one read, filtered by membership like every other
        // projection. A track in a library the caller cannot see is not there.
        let row = sqlx::query(
            "SELECT t.library_id, t.title, t.full_hash, m.role FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE t.id=? AND m.user_id=?",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        let library_id = parse_uuid(row.try_get("library_id")?)?;
        let scanned_title: String = row.try_get("title")?;
        let full_hash: String = row.try_get("full_hash")?;
        let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_write_metadata() {
            // Blurred onto 404 by the surfaces above, like every other refusal
            // that would otherwise confirm what a caller may not reach.
            return Err(ServiceError::Forbidden);
        }

        let title = clean(patch.title);
        let sort_title = clean(patch.sort_title);
        let musicbrainz_recording_id = clean(patch.musicbrainz_recording_id);
        let comment = clean(patch.comment);
        if patch.year.is_some_and(|year| !(1..=9999).contains(&year))
            || patch.track_number.is_some_and(|number| number < 0)
            || patch.disc_number.is_some_and(|number| number < 0)
        {
            return Err(ServiceError::Invalid);
        }
        let empty = title.is_none()
            && sort_title.is_none()
            && musicbrainz_recording_id.is_none()
            && comment.is_none()
            && patch.year.is_none()
            && patch.track_number.is_none()
            && patch.disc_number.is_none();

        let _writer = self.db.writer_guard().await;
        let now = now_ms();
        let mut tx = self.db.pool().begin().await?;
        if empty {
            // No corrections left is no row: an override that holds nothing but
            // NULLs would answer the same as its absence while still claiming
            // the track carries one.
            sqlx::query("DELETE FROM track_override WHERE track_id=?")
                .bind(track_id.to_string())
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query(
                "INSERT INTO track_override (track_id, library_id, title, sort_title, year, \
                   track_number, disc_number, musicbrainz_recording_id, comment, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (track_id) DO UPDATE SET title=excluded.title, \
                   sort_title=excluded.sort_title, year=excluded.year, \
                   track_number=excluded.track_number, disc_number=excluded.disc_number, \
                   musicbrainz_recording_id=excluded.musicbrainz_recording_id, \
                   comment=excluded.comment, updated_at=excluded.updated_at",
            )
            .bind(track_id.to_string())
            .bind(library_id.to_string())
            .bind(title.as_deref())
            .bind(sort_title.as_deref())
            .bind(patch.year)
            .bind(patch.track_number)
            .bind(patch.disc_number)
            .bind(musicbrainz_recording_id.as_deref())
            .bind(comment.as_deref())
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        // The index holds a copy of the title, rebuilt by every scan from the
        // file. Leaving it behind would have a corrected track keep answering
        // to the name it was corrected away from.
        sqlx::query("UPDATE track_fts SET title=? WHERE track_id=?")
            .bind(title.as_deref().unwrap_or(scanned_title.as_str()))
            .bind(track_id.to_string())
            .execute(&mut *tx)
            .await?;

        // Announced on the library feed, not the user journal: a correction
        // belongs to the library and every member sees it. The hash travels
        // with it unchanged, which is the client's evidence that its link
        // survived the edit.
        crate::catalog::record_library_event(
            &mut tx,
            library_id,
            "track",
            track_id,
            "upsert",
            &serde_json::json!({ "full_hash": full_hash }),
            now,
        )
        .await?;
        tx.commit().await?;
        drop(_writer);

        self.songs_by_ids(user_id, &[track_id])
            .await?
            .pop()
            .ok_or(ServiceError::NotFound)
    }
}
