//! Deezer as a source of an artist's portrait, chosen by hand (RFC-013
//! decision 4).
//!
//! The server does the looking, so a browser never loads a picture from Deezer
//! and tells it who is looking: a search asks Deezer's artist search, fetches
//! each candidate's picture into the artwork store, and answers previews the
//! server holds. The pick names a Deezer id and nothing else, so what was
//! offered is remembered — a search and its candidates — and the pick is
//! checked against it.
//!
//! A search is recorded first and fetched by a task of its own, which a dropped
//! request does not stop. That task runs under a fixed deadline and leaves the
//! search either complete or deleted; `complete = 0` only ever means a task is
//! fetching for it right now, which is what lets it stand for "one search at a
//! time" without a lock that a crash could leave held.

use std::time::Duration;

use serde::Serialize;
use sqlx::{Row, Sqlite, Transaction};
use utoipa::ToSchema;
use uuid::Uuid;

use super::artist_images::{chooser_library, record_artist_change, write_choice};
use super::{ArtistImageKind, DomainServices, ServiceError};
use crate::auth::authentication::now_ms;
use crate::remote::enrichment::{DeezerCandidate, Enrichment};

/// How many candidates one search offers: two rows of a picker, and twenty
/// image fetches at most behind one request.
const CANDIDATE_LIMIT: usize = 20;

/// The longest query kept; a name longer than this is not a name.
const MAX_QUERY_CHARS: usize = 200;

/// How long a search lives once complete: long enough to look through twenty
/// faces and choose, short enough that the tables hold what is on someone's
/// screen and not a history of searches.
const SEARCH_LIFE_MS: i64 = 60 * 60 * 1000;

/// How long a search's task may run, from the moment it was recorded: the
/// pace it shares with enrichment, the requests and the writes alike.
pub const ARTIST_IMAGE_SEARCH_DEADLINE: Duration = Duration::from_secs(5 * 60);
const FETCH_DEADLINE_MS: i64 = 5 * 60 * 1000;

/// One Deezer artist offered as the portrait, with the preview the server
/// fetched and holds. No picture URL: the client shows the preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ArtistImageCandidate {
    pub deezer_id: i64,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fans: Option<i64>,
    /// The preview, in the artwork store: `GET /api/v2/artwork/{hash}`.
    pub artwork_hash: String,
}

/// The query Deezer is asked, trimmed, and the key a search is stored under,
/// case-folded too: "Björk " and "björk" are one search. `None` for an empty
/// query or one too long to be a name.
fn query_key(query: &str) -> Option<(String, String)> {
    let query = query.trim();
    if query.is_empty() || query.chars().count() > MAX_QUERY_CHARS {
        return None;
    }
    Some((query.to_owned(), query.to_lowercase()))
}

/// The live, complete search of this artist under this key, if any.
async fn live_search(
    tx: &mut Transaction<'_, Sqlite>,
    artist_id: Uuid,
    key: &str,
    now: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT id FROM artist_image_search \
         WHERE artist_id=? AND query_key=? AND complete=1 AND expires_at>? \
         ORDER BY expires_at DESC LIMIT 1",
    )
    .bind(artist_id.to_string())
    .bind(key)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await
}

async fn candidates_of(
    tx: &mut Transaction<'_, Sqlite>,
    search_id: &str,
) -> Result<Vec<ArtistImageCandidate>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT deezer_id, name, fans, artwork_hash FROM artist_image_candidate \
         WHERE search_id=? ORDER BY position",
    )
    .bind(search_id)
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ArtistImageCandidate {
                deezer_id: row.try_get("deezer_id")?,
                name: row.try_get("name")?,
                fans: row.try_get("fans")?,
                artwork_hash: row.try_get("artwork_hash")?,
            })
        })
        .collect()
}

/// Deletes the searches no one can use any more, inside the sweep's
/// transaction and before it looks for unreferenced artwork: the complete ones
/// past their hour, and the incomplete ones past their fetch deadline, which
/// no running task can be. A search still fetching has no `expires_at` to
/// pass and is never touched here.
pub(super) async fn forget_spent_searches(
    tx: &mut Transaction<'_, Sqlite>,
    now: i64,
) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "DELETE FROM artist_image_search \
         WHERE (complete=1 AND expires_at<=?) OR (complete=0 AND searched_at<=?)",
    )
    .bind(now)
    .bind(now.saturating_sub(FETCH_DEADLINE_MS))
    .execute(&mut **tx)
    .await
    .map(|done| done.rows_affected())
}

impl DomainServices {
    /// Shortens how long a portrait search's task may run, so a test can see
    /// it stop without waiting five minutes. Never shorter is the rule the
    /// 429 and the sweep rely on: they count a search as stopped only past
    /// [`ARTIST_IMAGE_SEARCH_DEADLINE`], which a shorter task never reaches.
    pub fn shorten_image_search_deadline(&self, deadline: Duration) {
        let deadline = deadline.min(ARTIST_IMAGE_SEARCH_DEADLINE);
        self.image_search_deadline_ms.store(
            u64::try_from(deadline.as_millis()).unwrap_or(u64::MAX),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    fn image_search_deadline(&self) -> Duration {
        Duration::from_millis(
            self.image_search_deadline_ms
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    fn enrichment_engine(&self) -> Enrichment {
        self.enrichment
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    /// Deezer's artists for `query`, offered as this artist's portrait.
    ///
    /// `NotFound` where Deezer is not configured now, and for anyone who may
    /// not choose the artist's images; `Invalid` for an empty query; `Busy`
    /// while the same account's previous search is still fetching;
    /// `Unavailable` when Deezer or a picture host could not be reached. The
    /// same query for the same artist, while a search for it lives, is
    /// answered from that search and asks Deezer nothing.
    pub async fn artist_image_candidates(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
        query: &str,
    ) -> Result<Vec<ArtistImageCandidate>, ServiceError> {
        let engine = self.enrichment_engine();
        if !engine.searches_deezer() {
            return Err(ServiceError::NotFound);
        }
        let (asked, key) = query_key(query).ok_or(ServiceError::Invalid)?;
        let (search_id, deadline_ms, deadline) = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            chooser_library(&mut tx, user_id, artist_id).await?;
            let now = now_ms();
            let task_deadline = self.image_search_deadline();
            let deadline = tokio::time::Instant::now() + task_deadline;
            if let Some(search_id) = live_search(&mut tx, artist_id, &key, now).await? {
                let candidates = candidates_of(&mut tx, &search_id).await?;
                tx.commit().await?;
                return Ok(candidates);
            }
            // Only a search that may still be fetching counts: one past its
            // deadline has a task that has already stopped.
            let busy: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM artist_image_search \
                 WHERE searched_by=? AND complete=0 AND searched_at>?)",
            )
            .bind(user_id.to_string())
            .bind(now.saturating_sub(FETCH_DEADLINE_MS))
            .fetch_one(&mut *tx)
            .await?;
            if busy {
                return Err(ServiceError::Busy);
            }
            let search_id = Uuid::new_v4().to_string();
            sqlx::query(
                "INSERT INTO artist_image_search \
                   (id, artist_id, query_key, searched_by, searched_at) \
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(&search_id)
            .bind(artist_id.to_string())
            .bind(&key)
            .bind(user_id.to_string())
            .bind(now)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            let deadline_ms =
                now.saturating_add(i64::try_from(task_deadline.as_millis()).unwrap_or(i64::MAX));
            (search_id, deadline_ms, deadline)
        };
        // The fetching is not the request's: a client that goes away drops
        // this handler mid-await, and the task goes on to complete the search
        // or delete it.
        let services = self.clone();
        tokio::spawn(async move {
            services
                .fetch_search(engine, search_id, deadline_ms, deadline, asked)
                .await
        })
        .await
        .map_err(|_| ServiceError::Unavailable)?
    }

    /// The task behind one search: everything under its deadline, then the
    /// search complete or deleted.
    async fn fetch_search(
        &self,
        engine: Enrichment,
        search_id: String,
        deadline_ms: i64,
        deadline: tokio::time::Instant,
        query: String,
    ) -> Result<Vec<ArtistImageCandidate>, ServiceError> {
        let fetched = tokio::time::timeout_at(deadline, async {
            let candidates = engine
                .deezer_candidates(&query, CANDIDATE_LIMIT)
                .await
                .map_err(|_| ServiceError::Unavailable)?;
            self.complete_search(&search_id, deadline_ms, &candidates)
                .await
        })
        .await
        .unwrap_or(Err(ServiceError::Unavailable));
        if fetched.is_err() {
            self.abandon_search(&search_id).await;
        }
        fetched
    }

    /// Stores what a search offered and opens its hour, under the writer gate
    /// and in one transaction that first checks the deadline: past it, the
    /// search is deleted rather than completed.
    async fn complete_search(
        &self,
        search_id: &str,
        deadline_ms: i64,
        candidates: &[DeezerCandidate],
    ) -> Result<Vec<ArtistImageCandidate>, ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        let now = now_ms();
        if now > deadline_ms {
            sqlx::query("DELETE FROM artist_image_search WHERE id=?")
                .bind(search_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Err(ServiceError::Unavailable);
        }
        // Nothing to complete when the artist went, and its searches with it.
        let completed = sqlx::query(
            "UPDATE artist_image_search SET complete=1, expires_at=? \
             WHERE id=? AND complete=0",
        )
        .bind(now.saturating_add(SEARCH_LIFE_MS))
        .bind(search_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if completed == 0 {
            return Err(ServiceError::NotFound);
        }
        let mut offered = Vec::with_capacity(candidates.len());
        for (position, candidate) in candidates.iter().enumerate() {
            let preview = &candidate.preview;
            sqlx::query(
                "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
                 VALUES (?, ?, 'deezer', ?, ?) ON CONFLICT (hash) DO NOTHING",
            )
            .bind(&preview.hash)
            .bind(preview.format)
            .bind(preview.byte_size)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO artist_image_candidate \
                   (search_id, position, deezer_id, name, fans, artwork_hash) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(search_id)
            .bind(position as i64)
            .bind(candidate.deezer_id)
            .bind(&candidate.name)
            .bind(candidate.fans)
            .bind(&preview.hash)
            .execute(&mut *tx)
            .await?;
            offered.push(ArtistImageCandidate {
                deezer_id: candidate.deezer_id,
                name: candidate.name.clone(),
                fans: candidate.fans,
                artwork_hash: preview.hash.clone(),
            });
        }
        tx.commit().await?;
        Ok(offered)
    }

    /// Deletes a search its task could not complete. The previews it stored
    /// are named by nothing and go with the next sweep. A failure here is
    /// left to the sweep too, which deletes the search once past its deadline.
    async fn abandon_search(&self, search_id: &str) {
        let _writer = self.db.writer_guard().await;
        let deleted = sqlx::query("DELETE FROM artist_image_search WHERE id=? AND complete=0")
            .bind(search_id)
            .execute(self.db.pool())
            .await;
        if let Err(error) = deleted {
            tracing::warn!(%error, "could not delete a failed portrait search");
        }
    }

    /// Deletes every search left incomplete: run at boot, before anything is
    /// served, since nothing can be fetching for one across a restart.
    pub async fn forget_unfinished_image_searches(&self) -> Result<u64, ServiceError> {
        let _writer = self.db.writer_guard().await;
        Ok(
            sqlx::query("DELETE FROM artist_image_search WHERE complete=0")
                .execute(self.db.pool())
                .await?
                .rows_affected(),
        )
    }

    /// Choose the portrait a live search of this artist offered as
    /// `deezer_id`: that candidate's preview, never fetched again.
    ///
    /// `NotFound` where Deezer is not configured now, a live search
    /// notwithstanding, and for anyone who may not choose; `Invalid` for an id
    /// no live search of this artist offered. The read and the write share
    /// one transaction under the writer gate, which the sweep takes too, so a
    /// preview live when it is read cannot be swept before it is named.
    pub async fn choose_deezer_portrait(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
        deezer_id: i64,
    ) -> Result<(), ServiceError> {
        if !self.enrichment_engine().searches_deezer() {
            return Err(ServiceError::NotFound);
        }
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        let library_id = chooser_library(&mut tx, user_id, artist_id).await?;
        let now = now_ms();
        // The most recent search that offered it: the previews are the same
        // picture at the same size, so this decides nothing visible.
        let preview: Option<String> = sqlx::query_scalar(
            "SELECT c.artwork_hash FROM artist_image_candidate c \
             JOIN artist_image_search s ON s.id=c.search_id \
             WHERE s.artist_id=? AND s.complete=1 AND s.expires_at>? AND c.deezer_id=? \
             ORDER BY s.searched_at DESC LIMIT 1",
        )
        .bind(artist_id.to_string())
        .bind(now)
        .bind(deezer_id)
        .fetch_optional(&mut *tx)
        .await?;
        let preview = preview.ok_or(ServiceError::Invalid)?;
        write_choice(
            &mut tx,
            artist_id,
            ArtistImageKind::Portrait,
            &preview,
            "deezer",
            user_id,
            now,
        )
        .await
        .map_err(|error| match error {
            // A preview that slipped out from under the pick fails the key
            // rather than storing a dangling hash; the owner searches again.
            sqlx::Error::Database(ref database) if database.is_foreign_key_violation() => {
                ServiceError::Invalid
            }
            error => ServiceError::Database(error),
        })?;
        record_artist_change(&mut tx, library_id, artist_id, now).await?;
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
