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

/// A patch field resolved against the correction the track already carries.
///
/// The three states of [`TrackMetadataPatch`] collapse here and nowhere else:
/// absent keeps what is stored, anything present — a value or `null` —
/// replaces it.
fn merge<T>(requested: Option<Option<T>>, stored: Option<T>) -> Option<T> {
    match requested {
        None => stored,
        Some(value) => value,
    }
}

/// Whether a patch field sets a value outside what is allowed. A field left out
/// or removed has nothing to check: what is already stored was checked when it
/// was written.
fn out_of_bounds(value: Option<Option<i64>>, allowed: impl Fn(i64) -> bool) -> bool {
    matches!(value, Some(Some(value)) if !allowed(value))
}

impl DomainServices {
    /// Corrects some of a track's tags, leaving the rest as they are.
    ///
    /// A field the patch leaves out keeps whatever correction it had, `null`
    /// removes one, and a value sets one. [`TrackMetadataPatch`] says why that
    /// is three states rather than two.
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
        origin_device_id: Option<Uuid>,
    ) -> Result<SongItem, ServiceError> {
        // Shape first, and before the gate. These are pure value checks on the
        // request: nothing about them can change under a concurrent write, so
        // refusing a malformed patch should not queue behind a scan for the
        // right to be told so.
        if out_of_bounds(patch.year, |year| (1..=9999).contains(&year))
            || out_of_bounds(patch.track_number, |number| number >= 0)
            || out_of_bounds(patch.disc_number, |number| number >= 0)
        {
            return Err(ServiceError::Invalid);
        }
        // Only what the request itself says is settled here. What it resolves
        // to depends on the stored correction, which a concurrent write can
        // change, so that is decided under the gate below. A blank string
        // removes rather than sets. A list of blanks is an empty list, and an
        // empty list survives: saying a track credits nobody is a correction
        // rather than the absence of one.
        let requested_title = patch.title.map(clean);
        let requested_sort_title = patch.sort_title.map(clean);
        let requested_musicbrainz_recording_id = patch.musicbrainz_recording_id.map(clean);
        let requested_comment = patch.comment.map(clean);
        let requested_artists = patch.artists.map(|list| list.map(clean_all));
        let requested_genres = patch.genres.map(|list| list.map(clean_all));

        // A patch that mentions no field has nothing to merge, so it writes
        // nothing and announces nothing: an `upsert` on the library feed for a
        // change that never happened would send every client to refetch the
        // track. It is still a request to correct this track, so it is refused
        // exactly as a real patch would be — read without the gate, because
        // there is no write for a revoked role to slip in front of.
        if requested_title.is_none()
            && requested_sort_title.is_none()
            && patch.year.is_none()
            && patch.track_number.is_none()
            && patch.disc_number.is_none()
            && requested_musicbrainz_recording_id.is_none()
            && requested_comment.is_none()
            && requested_artists.is_none()
            && requested_genres.is_none()
        {
            let role: Option<String> = sqlx::query_scalar(
                "SELECT m.role FROM track t \
                 JOIN library_member m ON m.library_id=t.library_id \
                 WHERE t.id=? AND m.user_id=?",
            )
            .bind(track_id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(self.db.pool())
            .await?;
            let role = crate::database::LibraryRole::from_str(&role.ok_or(ServiceError::NotFound)?)
                .map_err(|_| ServiceError::Invalid)?;
            if !role.may_write_metadata() {
                return Err(ServiceError::Forbidden);
            }
            return self
                .songs_by_ids(user_id, &[track_id])
                .await?
                .pop()
                .ok_or(ServiceError::NotFound);
        }

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
        // the dropped list corrected — the hole a single check over both lists
        // had, one field at a time.
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
                    // Only an explicit `null` drops a list. A list the patch
                    // leaves out keeps its correction, so there is nothing to
                    // restore and no reason to read the file.
                    let dropped = (hint.try_get::<i64, _>("had_artists")? != 0
                        && matches!(requested_artists, Some(None)))
                        || (hint.try_get::<i64, _>("had_genres")? != 0
                            && matches!(requested_genres, Some(None)));
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
                    ovr.title AS o_title, ovr.sort_title AS o_sort_title, \
                    ovr.year AS o_year, ovr.track_number AS o_track_number, \
                    ovr.disc_number AS o_disc_number, \
                    ovr.musicbrainz_recording_id AS o_musicbrainz_recording_id, \
                    ovr.comment AS o_comment \
             FROM track t \
             JOIN library_member m ON m.library_id=t.library_id \
             LEFT JOIN track_override ovr ON ovr.track_id=t.id \
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
        let role = crate::database::LibraryRole::from_str(row.try_get::<&str, _>("role")?)
            .map_err(|_| ServiceError::Invalid)?;
        if !role.may_write_metadata() {
            // Blurred onto 404 by the surfaces above, like every other refusal
            // that would otherwise confirm what a caller may not reach.
            return Err(ServiceError::Forbidden);
        }

        // What the track carries afterwards: every field the patch mentions,
        // over the stored correction for the rest. Merged here, under the gate
        // and in this transaction, because merging against a correction another
        // writer has since replaced would write the old one back.
        //
        // The lists through the helper the scan reads them with, which refuses
        // a stored list it cannot decode instead of reading it as none. Read as
        // none, a patch that never mentioned it would erase it by writing the
        // merge back.
        let stored_lists = crate::catalog::track_override_lists(&mut tx, track_id).await?;
        let title = merge(requested_title, row.try_get("o_title")?);
        let sort_title = merge(requested_sort_title, row.try_get("o_sort_title")?);
        let year = merge(patch.year, row.try_get("o_year")?);
        let track_number = merge(patch.track_number, row.try_get("o_track_number")?);
        let disc_number = merge(patch.disc_number, row.try_get("o_disc_number")?);
        let musicbrainz_recording_id = merge(
            requested_musicbrainz_recording_id,
            row.try_get("o_musicbrainz_recording_id")?,
        );
        let comment = merge(requested_comment, row.try_get("o_comment")?);
        let artists = merge(requested_artists.clone(), stored_lists.artists.clone());
        let genres = merge(requested_genres.clone(), stored_lists.genres.clone());
        // The authority, per field like the hint above.
        let dropped_a_list = (stored_lists.artists.is_some() && artists.is_none())
            || (stored_lists.genres.is_some() && genres.is_none());
        let empty = title.is_none()
            && sort_title.is_none()
            && year.is_none()
            && track_number.is_none()
            && disc_number.is_none()
            && musicbrainz_recording_id.is_none()
            && comment.is_none()
            && artists.is_none()
            && genres.is_none();
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
            .bind(year)
            .bind(track_number)
            .bind(disc_number)
            .bind(musicbrainz_recording_id.as_deref())
            .bind(comment.as_deref())
            .bind(encode_list(artists.as_deref())?)
            .bind(encode_list(genres.as_deref())?)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }

        // A correction being removed hands the track back to its file, and the
        // way to be sure the result is what a scan would have written is to run
        // the scan's own apply — with the correction already deleted above, so
        // it derives from the tags rather than from what it is undoing. In the
        // same transaction, so the removal and the restoration cannot come
        // apart. A list correction the patch kept is still in the row written
        // above, and the apply reads it back over the file.
        if dropped_a_list {
            let Some(input) = restored else {
                // The hint and the transaction disagreed, which means the
                // correction appeared between the two reads. Refusing costs the
                // caller a retry; guessing would cost the track its credits.
                return Err(ServiceError::Unavailable);
            };
            // Carried through rather than required. A track that never came
            // from a scan has none, and re-deriving its tags does not give it
            // one: this call is not a scan either. Demanding one here would
            // have refused every received file until a scan happened to walk
            // past it.
            let scan_id = last_seen_scan_id.map(parse_uuid).transpose()?;
            crate::database::Database::apply_catalog_track_in_transaction(
                &mut tx,
                self.db.pid(),
                library_id,
                scan_id,
                &crate::catalog::CatalogApply {
                    input,
                    existing_id: Some(track_id),
                    moved: false,
                    origin_device_id,
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

        // The rows a list implies, written through the same helper the scan
        // consults — so a rescan derives what this call just wrote rather than
        // something merely similar. Only for a list this patch set: one it left
        // out already has its rows, and rewriting them identically would still
        // delete and reinsert every credit.
        let effective = crate::catalog::TrackOverrideLists {
            title: title.clone(),
            artists: requested_artists.flatten(),
            genres: requested_genres.flatten(),
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
            crate::catalog::LibraryChange {
                library_id,
                entity_type: "track",
                entity_id: track_id,
                action: "upsert",
                payload: serde_json::json!({ "full_hash": full_hash }),
                changed_at: now,
                origin_device_id,
            },
        )
        .await?;
        tx.commit().await?;
        drop(_writer);

        self.songs_by_ids(user_id, &[track_id])
            .await?
            .pop()
            .ok_or(ServiceError::NotFound)
    }

    /// What the file says and what the correction says, side by side.
    ///
    /// Read in one snapshot: a page that showed a source from before a write
    /// and an override from after it would describe a track that never
    /// existed. Tenancy is in the join, so a track in a library the caller is
    /// not a member of is missing rather than forbidden.
    ///
    /// `LEFT JOIN`, because most tracks carry no correction at all and an
    /// absent row is an answer — every field `null` — rather than a 404.
    pub async fn track_overrides(
        &self,
        user_id: Uuid,
        track_id: Uuid,
    ) -> Result<TrackOverrides, ServiceError> {
        let row = sqlx::query(
            "SELECT t.title, t.sort_title, t.year, t.track_number, t.disc_number,                     t.musicbrainz_recording_id, t.comment,                     ovr.title AS o_title, ovr.sort_title AS o_sort_title,                     ovr.year AS o_year, ovr.track_number AS o_track_number,                     ovr.disc_number AS o_disc_number,                     ovr.musicbrainz_recording_id AS o_musicbrainz_recording_id,                     ovr.comment AS o_comment, ovr.artists AS o_artists,                     ovr.genres AS o_genres              FROM track t              JOIN library_member m ON m.library_id=t.library_id              LEFT JOIN track_override ovr ON ovr.track_id=t.id              WHERE t.id=? AND m.user_id=?",
        )
        .bind(track_id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;

        // Stored as JSON rather than the `;`-joined form the tag columns use,
        // because an override is a list someone typed on purpose. A row that
        // will not parse is reported as no correction rather than taking the
        // request down: the editor then shows the file's credits, which is the
        // safe reading of a value nobody can interpret.
        let list = |column: &str| -> Option<Vec<String>> {
            row.try_get::<Option<String>, _>(column)
                .ok()
                .flatten()
                .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        };

        Ok(TrackOverrides {
            source: TrackSourceTags {
                title: row.try_get("title")?,
                sort_title: row.try_get("sort_title")?,
                year: row.try_get("year")?,
                track_number: row.try_get("track_number")?,
                disc_number: row.try_get("disc_number")?,
                musicbrainz_recording_id: row.try_get("musicbrainz_recording_id")?,
                comment: row.try_get("comment")?,
            },
            overrides: TrackOverrideValues {
                title: row.try_get("o_title")?,
                sort_title: row.try_get("o_sort_title")?,
                year: row.try_get("o_year")?,
                track_number: row.try_get("o_track_number")?,
                disc_number: row.try_get("o_disc_number")?,
                musicbrainz_recording_id: row.try_get("o_musicbrainz_recording_id")?,
                comment: row.try_get("o_comment")?,
                artists: list("o_artists"),
                genres: list("o_genres"),
            },
        })
    }
}
