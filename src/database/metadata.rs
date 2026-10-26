//! The provider caches: artist metadata and neighbours (RFC-011), the
//! portraits and banners linked from them (RFC-014), and lyrics lookups
//! (RFC-024).

use sqlx::Row;
use uuid::Uuid;

use super::Database;

/// RFC-011's cache, which [`crate::enrichment`] fills and reads.
///
/// No user id and no tenancy anywhere in it: what a provider says about a name
/// is the same whoever asks, and nothing here is account data (decision 4).
/// Tenancy applies where a cached name meets the catalogue, which is the
/// reader's query, not these.
impl Database {
    /// Every provider's row for one canonical name, in no particular order.
    ///
    /// A row from a provider this build does not know — written by a later
    /// version the instance was downgraded from — is passed over rather than
    /// refused: it is a fact about a provider nobody configured here.
    pub async fn metadata_artist_rows(
        &self,
        name_canonical: &str,
    ) -> Result<Vec<crate::enrichment::CachedArtist>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT provider, outcome, bio_summary, bio_full, image_hash, fanart_hash, \
                    fetched_at, expires_at \
             FROM metadata_artist WHERE name_canonical = ?",
        )
        .bind(name_canonical)
        .fetch_all(&self.pool)
        .await?;
        let mut cached = Vec::with_capacity(rows.len());
        for row in rows {
            let provider: String = row.try_get("provider")?;
            let Some(provider) = crate::enrichment::Provider::parse(&provider) else {
                continue;
            };
            let outcome: String = row.try_get("outcome")?;
            let outcome = crate::enrichment::Outcome::parse(&outcome).ok_or_else(|| {
                sqlx::Error::Decode(format!("unknown enrichment outcome {outcome:?}").into())
            })?;
            cached.push(crate::enrichment::CachedArtist {
                provider,
                outcome,
                bio_summary: row.try_get("bio_summary")?,
                bio_full: row.try_get("bio_full")?,
                image_hash: row.try_get("image_hash")?,
                fanart_hash: row.try_get("fanart_hash")?,
                fetched_at: row.try_get("fetched_at")?,
                expires_at: row.try_get("expires_at")?,
            });
        }
        Ok(cached)
    }

    /// One provider's ranked neighbours for one canonical name, best first.
    pub async fn metadata_artist_similar(
        &self,
        provider: crate::enrichment::Provider,
        name_canonical: &str,
    ) -> Result<Vec<crate::enrichment::CachedSimilar>, sqlx::Error> {
        let rows = sqlx::query(
            "SELECT similar_name, similar_canonical, score FROM metadata_artist_similar \
             WHERE provider = ? AND name_canonical = ? ORDER BY rank",
        )
        .bind(provider.as_str())
        .bind(name_canonical)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(crate::enrichment::CachedSimilar {
                    name: row.try_get("similar_name")?,
                    name_canonical: row.try_get("similar_canonical")?,
                    score: row.try_get("score")?,
                })
            })
            .collect()
    }

    /// Links the portrait `hash` to every artist called `name_canonical` that
    /// does not show it yet, emitting one artist event per row it moved.
    ///
    /// Read first without the writer gate, since this runs on the read path
    /// and almost always finds nothing to do. The artwork row is checked in
    /// the same statement as the link: the sweeper may have taken it since
    /// the cache row was read.
    pub(crate) async fn link_artist_portrait(
        &self,
        name_canonical: &str,
        hash: &str,
    ) -> Result<(), sqlx::Error> {
        let unlinked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM artist \
               WHERE canonical_name = ? AND artwork_hash IS NOT ?)",
        )
        .bind(name_canonical)
        .bind(hash)
        .fetch_one(&self.pool)
        .await?;
        if !unlinked {
            return Ok(());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        link_portrait_in(
            &mut tx,
            name_canonical,
            hash,
            crate::authentication::now_ms(),
        )
        .await?;
        tx.commit().await
    }

    /// Links the banner `hash` to every artist called `name_canonical` that
    /// does not show it yet — [`Self::link_artist_portrait`], for the fanart
    /// of RFC-014.
    pub(crate) async fn link_artist_fanart(
        &self,
        name_canonical: &str,
        hash: &str,
    ) -> Result<(), sqlx::Error> {
        let unlinked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM artist \
               WHERE canonical_name = ? AND fanart_hash IS NOT ?)",
        )
        .bind(name_canonical)
        .bind(hash)
        .fetch_one(&self.pool)
        .await?;
        if !unlinked {
            return Ok(());
        }
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        link_fanart_in(
            &mut tx,
            name_canonical,
            Some(hash),
            crate::authentication::now_ms(),
        )
        .await?;
        tx.commit().await
    }

    /// What one lyrics provider said for one lookup key (RFC-024 decision
    /// 4), if it said anything.
    pub async fn lyrics_lookup_row(
        &self,
        provider: &str,
        lookup_key: &str,
    ) -> Result<Option<crate::lrclib::CachedLyrics>, sqlx::Error> {
        let Some(row) = sqlx::query(
            "SELECT outcome, synced_lyrics, plain_lyrics, fetched_at, expires_at \
             FROM lyrics_lookup WHERE provider = ? AND lookup_key = ?",
        )
        .bind(provider)
        .bind(lookup_key)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(None);
        };
        let outcome: String = row.try_get("outcome")?;
        let outcome = crate::lrclib::LyricsOutcome::parse(&outcome).ok_or_else(|| {
            sqlx::Error::Decode(format!("unknown lyrics outcome {outcome:?}").into())
        })?;
        Ok(Some(crate::lrclib::CachedLyrics {
            outcome,
            synced: row.try_get("synced_lyrics")?,
            plain: row.try_get("plain_lyrics")?,
            fetched_at: row.try_get("fetched_at")?,
            expires_at: row.try_get("expires_at")?,
        }))
    }

    /// Records what a lyrics provider answered, replacing what it said
    /// before. Taken under the writer gate, after the request is over.
    pub(crate) async fn write_lyrics_lookup(
        &self,
        provider: &str,
        lookup_key: &str,
        answer: &crate::lrclib::CachedLyrics,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        sqlx::query(
            "INSERT INTO lyrics_lookup \
                (provider, lookup_key, outcome, synced_lyrics, plain_lyrics, fetched_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (provider, lookup_key) DO UPDATE SET \
                outcome = excluded.outcome, synced_lyrics = excluded.synced_lyrics, \
                plain_lyrics = excluded.plain_lyrics, fetched_at = excluded.fetched_at, \
                expires_at = excluded.expires_at",
        )
        .bind(provider)
        .bind(lookup_key)
        .bind(answer.outcome.as_str())
        .bind(answer.synced.as_deref())
        .bind(answer.plain.as_deref())
        .bind(answer.fetched_at)
        .bind(answer.expires_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }

    /// Records what a provider answered: its row, its whole neighbour list,
    /// and its portrait — the artwork row and every artist it belongs to —
    /// in one transaction.
    ///
    /// One transaction is what keeps the sweeper from ever seeing the artwork
    /// row unreferenced: it deletes under this same gate, so it runs wholly
    /// before or wholly after. The writer gate is taken here and nowhere
    /// earlier — the requests that produced `answer` are over, and the
    /// portrait's file is already in the store, so no network or file I/O
    /// ever runs under it.
    pub(crate) async fn write_metadata_artist(
        &self,
        provider: crate::enrichment::Provider,
        name_canonical: &str,
        answer: &crate::enrichment::ArtistAnswer,
        fetched_at: i64,
        expires_at: i64,
    ) -> Result<(), sqlx::Error> {
        let _writer = self.writer_guard().await;
        let mut tx = self.begin_write(&_writer).await?;
        // What this provider's row named before, read before it is replaced:
        // a refresh without the banner unlinks it (RFC-014 decision 3).
        let previous_fanart: Option<String> = sqlx::query_scalar(
            "SELECT fanart_hash FROM metadata_artist WHERE provider = ? AND name_canonical = ?",
        )
        .bind(provider.as_str())
        .bind(name_canonical)
        .fetch_optional(&mut *tx)
        .await?
        .flatten();
        for image in [&answer.image, &answer.fanart].into_iter().flatten() {
            sqlx::query(
                "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
                 VALUES (?, ?, ?, ?, ?) ON CONFLICT (hash) DO NOTHING",
            )
            .bind(&image.hash)
            .bind(image.format)
            .bind(provider.as_str())
            .bind(image.byte_size)
            .bind(fetched_at)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO metadata_artist \
                (provider, name_canonical, outcome, bio_summary, bio_full, image_hash, \
                 fanart_hash, fetched_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (provider, name_canonical) DO UPDATE SET \
                outcome = excluded.outcome, bio_summary = excluded.bio_summary, \
                bio_full = excluded.bio_full, image_hash = excluded.image_hash, \
                fanart_hash = excluded.fanart_hash, \
                fetched_at = excluded.fetched_at, expires_at = excluded.expires_at",
        )
        .bind(provider.as_str())
        .bind(name_canonical)
        .bind(answer.outcome().as_str())
        .bind(answer.bio_summary.as_deref())
        .bind(answer.bio_full.as_deref())
        .bind(answer.image.as_ref().map(|image| image.hash.as_str()))
        .bind(answer.fanart.as_ref().map(|fanart| fanart.hash.as_str()))
        .bind(fetched_at)
        .bind(expires_at)
        .execute(&mut *tx)
        .await?;
        if let Some(image) = &answer.image {
            link_portrait_in(&mut tx, name_canonical, &image.hash, fetched_at).await?;
        }
        match (&answer.fanart, previous_fanart) {
            (Some(fanart), _) => {
                link_fanart_in(&mut tx, name_canonical, Some(&fanart.hash), fetched_at).await?;
            }
            // A miss, or a hit that no longer carries the fanart: the rows
            // still showing the one this provider named go back to none.
            (None, Some(previous)) => {
                unlink_fanart_in(&mut tx, name_canonical, &previous, fetched_at).await?;
            }
            (None, None) => {}
        }
        sqlx::query(
            "DELETE FROM metadata_artist_similar WHERE provider = ? AND name_canonical = ?",
        )
        .bind(provider.as_str())
        .bind(name_canonical)
        .execute(&mut *tx)
        .await?;
        for (rank, similar) in answer.similar.iter().enumerate() {
            sqlx::query(
                "INSERT INTO metadata_artist_similar \
                    (provider, name_canonical, rank, similar_name, similar_canonical, score) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(provider.as_str())
            .bind(name_canonical)
            .bind(rank as i64)
            .bind(&similar.name)
            .bind(&similar.name_canonical)
            .bind(similar.score)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await
    }
}

/// Points every artist called `name_canonical` at the portrait `hash`, and
/// records one artist event for each row that actually moved — RFC-007
/// decision 6's rule for albums, which an artist now follows: a refresh that
/// fetched the same picture changes nothing and says nothing.
///
/// Artist rows are per library, and so are events, so each moved row carries
/// its own library. A row whose artwork row is gone is not linked: the cache
/// may name a hash the sweeper has since taken. Nor is a hash the cache no
/// longer names: a read that saw the old row, then repairs after a fill
/// replaced it, would put the old portrait back and announce it.
async fn link_portrait_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name_canonical: &str,
    hash: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    let moved = sqlx::query(
        "UPDATE artist SET artwork_hash = ?, updated_at = ? \
         WHERE canonical_name = ? AND artwork_hash IS NOT ? \
           AND EXISTS (SELECT 1 FROM artwork WHERE artwork.hash = ?) \
           AND EXISTS (SELECT 1 FROM metadata_artist \
                       WHERE name_canonical = ? AND image_hash = ?) \
         RETURNING id, library_id",
    )
    .bind(hash)
    .bind(now)
    .bind(name_canonical)
    .bind(hash)
    .bind(hash)
    .bind(name_canonical)
    .bind(hash)
    .fetch_all(&mut **tx)
    .await?;
    for row in moved {
        let id: String = row.try_get("id")?;
        let library_id: String = row.try_get("library_id")?;
        let parse =
            |value: &str| Uuid::parse_str(value).map_err(|error| sqlx::Error::Decode(error.into()));
        crate::catalog::record_library_event(
            tx,
            crate::catalog::LibraryChange {
                library_id: parse(&library_id)?,
                entity_type: "artist",
                entity_id: parse(&id)?,
                action: "upsert",
                payload: serde_json::json!({}),
                changed_at: now,
                origin_device_id: None,
            },
        )
        .await?;
    }
    Ok(())
}

/// Points every artist called `name_canonical` at the banner `hash`
/// (RFC-014 decision 3), as [`link_portrait_in`] does the portrait: only rows
/// that move, one artist event each, and only while the artwork row exists and
/// a cache row still names it.
async fn link_fanart_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name_canonical: &str,
    hash: Option<&str>,
    now: i64,
) -> Result<(), sqlx::Error> {
    let Some(hash) = hash else {
        return Ok(());
    };
    let moved = sqlx::query(
        "UPDATE artist SET fanart_hash = ?, updated_at = ? \
         WHERE canonical_name = ? AND fanart_hash IS NOT ? \
           AND EXISTS (SELECT 1 FROM artwork WHERE artwork.hash = ?) \
           AND EXISTS (SELECT 1 FROM metadata_artist \
                       WHERE name_canonical = ? AND fanart_hash = ?) \
         RETURNING id, library_id",
    )
    .bind(hash)
    .bind(now)
    .bind(name_canonical)
    .bind(hash)
    .bind(hash)
    .bind(name_canonical)
    .bind(hash)
    .fetch_all(&mut **tx)
    .await?;
    record_artist_moves(tx, moved, now).await
}

/// Takes the banner `previous` off every artist called `name_canonical` that
/// still shows it: the provider no longer offers it (RFC-014 decision 3). Only
/// that hash — a row already showing a newer one is not this refresh's to
/// clear.
async fn unlink_fanart_in(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    name_canonical: &str,
    previous: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    let moved = sqlx::query(
        "UPDATE artist SET fanart_hash = NULL, updated_at = ? \
         WHERE canonical_name = ? AND fanart_hash = ? \
         RETURNING id, library_id",
    )
    .bind(now)
    .bind(name_canonical)
    .bind(previous)
    .fetch_all(&mut **tx)
    .await?;
    record_artist_moves(tx, moved, now).await
}

/// One artist event per moved row, in the row's own library.
async fn record_artist_moves(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    moved: Vec<sqlx::sqlite::SqliteRow>,
    now: i64,
) -> Result<(), sqlx::Error> {
    for row in moved {
        let id: String = row.try_get("id")?;
        let library_id: String = row.try_get("library_id")?;
        let parse =
            |value: &str| Uuid::parse_str(value).map_err(|error| sqlx::Error::Decode(error.into()));
        crate::catalog::record_library_event(
            tx,
            crate::catalog::LibraryChange {
                library_id: parse(&library_id)?,
                entity_type: "artist",
                entity_id: parse(&id)?,
                action: "upsert",
                payload: serde_json::json!({}),
                changed_at: now,
                origin_device_id: None,
            },
        )
        .await?;
    }
    Ok(())
}
