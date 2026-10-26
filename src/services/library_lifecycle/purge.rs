//! Purging missing tracks and restoring them, and journaling what a removal
//! took with it.

use super::*;

/// Deletes the favourites and ratings of the albums and artists among
/// `albums` and `artists` (JSON arrays of ids) that no longer exist, and
/// answers them, so the accounts that held them can be told.
async fn forget_vanished_entities(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    albums: &str,
    artists: &str,
    removed: &mut Removed,
) -> Result<(), sqlx::Error> {
    removed.stars.extend(
        sqlx::query_as::<_, (String, String, String)>(
            "DELETE FROM user_star WHERE entity_type = 'album' \
               AND entity_id IN (SELECT value FROM json_each(?)) \
               AND NOT EXISTS (SELECT 1 FROM album WHERE album.id = user_star.entity_id) \
             RETURNING user_id, entity_type, entity_id",
        )
        .bind(albums)
        .fetch_all(&mut **tx)
        .await?,
    );
    removed.ratings.extend(
        sqlx::query_as::<_, (String, String, String)>(
            "DELETE FROM user_rating WHERE entity_type = 'album' \
               AND entity_id IN (SELECT value FROM json_each(?)) \
               AND NOT EXISTS (SELECT 1 FROM album WHERE album.id = user_rating.entity_id) \
             RETURNING user_id, entity_type, entity_id",
        )
        .bind(albums)
        .fetch_all(&mut **tx)
        .await?,
    );
    removed.stars.extend(
        sqlx::query_as::<_, (String, String, String)>(
            "DELETE FROM user_star WHERE entity_type = 'artist' \
               AND entity_id IN (SELECT value FROM json_each(?)) \
               AND NOT EXISTS (SELECT 1 FROM artist WHERE artist.id = user_star.entity_id) \
             RETURNING user_id, entity_type, entity_id",
        )
        .bind(artists)
        .fetch_all(&mut **tx)
        .await?,
    );
    removed.ratings.extend(
        sqlx::query_as::<_, (String, String, String)>(
            "DELETE FROM user_rating WHERE entity_type = 'artist' \
               AND entity_id IN (SELECT value FROM json_each(?)) \
               AND NOT EXISTS (SELECT 1 FROM artist WHERE artist.id = user_rating.entity_id) \
             RETURNING user_id, entity_type, entity_id",
        )
        .bind(artists)
        .fetch_all(&mut **tx)
        .await?,
    );
    Ok(())
}

/// What a removal took from accounts, so their sync journals can say so
/// (RFC-021, what was still open): each `(account, entity type, entity id)`
/// favourite and rating that went, and the playlists that lost an entry.
#[derive(Default)]
pub(super) struct Removed {
    pub(super) stars: Vec<(String, String, String)>,
    pub(super) ratings: Vec<(String, String, String)>,
    pub(super) playlists: Vec<String>,
}

/// The playlists holding any of `tracks` (a JSON array of ids), read before
/// the tracks go and take their entries with them.
async fn playlists_holding(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    tracks: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT DISTINCT playlist_id FROM playlist_track \
         WHERE track_id IN (SELECT value FROM json_each(?))",
    )
    .bind(tracks)
    .fetch_all(&mut **tx)
    .await
}

/// What one purge removed, and the canvases it may have left with no link:
/// those are released after the commit, under their own locks, as removing a
/// canvas releases them.
#[derive(Default)]
pub(super) struct Purged {
    pub(super) tracks: i64,
    pub(super) canvases: Vec<String>,
    pub(super) removed: Removed,
}

/// The purge itself, in the caller's transaction under the writer gate it
/// holds: the tracks that are **still** unavailable among those selected, what
/// cascades from them, their favourites and search rows, then the scan's own
/// consolidation, so no album or artist is left holding nothing.
pub(super) async fn purge_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    library_id: Uuid,
    selection: &Selection,
) -> Result<Purged, sqlx::Error> {
    let library = library_id.to_string();
    let targets: Vec<(String, Option<String>)> = match selection {
        Selection::All => {
            sqlx::query_as(
                "SELECT id, album_id FROM track WHERE library_id = ? AND is_available = 0",
            )
            .bind(&library)
            .fetch_all(&mut **tx)
            .await?
        }
        Selection::Named(ids) => {
            sqlx::query_as(
                "SELECT id, album_id FROM track WHERE library_id = ? AND is_available = 0 \
                   AND id IN (SELECT value FROM json_each(?))",
            )
            .bind(&library)
            .bind(ids)
            .fetch_all(&mut **tx)
            .await?
        }
        Selection::OlderThan(cutoff) => {
            sqlx::query_as(
                "SELECT id, album_id FROM track WHERE library_id = ? AND is_available = 0 \
                   AND COALESCE(unavailable_since, updated_at) < ?",
            )
            .bind(&library)
            .bind(cutoff)
            .fetch_all(&mut **tx)
            .await?
        }
        Selection::PurgedBy(cutoff) => {
            sqlx::query_as(
                "SELECT id, album_id FROM track WHERE library_id = ? AND is_available = 0 \
                   AND purged_at IS NOT NULL AND purged_at <= ?",
            )
            .bind(&library)
            .bind(cutoff)
            .fetch_all(&mut **tx)
            .await?
        }
    };
    if targets.is_empty() {
        return Ok(Purged::default());
    }
    let tracks = serde_json::to_string(
        &targets
            .iter()
            .map(|(id, _)| id.as_str())
            .collect::<Vec<_>>(),
    )
    .expect("a list of strings serializes");
    // What may be left holding nothing once these go, read before they do.
    let albums: Vec<String> = targets
        .iter()
        .filter_map(|(_, album)| album.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut artists: std::collections::BTreeSet<String> = sqlx::query_scalar(
        "SELECT DISTINCT artist_id FROM track_participant \
         WHERE track_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&tracks)
    .fetch_all(&mut **tx)
    .await?
    .into_iter()
    .collect();
    let albums = serde_json::to_string(&albums).expect("a list of strings serializes");
    artists.extend(
        sqlx::query_scalar::<_, String>(
            "SELECT album_artist_id FROM album WHERE album_artist_id IS NOT NULL \
               AND id IN (SELECT value FROM json_each(?))",
        )
        .bind(&albums)
        .fetch_all(&mut **tx)
        .await?,
    );
    let artists = serde_json::to_string(&artists).expect("a list of strings serializes");
    // The loops these tracks carry: their links cascade, the blobs do not.
    let canvases: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT canvas_hash FROM track_canvas \
         WHERE track_id IN (SELECT value FROM json_each(?))",
    )
    .bind(&tracks)
    .fetch_all(&mut **tx)
    .await?;
    let mut removed = Removed {
        playlists: playlists_holding(tx, &tracks).await?,
        ..Removed::default()
    };

    // Neither has a foreign key to cascade through: an entity id names a
    // track, an album or an artist, whichever the type says. Answered, so the
    // accounts that held them are told through their journals.
    removed.stars = sqlx::query_as(
        "DELETE FROM user_star WHERE entity_type = 'track' \
           AND entity_id IN (SELECT value FROM json_each(?)) \
         RETURNING user_id, entity_type, entity_id",
    )
    .bind(&tracks)
    .fetch_all(&mut **tx)
    .await?;
    removed.ratings = sqlx::query_as(
        "DELETE FROM user_rating WHERE entity_type = 'track' \
           AND entity_id IN (SELECT value FROM json_each(?)) \
         RETURNING user_id, entity_type, entity_id",
    )
    .bind(&tracks)
    .fetch_all(&mut **tx)
    .await?;
    // A virtual table: nothing cascades into it.
    sqlx::query("DELETE FROM track_fts WHERE track_id IN (SELECT value FROM json_each(?))")
        .bind(&tracks)
        .execute(&mut **tx)
        .await?;
    // `is_available = 0` again, in the statement that deletes: the selection
    // above was read in this transaction under the writer gate, so nothing can
    // have brought a track back since, and this says so where it matters.
    let purged = sqlx::query(
        "DELETE FROM track WHERE library_id = ? AND is_available = 0 \
           AND id IN (SELECT value FROM json_each(?))",
    )
    .bind(&library)
    .bind(&tracks)
    .execute(&mut **tx)
    .await?
    .rows_affected() as i64;
    // The scan's own consolidation: the album and artist sweeps, the credits
    // and the counts, the artist search index — over what is left.
    Database::consolidate_sort_names_in(tx, library_id).await?;
    Database::consolidate_catalog_derivations_in(tx, library_id).await?;
    forget_vanished_entities(tx, &albums, &artists, &mut removed).await?;
    sqlx::query(
        "DELETE FROM genre WHERE library_id = ? \
           AND NOT EXISTS (SELECT 1 FROM track_genre tg WHERE tg.genre_id = genre.id)",
    )
    .bind(&library)
    .execute(&mut **tx)
    .await?;
    Ok(Purged {
        tracks: purged,
        canvases,
        removed,
    })
}

/// Takes the selected missing tracks out of sight without deleting anything:
/// the first half of a purge when the server keeps a grace window. Only those
/// still unavailable and not already purged. Answers how many.
pub(super) async fn hide_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    library_id: Uuid,
    selection: &Selection,
    now: i64,
) -> Result<i64, sqlx::Error> {
    let library = library_id.to_string();
    let hidden = match selection {
        Selection::All => {
            sqlx::query(
                "UPDATE track SET purged_at = ? \
                 WHERE library_id = ? AND is_available = 0 AND purged_at IS NULL",
            )
            .bind(now)
            .bind(&library)
            .execute(&mut **tx)
            .await?
        }
        Selection::Named(ids) => {
            sqlx::query(
                "UPDATE track SET purged_at = ? \
                 WHERE library_id = ? AND is_available = 0 AND purged_at IS NULL \
                   AND id IN (SELECT value FROM json_each(?))",
            )
            .bind(now)
            .bind(&library)
            .bind(ids)
            .execute(&mut **tx)
            .await?
        }
        Selection::OlderThan(cutoff) => {
            sqlx::query(
                "UPDATE track SET purged_at = ? \
                 WHERE library_id = ? AND is_available = 0 AND purged_at IS NULL \
                   AND COALESCE(unavailable_since, updated_at) < ?",
            )
            .bind(now)
            .bind(&library)
            .bind(cutoff)
            .execute(&mut **tx)
            .await?
        }
        Selection::PurgedBy(_) => return Ok(0),
    };
    Ok(hidden.rows_affected() as i64)
}

impl DomainServices {
    /// Purges missing tracks, explicitly (RFC-021 decision 4).
    ///
    /// Only those still unavailable when it runs: a track that came back
    /// between the listing and the request is kept. One transaction under the
    /// writer gate, refused while the library is being scanned — a running
    /// scan may be about to relocate one of these onto a file it just found.
    pub async fn purge_unavailable_tracks(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        request: PurgeRequest,
    ) -> Result<PurgeOutcome, ServiceError> {
        let selection = match (request.all, request.track_ids) {
            (true, None) => Selection::All,
            (false, Some(ids)) if !ids.is_empty() => Selection::Named(
                serde_json::to_string(&ids.iter().map(|id| id.to_string()).collect::<Vec<_>>())
                    .expect("a list of strings serializes"),
            ),
            _ => return Err(ServiceError::Invalid),
        };
        // Standing before the scan lock: a `409` for a library the caller may
        // not see would say that it exists, and that it is being scanned. Read
        // again in the transaction below, for a membership revoked meanwhile.
        {
            let mut connection = self.db.pool().acquire().await?;
            if !standing_on(&mut connection, actor, library_id)
                .await?
                .curates()
            {
                return Err(ServiceError::Forbidden);
            }
        }
        let _scan_lock = self
            .scanner
            .try_lock_library(library_id)
            .ok_or(ServiceError::Conflict)?;
        // With a grace window, the purge only takes the tracks out of sight,
        // and the finishing pass deletes them once it has passed (RFC-021,
        // undoing a purge). Without one, it deletes them now, as it always did.
        if !self.purge_grace.is_zero() {
            let now = now_ms();
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            if !standing_on(&mut tx, actor, library_id).await?.curates() {
                return Err(ServiceError::Forbidden);
            }
            let hidden = hide_in(&mut tx, library_id, &selection, now).await?;
            let restorable_until =
                now.saturating_add(i64::try_from(self.purge_grace.as_millis()).unwrap_or(i64::MAX));
            if hidden > 0 {
                audit(
                    &mut tx,
                    Some(actor.id),
                    "library.tracks_purged",
                    library_id,
                    serde_json::json!({ "tracks": hidden, "restorable_until": restorable_until }),
                )
                .await?;
            }
            tx.commit().await?;
            return Ok(PurgeOutcome {
                purged: hidden,
                restorable_until: Some(restorable_until),
            });
        }
        let purged = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            if !standing_on(&mut tx, actor, library_id).await?.curates() {
                return Err(ServiceError::Forbidden);
            }
            let mut purged = purge_in(&mut tx, library_id, &selection).await?;
            if purged.tracks > 0 {
                audit(
                    &mut tx,
                    Some(actor.id),
                    "library.tracks_purged",
                    library_id,
                    serde_json::json!({ "tracks": purged.tracks }),
                )
                .await?;
            }
            let receipts = self
                .journal_removed(&_writer, &mut tx, &std::mem::take(&mut purged.removed))
                .await?;
            tx.commit().await?;
            for (user_id, receipt) in receipts {
                self.sync.publish(user_id, receipt);
            }
            purged
        };
        // After the commit and out of the writer gate, which releasing takes.
        for hash in &purged.canvases {
            self.release_canvas_blob(hash).await;
        }
        Ok(PurgeOutcome {
            purged: purged.tracks,
            restorable_until: None,
        })
    }

    /// Takes a purge back, for tracks still within their grace window: they
    /// return to the missing-tracks listing, every playlist entry, favourite
    /// and play still attached, as if the purge had never been asked for.
    /// Named tracks, or all of them, and never neither. Nothing is scanned:
    /// a restored track is still missing until its file is found again.
    pub async fn restore_purged_tracks(
        &self,
        actor: LibraryActor,
        library_id: Uuid,
        request: PurgeRequest,
    ) -> Result<RestoreOutcome, ServiceError> {
        let named = match (request.all, request.track_ids) {
            (true, None) => None,
            (false, Some(ids)) if !ids.is_empty() => Some(
                serde_json::to_string(&ids.iter().map(|id| id.to_string()).collect::<Vec<_>>())
                    .expect("a list of strings serializes"),
            ),
            _ => return Err(ServiceError::Invalid),
        };
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        if !standing_on(&mut tx, actor, library_id).await?.curates() {
            return Err(ServiceError::Forbidden);
        }
        // Under the writer gate, which the finishing pass takes too: a track
        // is either restored here or deleted there, never both. Only within
        // the window: one past it is due for deletion even if the pass has not
        // reached it yet — it ran before, or the library was being scanned.
        let restorable_after = now_ms()
            .saturating_sub(i64::try_from(self.purge_grace.as_millis()).unwrap_or(i64::MAX));
        let restored = sqlx::query(
            "UPDATE track SET purged_at = NULL \
             WHERE library_id = ? AND purged_at IS NOT NULL AND purged_at > ? \
               AND (? IS NULL OR id IN (SELECT value FROM json_each(?)))",
        )
        .bind(library_id.to_string())
        .bind(restorable_after)
        .bind(named.as_deref())
        .bind(named.as_deref())
        .execute(&mut *tx)
        .await?
        .rows_affected() as i64;
        if restored > 0 {
            audit(
                &mut tx,
                Some(actor.id),
                "library.purge_restored",
                library_id,
                serde_json::json!({ "tracks": restored }),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(RestoreOutcome { restored })
    }

    /// Tells each account what a removal took from it, through its own sync
    /// journal (RFC-021, what was still open): a favourite or a rating that
    /// went with its track, album or artist is a `delete`, and a playlist
    /// that lost entries is an `upsert` of what it now holds, as an edit of
    /// it would journal. A device mirroring the account then drops them at its
    /// next pull instead of keeping them until the playlist next changes.
    ///
    /// In the removal's own transaction, under the writer gate it holds: the
    /// journal and the catalogue land together or not at all. The receipts
    /// are published by the caller once that transaction has committed.
    pub(super) async fn journal_removed(
        &self,
        writer: &tokio::sync::OwnedMutexGuard<()>,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        removed: &Removed,
    ) -> Result<Vec<(Uuid, MutationReceipt)>, ServiceError> {
        let mut receipts = Vec::new();
        for (kind, rows) in [("favorite", &removed.stars), ("rating", &removed.ratings)] {
            for (user, entity_type, entity) in rows {
                let user_id = parse_uuid(user.clone())?;
                let entity_id = parse_uuid(entity.clone())?;
                let action = if kind == "favorite" {
                    "unstar"
                } else {
                    "set-rating"
                };
                let context = self
                    .server_claim(
                        writer,
                        tx,
                        user_id,
                        action,
                        &format!("{entity_type}:{entity_id}"),
                    )
                    .await?;
                let payload = if kind == "favorite" {
                    serde_json::json!({
                        "entity_type": entity_type,
                        "entity_id": entity_id,
                        "starred": false,
                    })
                } else {
                    serde_json::json!({
                        "entity_type": entity_type,
                        "entity_id": entity_id,
                        "rating": 0,
                    })
                };
                let receipt = self
                    .sync
                    .complete_operation(
                        tx,
                        user_id,
                        context,
                        kind,
                        entity_id,
                        "delete",
                        &payload,
                        Some(entity_id),
                    )
                    .await?;
                receipts.push((user_id, receipt));
            }
        }
        let now = now_ms();
        for playlist in &removed.playlists {
            // Gone with the library's own owner, or with nothing left to say.
            let Some(row) = sqlx::query(
                "UPDATE playlist SET updated_at = ? WHERE id = ? \
                 RETURNING owner_user_id, name, comment, public",
            )
            .bind(now)
            .bind(playlist)
            .fetch_optional(&mut **tx)
            .await?
            else {
                continue;
            };
            let user_id = parse_uuid(row.try_get("owner_user_id")?)?;
            let playlist_id = parse_uuid(playlist.clone())?;
            let track_ids: Vec<String> = sqlx::query_scalar(
                "SELECT track_id FROM playlist_track WHERE playlist_id = ? ORDER BY position",
            )
            .bind(playlist)
            .fetch_all(&mut **tx)
            .await?;
            let context = self
                .server_claim(
                    writer,
                    tx,
                    user_id,
                    "update",
                    &format!("playlist:{playlist_id}"),
                )
                .await?;
            let receipt = self
                .sync
                .complete_operation(
                    tx,
                    user_id,
                    context,
                    "playlist",
                    playlist_id,
                    "upsert",
                    &serde_json::json!({
                        "id": playlist_id,
                        "name": row.try_get::<String, _>("name")?,
                        "comment": row.try_get::<Option<String>, _>("comment")?,
                        "public": row.try_get::<i64, _>("public")? != 0,
                        "track_ids": track_ids,
                    }),
                    Some(playlist_id),
                )
                .await?;
            receipts.push((user_id, receipt));
        }
        Ok(receipts)
    }

    /// Opens a journal operation the server makes on an account's behalf. A
    /// fresh operation id cannot be a replay, so anything but a new claim is a
    /// fault rather than an answer.
    async fn server_claim(
        &self,
        writer: &tokio::sync::OwnedMutexGuard<()>,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        user_id: Uuid,
        action: &str,
        target: &str,
    ) -> Result<MutationContext, ServiceError> {
        let context = MutationContext::server_generated();
        let intent = MutationIntent::new(
            action,
            target,
            &serde_json::json!({ "removed_from_catalogue": true }),
        );
        match self
            .sync
            .claim_operation(writer, tx, user_id, context, intent)
            .await?
        {
            OperationClaim::New => Ok(context),
            OperationClaim::Replayed(_) => Err(ServiceError::Conflict),
        }
    }
}
