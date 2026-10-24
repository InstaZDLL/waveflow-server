//! Splitting a joined artist credit, by decision (RFC-015).
//!
//! A decision belongs to the library: taken by its owners and managers, read
//! back by every scan, applied at once to the stored catalogue without
//! reading a file, and undone the same way. The rule itself is
//! `crate::credit_split`; this is who may invoke it, and what follows from it
//! — the library feed, the derivations a scan would rerun, and the favourites
//! and ratings that would otherwise point at an artist nobody credits.

use std::collections::{BTreeSet, HashSet};

use sqlx::{Sqlite, Transaction};
use tokio::sync::OwnedMutexGuard;

use super::*;
use crate::authentication::now_ms;
use crate::credit_split::{credit_key, fragments, is_candidate};
use crate::database::LibraryRole;

/// One fragment of a credit an owner is asked about, and whether the library
/// already has an artist of that name — the strongest hint that a split is
/// right.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreditFragment {
    pub name: String,
    /// The artist this fragment would land on, when it exists already.
    pub artist_id: Option<Uuid>,
    /// Albums it is the album artist of, `0` when it does not exist.
    pub album_count: i64,
}

/// A joined credit with no decision yet.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreditCandidate {
    /// The artist row the joined credit makes today.
    pub artist_id: Uuid,
    pub name: String,
    /// Available tracks crediting it as artist or album artist.
    pub track_count: i64,
    pub fragments: Vec<CreditFragment>,
}

/// What was decided about a credit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum CreditDecisionKind {
    Split,
    Keep,
}

impl CreditDecisionKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Keep => "keep",
        }
    }
}

/// A recorded decision.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreditDecisionRecord {
    pub id: Uuid,
    /// The credit as it read when the decision was taken.
    pub credit: String,
    pub decision: CreditDecisionKind,
    /// The names a split credits instead, in order; absent for a keep.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<String>>,
    /// Who decided, by username; `null` once that account is gone.
    pub decided_by: Option<String>,
    pub decided_at: i64,
}

/// The caller may decide about this library's credits: `NotFound` for a
/// library they are not in, `Forbidden` for one they may only listen to, both
/// answered as the same `404`.
pub(super) async fn curated_library(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: Uuid,
    library_id: Uuid,
) -> Result<(), ServiceError> {
    let role: String =
        sqlx::query_scalar("SELECT role FROM library_member WHERE user_id=? AND library_id=?")
            .bind(user_id.to_string())
            .bind(library_id.to_string())
            .fetch_optional(&mut **tx)
            .await?
            .ok_or(ServiceError::NotFound)?;
    let role = LibraryRole::from_str(&role).map_err(|_| ServiceError::Invalid)?;
    if role.may_write_metadata() {
        Ok(())
    } else {
        Err(ServiceError::Forbidden)
    }
}

/// What a pass over the affected tracks changed, for the library feed.
#[derive(Default)]
struct Moved {
    tracks: Vec<(Uuid, String)>,
    albums: BTreeSet<Uuid>,
}

fn record_event(
    library_id: Uuid,
    entity_type: &'static str,
    entity_id: Uuid,
    action: &'static str,
    payload: serde_json::Value,
    now: i64,
) -> crate::catalog::LibraryChange<'static> {
    crate::catalog::LibraryChange {
        library_id,
        entity_type,
        entity_id,
        action,
        payload,
        changed_at: now,
        origin_device_id: None,
    }
}

impl DomainServices {
    /// The joined credits of a library that wait for a decision (RFC-015
    /// decision 8), sorted as the desktop sorts them: most fragments already
    /// known first, then the most tracks, then the name.
    pub async fn credit_candidates(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<Vec<CreditCandidate>, ServiceError> {
        let mut tx = self.db.pool().begin().await?;
        curated_library(&mut tx, user_id, library_id).await?;
        // The four characters narrow the read; the rule itself is Rust's,
        // because "two fragments, none the whole" is not a pattern.
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT ar.id, ar.name, COUNT(DISTINCT t.id) FROM artist ar \
             JOIN track_participant tp ON tp.artist_id = ar.id \
               AND tp.role IN ('artist', 'albumartist') \
             JOIN track t ON t.id = tp.track_id AND t.is_available = 1 \
             WHERE ar.library_id = ? \
               AND (ar.name LIKE '%,%' OR ar.name LIKE '%，%' \
                    OR ar.name LIKE '%、%' OR ar.name LIKE '%､%') \
             GROUP BY ar.id",
        )
        .bind(library_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
        let decided: HashSet<String> = sqlx::query_scalar(
            "SELECT credit_key FROM artist_credit_decision WHERE library_id = ?",
        )
        .bind(library_id.to_string())
        .fetch_all(&mut *tx)
        .await?
        .into_iter()
        .collect();
        let mut candidates = Vec::new();
        for (id, name, track_count) in rows {
            if !is_candidate(&name) || decided.contains(&credit_key(&name)) {
                continue;
            }
            let mut parts = Vec::new();
            for fragment in fragments(&name) {
                let derived = self.db.pid().artist_id(library_id, fragment.trim());
                let known: Option<i64> = sqlx::query_scalar(
                    "SELECT COALESCE((SELECT album_count FROM artist_role_stats \
                       WHERE artist_id = ar.id AND role = 'albumartist'), 0) \
                     FROM artist ar WHERE ar.id = ? AND ar.library_id = ?",
                )
                .bind(derived.to_string())
                .bind(library_id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
                parts.push(CreditFragment {
                    name: fragment,
                    artist_id: known.map(|_| derived),
                    album_count: known.unwrap_or(0),
                });
            }
            candidates.push(CreditCandidate {
                artist_id: parse_uuid(id)?,
                name,
                track_count,
                fragments: parts,
            });
        }
        tx.commit().await?;
        candidates.sort_by(|left, right| {
            let known = |candidate: &CreditCandidate| {
                candidate
                    .fragments
                    .iter()
                    .filter(|fragment| fragment.artist_id.is_some())
                    .count()
            };
            known(right)
                .cmp(&known(left))
                .then(right.track_count.cmp(&left.track_count))
                .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
                .then_with(|| left.artist_id.cmp(&right.artist_id))
        });
        Ok(candidates)
    }

    /// A library's decisions, newest first.
    pub async fn credit_decisions(
        &self,
        user_id: Uuid,
        library_id: Uuid,
    ) -> Result<Vec<CreditDecisionRecord>, ServiceError> {
        let mut tx = self.db.pool().begin().await?;
        curated_library(&mut tx, user_id, library_id).await?;
        let rows = sqlx::query(
            "SELECT d.id, d.credit, d.decision, d.parts, a.username, d.decided_at \
             FROM artist_credit_decision d LEFT JOIN account a ON a.id = d.decided_by \
             WHERE d.library_id = ? ORDER BY d.decided_at DESC, d.id",
        )
        .bind(library_id.to_string())
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        rows.into_iter().map(decision_from_row).collect()
    }

    /// Records a decision about the credit `artist_id` makes, and applies it at
    /// once (RFC-015 decisions 3 and 5).
    ///
    /// The artist must be a candidate; a credit already decided is a `409`, so
    /// changing one's mind is an undo followed by a new decision, and two
    /// owners deciding at once cannot overwrite each other unseen.
    pub async fn decide_credit(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        artist_id: Uuid,
        kind: CreditDecisionKind,
    ) -> Result<CreditDecisionRecord, ServiceError> {
        let writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        curated_library(&mut tx, user_id, library_id).await?;
        let name: String =
            sqlx::query_scalar("SELECT name FROM artist WHERE id = ? AND library_id = ?")
                .bind(artist_id.to_string())
                .bind(library_id.to_string())
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(ServiceError::NotFound)?;
        if !is_candidate(&name) {
            return Err(ServiceError::Invalid);
        }
        let key = credit_key(&name);
        let taken: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM artist_credit_decision WHERE library_id = ? AND credit_key = ?",
        )
        .bind(library_id.to_string())
        .bind(&key)
        .fetch_optional(&mut *tx)
        .await?;
        if taken.is_some() {
            return Err(ServiceError::Conflict);
        }
        let parts = (kind == CreditDecisionKind::Split).then(|| fragments(&name));
        let id = Uuid::new_v4();
        let now = now_ms();
        sqlx::query(
            "INSERT INTO artist_credit_decision \
               (id, library_id, credit_key, credit, decision, parts, decided_by, decided_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(library_id.to_string())
        .bind(&key)
        .bind(name.trim())
        .bind(kind.as_str())
        .bind(
            parts
                .as_ref()
                .map(|parts| serde_json::to_string(parts).map_err(|_| ServiceError::Invalid))
                .transpose()?,
        )
        .bind(user_id.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;

        let mut receipts = Vec::new();
        if let Some(parts) = &parts {
            let tracks: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT track_id FROM track_participant WHERE artist_id = ?",
            )
            .bind(artist_id.to_string())
            .fetch_all(&mut *tx)
            .await?;
            let moved = self
                .recompute_tracks(&mut tx, library_id, &tracks, now)
                .await?;
            self.record_moved(&mut tx, library_id, &moved, now).await?;
            let part_ids: Vec<Uuid> = parts
                .iter()
                .map(|part| self.db.pid().artist_id(library_id, part.trim()))
                .collect();
            for part in &part_ids {
                crate::catalog::record_library_event(
                    &mut tx,
                    record_event(
                        library_id,
                        "artist",
                        *part,
                        "upsert",
                        serde_json::json!({}),
                        now,
                    ),
                )
                .await?;
            }
            if !still_credited(&mut tx, artist_id).await? {
                receipts = self
                    .carry_user_data(&writer, &mut tx, artist_id, &part_ids)
                    .await?;
                crate::catalog::record_library_event(
                    &mut tx,
                    record_event(
                        library_id,
                        "artist",
                        artist_id,
                        "delete",
                        serde_json::json!({}),
                        now,
                    ),
                )
                .await?;
            }
        }
        if parts.is_some() {
            rederive(&mut tx, library_id).await?;
        }
        tx.commit().await?;
        drop(writer);
        for (user, receipt) in receipts {
            self.sync.publish(user, receipt);
        }
        self.credit_decisions(user_id, library_id)
            .await?
            .into_iter()
            .find(|decision| decision.id == id)
            .ok_or(ServiceError::NotFound)
    }

    /// Removes a decision and restores what the library would hold without it
    /// (RFC-015 decision 7): the same computation, run again for the tracks
    /// the split reached, reading the file's credits back from `track_credit`.
    ///
    /// Favourites carried onto the parts stay there: by now the account may
    /// like them on their own merits.
    pub async fn remove_credit_decision(
        &self,
        user_id: Uuid,
        library_id: Uuid,
        decision_id: Uuid,
    ) -> Result<(), ServiceError> {
        let writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        curated_library(&mut tx, user_id, library_id).await?;
        let row: (String, String, Option<String>) = sqlx::query_as(
            "SELECT credit, decision, parts FROM artist_credit_decision \
             WHERE id = ? AND library_id = ?",
        )
        .bind(decision_id.to_string())
        .bind(library_id.to_string())
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(ServiceError::NotFound)?;
        sqlx::query("DELETE FROM artist_credit_decision WHERE id = ?")
            .bind(decision_id.to_string())
            .execute(&mut *tx)
            .await?;
        let (credit, decision, parts) = row;
        let split = decision == "split";
        if split {
            let parts: Vec<String> = parts
                .and_then(|parts| serde_json::from_str(&parts).ok())
                .unwrap_or_default();
            let part_ids: Vec<Uuid> = parts
                .iter()
                .map(|part| self.db.pid().artist_id(library_id, part.trim()))
                .collect();
            // Every track the split reached credits at least one of its parts:
            // the first occurrence of each is kept. Recomputing a track the
            // undo does not change writes nothing, so a superset is safe.
            let mut tracks = Vec::new();
            for part in &part_ids {
                tracks.extend(
                    sqlx::query_scalar::<_, String>(
                        "SELECT DISTINCT track_id FROM track_participant WHERE artist_id = ?",
                    )
                    .bind(part.to_string())
                    .fetch_all(&mut *tx)
                    .await?,
                );
            }
            tracks.sort();
            tracks.dedup();
            let now = now_ms();
            let moved = self
                .recompute_tracks(&mut tx, library_id, &tracks, now)
                .await?;
            self.record_moved(&mut tx, library_id, &moved, now).await?;
            let joined = self.db.pid().artist_id(library_id, credit.trim());
            if still_credited(&mut tx, joined).await? {
                crate::catalog::record_library_event(
                    &mut tx,
                    record_event(
                        library_id,
                        "artist",
                        joined,
                        "upsert",
                        serde_json::json!({}),
                        now,
                    ),
                )
                .await?;
            }
            for part in &part_ids {
                let action = if still_credited(&mut tx, *part).await? {
                    "upsert"
                } else {
                    "delete"
                };
                crate::catalog::record_library_event(
                    &mut tx,
                    record_event(
                        library_id,
                        "artist",
                        *part,
                        action,
                        serde_json::json!({}),
                        now,
                    ),
                )
                .await?;
            }
        }
        if split {
            rederive(&mut tx, library_id).await?;
        }
        tx.commit().await?;
        drop(writer);
        Ok(())
    }

    async fn recompute_tracks(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        library_id: Uuid,
        tracks: &[String],
        now: i64,
    ) -> Result<Moved, ServiceError> {
        let decisions = crate::catalog::credit_decisions(tx, library_id).await?;
        let mut moved = Moved::default();
        for track in tracks {
            let track_id = parse_uuid(track.clone())?;
            let outcome = crate::catalog::recompute_track_participants(
                tx,
                self.db.pid(),
                library_id,
                track_id,
                &decisions,
                now,
            )
            .await?;
            if outcome.changed {
                moved.tracks.push((track_id, outcome.full_hash));
                moved.albums.extend(outcome.album_id);
            }
        }
        Ok(moved)
    }

    /// What a scan of the same change would put on the library feed: each
    /// track with its hash, each album once.
    async fn record_moved(
        &self,
        tx: &mut Transaction<'_, Sqlite>,
        library_id: Uuid,
        moved: &Moved,
        now: i64,
    ) -> Result<(), ServiceError> {
        for (track, full_hash) in &moved.tracks {
            crate::catalog::record_library_event(
                tx,
                record_event(
                    library_id,
                    "track",
                    *track,
                    "upsert",
                    serde_json::json!({ "full_hash": full_hash }),
                    now,
                ),
            )
            .await?;
        }
        for album in &moved.albums {
            crate::catalog::record_library_event(
                tx,
                record_event(
                    library_id,
                    "album",
                    *album,
                    "upsert",
                    serde_json::json!({}),
                    now,
                ),
            )
            .await?;
        }
        Ok(())
    }

    /// Carries each account's favourite and rating on an artist the split
    /// removed onto every part, through that account's journal (RFC-015
    /// decision 4), so a desktop mirror sees the same move the server made.
    ///
    /// A part the account already starred keeps its own date; a part it
    /// already rated keeps its own rating.
    async fn carry_user_data(
        &self,
        writer: &OwnedMutexGuard<()>,
        tx: &mut Transaction<'_, Sqlite>,
        from: Uuid,
        to: &[Uuid],
    ) -> Result<Vec<(Uuid, MutationReceipt)>, ServiceError> {
        let mut receipts = Vec::new();
        let stars: Vec<(String, i64)> = sqlx::query_as(
            "SELECT user_id, starred_at FROM user_star \
             WHERE entity_type = 'artist' AND entity_id = ?",
        )
        .bind(from.to_string())
        .fetch_all(&mut **tx)
        .await?;
        for (user, starred_at) in stars {
            let user_id = parse_uuid(user)?;
            for part in to {
                let context = self.claim(writer, tx, user_id, "star", *part).await?;
                sqlx::query(
                    "INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) \
                     VALUES (?, 'artist', ?, ?) ON CONFLICT DO NOTHING",
                )
                .bind(user_id.to_string())
                .bind(part.to_string())
                .bind(starred_at)
                .execute(&mut **tx)
                .await?;
                let receipt = self
                    .sync
                    .complete_operation(
                        tx,
                        user_id,
                        context,
                        "favorite",
                        *part,
                        "upsert",
                        &serde_json::json!({
                            "entity_type": "artist",
                            "entity_id": part,
                            "starred": true,
                        }),
                        Some(*part),
                    )
                    .await?;
                receipts.push((user_id, receipt));
            }
            let context = self.claim(writer, tx, user_id, "unstar", from).await?;
            sqlx::query(
                "DELETE FROM user_star WHERE user_id = ? AND entity_type = 'artist' \
                 AND entity_id = ?",
            )
            .bind(user_id.to_string())
            .bind(from.to_string())
            .execute(&mut **tx)
            .await?;
            let receipt = self
                .sync
                .complete_operation(
                    tx,
                    user_id,
                    context,
                    "favorite",
                    from,
                    "delete",
                    &serde_json::json!({
                        "entity_type": "artist",
                        "entity_id": from,
                        "starred": false,
                    }),
                    Some(from),
                )
                .await?;
            receipts.push((user_id, receipt));
        }

        let ratings: Vec<(String, i64)> = sqlx::query_as(
            "SELECT user_id, rating FROM user_rating \
             WHERE entity_type = 'artist' AND entity_id = ?",
        )
        .bind(from.to_string())
        .fetch_all(&mut **tx)
        .await?;
        for (user, rating) in ratings {
            let user_id = parse_uuid(user)?;
            for part in to {
                let rated: Option<i64> = sqlx::query_scalar(
                    "SELECT 1 FROM user_rating WHERE user_id = ? AND entity_type = 'artist' \
                     AND entity_id = ?",
                )
                .bind(user_id.to_string())
                .bind(part.to_string())
                .fetch_optional(&mut **tx)
                .await?;
                if rated.is_some() {
                    continue;
                }
                let context = self.claim(writer, tx, user_id, "set-rating", *part).await?;
                sqlx::query(
                    "INSERT INTO user_rating (user_id, entity_type, entity_id, rating, updated_at) \
                     VALUES (?, 'artist', ?, ?, ?)",
                )
                .bind(user_id.to_string())
                .bind(part.to_string())
                .bind(rating)
                .bind(now_ms())
                .execute(&mut **tx)
                .await?;
                let receipt = self
                    .sync
                    .complete_operation(
                        tx,
                        user_id,
                        context,
                        "rating",
                        *part,
                        "upsert",
                        &serde_json::json!({
                            "entity_type": "artist",
                            "entity_id": part,
                            "rating": rating,
                        }),
                        Some(*part),
                    )
                    .await?;
                receipts.push((user_id, receipt));
            }
            let context = self.claim(writer, tx, user_id, "set-rating", from).await?;
            sqlx::query(
                "DELETE FROM user_rating WHERE user_id = ? AND entity_type = 'artist' \
                 AND entity_id = ?",
            )
            .bind(user_id.to_string())
            .bind(from.to_string())
            .execute(&mut **tx)
            .await?;
            let receipt = self
                .sync
                .complete_operation(
                    tx,
                    user_id,
                    context,
                    "rating",
                    from,
                    "delete",
                    &serde_json::json!({
                        "entity_type": "artist",
                        "entity_id": from,
                        "rating": 0,
                    }),
                    Some(from),
                )
                .await?;
            receipts.push((user_id, receipt));
        }
        Ok(receipts)
    }

    /// Opens a journal operation the server makes on an account's behalf. A
    /// fresh operation id cannot be a replay, so anything but a new claim is a
    /// fault rather than an answer.
    async fn claim(
        &self,
        writer: &OwnedMutexGuard<()>,
        tx: &mut Transaction<'_, Sqlite>,
        user_id: Uuid,
        action: &str,
        artist: Uuid,
    ) -> Result<MutationContext, ServiceError> {
        let context = MutationContext::server_generated();
        let intent = MutationIntent::new(
            action,
            &format!("artist:{artist}"),
            &serde_json::json!({ "credit_split": true }),
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

/// The derivations a scan reruns at its end, rerun because the credits they
/// are derived from moved: album credits, per-role counts, the artist sweep
/// and the search index, then the sort names.
///
/// In the decision's own transaction, under the writer gate it already holds:
/// a decision recorded while its derivations failed would leave an artist row
/// the feed has already announced as deleted, and a retry would only answer
/// `409`. Together, the two land or neither does.
async fn rederive(tx: &mut Transaction<'_, Sqlite>, library_id: Uuid) -> Result<(), sqlx::Error> {
    crate::database::Database::consolidate_catalog_derivations_in(tx, library_id).await?;
    crate::database::Database::consolidate_sort_names_in(tx, library_id).await
}

/// Whether anything still credits an artist: a participant, or an album that
/// hangs off it. What the end-of-scan sweep checks before deleting the row.
async fn still_credited(
    tx: &mut Transaction<'_, Sqlite>,
    artist_id: Uuid,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM track_participant WHERE artist_id = ?) \
             OR EXISTS (SELECT 1 FROM album WHERE album_artist_id = ?)",
    )
    .bind(artist_id.to_string())
    .bind(artist_id.to_string())
    .fetch_one(&mut **tx)
    .await
}

fn decision_from_row(row: sqlx::sqlite::SqliteRow) -> Result<CreditDecisionRecord, ServiceError> {
    let decision = match row.try_get::<&str, _>("decision")? {
        "split" => CreditDecisionKind::Split,
        _ => CreditDecisionKind::Keep,
    };
    let parts: Option<String> = row.try_get("parts")?;
    Ok(CreditDecisionRecord {
        id: parse_uuid(row.try_get("id")?)?,
        credit: row.try_get("credit")?,
        decision,
        parts: parts.and_then(|parts| serde_json::from_str(&parts).ok()),
        decided_by: row.try_get("username")?,
        decided_at: row.try_get("decided_at")?,
    })
}
