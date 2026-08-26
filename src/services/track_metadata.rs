//! Correcting a track's tags without rewriting its file.

use super::*;

/// Every name trimmed and the blanks dropped. The list itself survives being
/// emptied: "this track credits nobody" is a correction, not its absence.
fn clean_all(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .collect()
}

/// Stored as JSON, because it arrived already separated. See the migration for
/// why re-joining it into the `;`-delimited form the tag columns use would give
/// back the ambiguity the correction was made to settle.
fn encode_list(values: Option<&[String]>) -> Result<Option<String>, ServiceError> {
    values
        .map(|values| serde_json::to_string(values).map_err(|_| ServiceError::Invalid))
        .transpose()
}

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
        // Shape first, and before the gate. These are pure value checks on the
        // request: nothing about them can change under a concurrent write, so
        // refusing a malformed patch should not queue behind a scan for the
        // right to be told so.
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
        // A list of blanks is no list. An empty list survives, because saying
        // a track credits nobody is a correction rather than the absence of one.
        let lists = crate::catalog::TrackOverrideLists {
            title: title.clone(),
            artists: patch.artists.map(clean_all),
            genres: patch.genres.map(clean_all),
        };
        let empty = title.is_none()
            && sort_title.is_none()
            && musicbrainz_recording_id.is_none()
            && comment.is_none()
            && patch.year.is_none()
            && patch.track_number.is_none()
            && patch.disc_number.is_none()
            && lists.artists.is_none()
            && lists.genres.is_none();

        // Removing a list correction needs the file, because the rows it wrote
        // replaced what the tags said and the catalogue no longer holds the
        // original. Read here, before the gate: file I/O has no business
        // happening while the process-wide writer gate is held, and a file that
        // cannot be read has to refuse the whole call rather than leave a
        // correction removed and its rows behind.
        //
        // This read is a hint about whether to bother — the transaction below
        // is the authority, and disagreeing with it is a race that costs a
        // retry rather than a wrong answer.
        // Per field, not for the pair. A patch that keeps one correction and
        // drops the other still drops one, and treating the two together left
        // the dropped list corrected — the same hole the wholesale case had,
        // one field at a time.
        let restored = {
            let hint = sqlx::query(
                "SELECT t.relative_path, l.root_path, m.role, \
                        ovr.artists IS NOT NULL AS had_artists, \
                        ovr.genres IS NOT NULL AS had_genres \
                 FROM track t \
                 JOIN library l ON l.id=t.library_id \
                 JOIN library_member m ON m.library_id=t.library_id \
                 JOIN track_override ovr ON ovr.track_id=t.id \
                 WHERE t.id=? AND m.user_id=?",
            )
            .bind(track_id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(self.db.pool())
            .await?;
            match hint {
                None => None,
                Some(hint) => {
                    // Refused here as well as under the gate below. This one is
                    // not the authority and does not need to be: it exists so a
                    // caller who may not write cannot make the server read a
                    // file and hash it end to end before being told no.
                    let role =
                        crate::database::LibraryRole::from_str(hint.try_get::<&str, _>("role")?)
                            .map_err(|_| ServiceError::Invalid)?;
                    let dropped = (hint.try_get::<i64, _>("had_artists")? != 0
                        && lists.artists.is_none())
                        || (hint.try_get::<i64, _>("had_genres")? != 0 && lists.genres.is_none());
                    if !role.may_write_metadata() || !dropped {
                        None
                    } else {
                        Some(
                            self.scanner
                                .read_track_input(
                                    std::path::Path::new(&hint.try_get::<String, _>("root_path")?),
                                    hint.try_get::<&str, _>("relative_path")?,
                                )
                                .await
                                .map_err(|error| {
                                    tracing::warn!(
                                        %error,
                                        track = %track_id,
                                        "cannot re-read a track whose correction is being removed"
                                    );
                                    ServiceError::Unavailable
                                })?,
                        )
                    }
                }
            }
        };

        // Authorization is different, and is read under the gate rather than
        // before it. Membership and role are mutable state, and the gate is
        // what serialises writers — so a role revoked or downgraded while this
        // call was deciding cannot commit between the check and the write.
        // Read inside the transaction as well, so the pair sees one snapshot.
        let _writer = self.db.writer_guard().await;
        let now = now_ms();
        let mut tx = self.db.pool().begin().await?;
        let row = sqlx::query(
            "SELECT t.library_id, t.title, t.full_hash, t.last_seen_scan_id, m.role, \
                    (SELECT ovr.artists IS NOT NULL FROM track_override ovr \
                       WHERE ovr.track_id=t.id) AS had_artists, \
                    (SELECT ovr.genres IS NOT NULL FROM track_override ovr \
                       WHERE ovr.track_id=t.id) AS had_genres \
             FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             WHERE t.id=? AND m.user_id=?",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ServiceError::NotFound)?;
        let library_id = parse_uuid(row.try_get("library_id")?)?;
        let scanned_title: String = row.try_get("title")?;
        let full_hash: String = row.try_get("full_hash")?;
        let last_seen_scan_id: Option<String> = row.try_get("last_seen_scan_id")?;
        // The authority, and it decides per field like the hint above.
        let dropped_a_list = (row.try_get::<Option<i64>, _>("had_artists")?.unwrap_or(0) != 0
            && lists.artists.is_none())
            || (row.try_get::<Option<i64>, _>("had_genres")?.unwrap_or(0) != 0
                && lists.genres.is_none());
        let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_write_metadata() {
            // Blurred onto 404 by the surfaces above, like every other refusal
            // that would otherwise confirm what a caller may not reach.
            return Err(ServiceError::Forbidden);
        }
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
                   track_number, disc_number, musicbrainz_recording_id, comment, artists, \
                   genres, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
                 ON CONFLICT (track_id) DO UPDATE SET title=excluded.title, \
                   sort_title=excluded.sort_title, year=excluded.year, \
                   track_number=excluded.track_number, disc_number=excluded.disc_number, \
                   musicbrainz_recording_id=excluded.musicbrainz_recording_id, \
                   comment=excluded.comment, artists=excluded.artists, \
                   genres=excluded.genres, updated_at=excluded.updated_at",
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
            .bind(encode_list(lists.artists.as_deref())?)
            .bind(encode_list(lists.genres.as_deref())?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        // A correction being removed hands the track back to its file, and the
        // way to be sure the result is what a scan would have written is to run
        // the scan's own apply — with the correction already deleted above, so
        // it derives from the tags rather than from what it is undoing. In the
        // same transaction, so the removal and the restoration cannot come
        // apart.
        if dropped_a_list {
            let Some(input) = restored else {
                // The hint and the transaction disagreed, which means the
                // correction appeared between the two reads. Refusing costs the
                // caller a retry; guessing would cost the track its credits.
                return Err(ServiceError::Unavailable);
            };
            let scan_id = last_seen_scan_id
                .map(parse_uuid)
                .transpose()?
                .ok_or(ServiceError::Unavailable)?;
            crate::database::Database::apply_catalog_track_in_transaction(
                &mut tx,
                self.db.pid(),
                library_id,
                scan_id,
                &crate::catalog::CatalogApply {
                    input,
                    existing_id: Some(track_id),
                    moved: false,
                },
                now,
            )
            .await?;
            tx.commit().await?;
            drop(_writer);
            return self
                .songs_by_ids(user_id, &[track_id])
                .await?
                .pop()
                .ok_or(ServiceError::NotFound);
        }

        // The rows an explicit list implies, written through the same helper the
        // scan consults — so a rescan derives what this call just wrote rather
        // than something merely similar.
        let effective = if empty {
            crate::catalog::TrackOverrideLists::default()
        } else {
            lists.clone()
        };
        crate::catalog::apply_track_override_lists(
            &mut tx,
            self.db.pid(),
            library_id,
            track_id,
            &effective,
            now,
        )
        .await?;
        if let Some(names) = &effective.artists {
            sqlx::query("UPDATE track SET artist_display=?, updated_at=? WHERE id=?")
                .bind(names.join("; "))
                .bind(now)
                .bind(track_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(names) = &effective.genres {
            sqlx::query("UPDATE track SET genre_display=?, updated_at=? WHERE id=?")
                .bind(names.join("; "))
                .bind(now)
                .bind(track_id.to_string())
                .execute(&mut *tx)
                .await?;
        }

        // The index holds a copy of the title, of every credited name and of
        // the genres, all rebuilt by each scan from the file. Leaving them
        // behind would have a corrected track keep answering to what it was
        // corrected away from — corrected to the eye and nowhere else.
        let credited: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT ar.name FROM track_participant tp \
             JOIN artist ar ON ar.id=tp.artist_id WHERE tp.track_id=? ORDER BY ar.name",
        )
        .bind(track_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE track_fts SET title=?, artists=?, \
             genres=(SELECT genre_display FROM track WHERE id=?) WHERE track_id=?",
        )
        .bind(title.as_deref().unwrap_or(scanned_title.as_str()))
        .bind(credited.join(" "))
        .bind(track_id.to_string())
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
