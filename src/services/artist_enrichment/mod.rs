//! What metadata enrichment says about an artist — RFC-011 decisions 4, 6, 7
//! and 11.
//!
//! **The tenancy lookup comes first and for its refusal.** An artist the caller
//! cannot see answers exactly as one that does not exist, before the cache is
//! read or a provider is asked anything.
//!
//! **The cache is read, never waited on.** A fill is scheduled for every
//! provider with nothing fresh, and the answer goes out with what is there —
//! the first visit to an artist shows nothing, and says it is `pending`.
//!
//! **A neighbour is only ever an artist the caller can see.** The cached list
//! holds names; they resolve through `library_member` with the caller as the
//! first bind, and a name that resolves to nothing is dropped rather than
//! handed out with an id a client could not follow.

use super::*;
use crate::remote::enrichment::{CachedArtist, Outcome, Provider};

/// How many neighbours the native artist page carries.
pub const ARTIST_PAGE_SIMILAR: usize = 20;

impl DomainServices {
    /// The native artist page: [`Self::artist`] with its enrichment.
    pub async fn artist_page(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
    ) -> Result<ArtistDetail, ServiceError> {
        let mut detail = self.artist(user_id, artist_id).await?;
        // After `artist`, whose lookup is the tenancy check: an artist this
        // account cannot see has answered 404 before anything here runs.
        detail.appears_on = Some(self.artist_appearances(user_id, artist_id).await?);
        detail.enrichment = Some(
            self.enrichment_of(user_id, artist_id, &detail.artist.name, ARTIST_PAGE_SIMILAR)
                .await?,
        );
        Ok(detail)
    }

    /// One artist's enrichment, with at most `similar_limit` neighbours.
    pub async fn artist_enrichment(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
        similar_limit: usize,
    ) -> Result<ArtistEnrichment, ServiceError> {
        let summary = self.artist_summary(user_id, artist_id).await?;
        self.enrichment_of(user_id, artist_id, &summary.artist.name, similar_limit)
            .await
    }

    /// The part after the tenancy lookup, which both entry points have done.
    async fn enrichment_of(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
        name: &str,
        similar_limit: usize,
    ) -> Result<ArtistEnrichment, ServiceError> {
        // Cloned out so no lock is held across the reads below; the clone is
        // an `Arc`.
        let engine = self
            .enrichment
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        if !engine.is_enabled() {
            return Ok(ArtistEnrichment::disabled());
        }
        let rows = engine.artist(name).await?;
        let canonical = waveflow_core::scanner::canonical_name(name);
        let similar = self
            .neighbours(user_id, &rows, &canonical, similar_limit)
            .await?;
        let bio = merged_bio(&rows);
        let image = merged_image(&rows);
        // Credited only while it is the banner shown (RFC-014 decision 4): a
        // banner chosen by hand came from no provider, whichever entry point
        // asks.
        let chosen_banner: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM artist_image_choice              WHERE artist_id=? AND kind='banner')",
        )
        .bind(artist_id.to_string())
        .fetch_one(self.db.pool())
        .await?;
        let banner = if chosen_banner {
            None
        } else {
            merged_banner(&rows)
        };
        // Only a fresh row is an answer. An expired one has just had its
        // refresh scheduled, and an expired miss is not "nobody knows" but
        // "being asked again" — `pending`, as decision 11 defines it. Stale
        // content still shows, and `ready` wins over both.
        let now = crate::auth::authentication::now_ms();
        let answered = rows.iter().filter(|row| row.expires_at > now).count();
        let state = state_of(
            bio.is_some() || image.is_some() || banner.is_some() || similar.is_some(),
            answered,
            engine.provider_count(),
        );
        Ok(ArtistEnrichment {
            state,
            bio,
            image,
            banner,
            similar,
        })
    }

    /// The neighbours from the first provider, in the operator's order, whose
    /// list names an artist the caller can see.
    async fn neighbours(
        &self,
        user_id: Uuid,
        rows: &[CachedArtist],
        canonical: &str,
        limit: usize,
    ) -> Result<Option<EnrichedSimilar>, ServiceError> {
        for row in rows.iter().filter(|row| row.outcome == Outcome::Hit) {
            let artists = self
                .visible_similar(user_id, row.provider, canonical, limit)
                .await?;
            if !artists.is_empty() {
                return Ok(Some(EnrichedSimilar {
                    artists,
                    source: source_of(row),
                }));
            }
        }
        Ok(None)
    }

    /// `getSimilarSongs` and `getSimilarSongs2`: a random draw of at most
    /// `limit` tracks from the artist behind `id` and from their neighbours.
    ///
    /// `id` names an artist, an album or a track; an album stands for its
    /// album artist and a track for its track artist. It is resolved first and
    /// for its refusal, so an id the caller cannot reach answers as one naming
    /// nothing, before the cache is read. An album or a track credited to no
    /// artist has no one to be similar to, and draws nothing.
    ///
    /// **The artist is in the draw**, not the neighbours alone: the
    /// specification's "songs from the given artist and similar artists".
    /// Every name draws through [`artist_credit_clause!`] by canonical name,
    /// so an artist the caller sees in two libraries contributes both, and
    /// through `song_select!`, whose membership bind is what keeps a track in
    /// a library the caller is not in out of the draw.
    ///
    /// **The draw is balanced across names**, not over one pool of tracks:
    /// with one pool, an artist with two hundred tracks beside neighbours
    /// with ten each fills most of the answer with their own songs, which is
    /// not what a client asking for similar songs means. Names take turns,
    /// one track each at random, and a name that runs out leaves its turns
    /// to the others, so `limit` is still reached while tracks remain.
    ///
    /// **The neighbours come from a configured provider first**, and from the
    /// catalogue when it names none the caller can see — no provider, the
    /// default; one that has not answered yet; or one that knows nobody here
    /// (RFC-024 decision 8, [`Self::local_neighbours`]). A seed with no
    /// neighbour of either kind draws from its own tracks alone. Until
    /// RFC-024 the default was the empty container.
    pub async fn similar_songs(
        &self,
        user_id: Uuid,
        id: Uuid,
        limit: i64,
    ) -> Result<Vec<SongItem>, ServiceError> {
        if !(0..=MAX_BROWSE_LIMIT).contains(&limit) {
            return Err(ServiceError::Invalid);
        }
        let Some(artist_id) = self.artist_behind(user_id, id).await? else {
            return Ok(Vec::new());
        };
        let artist = self.artist_summary(user_id, artist_id).await?;
        let engine = self
            .enrichment
            .read()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        if limit == 0 {
            return Ok(Vec::new());
        }
        let canonical = waveflow_core::scanner::canonical_name(&artist.artist.name);
        let mut names = vec![canonical.clone()];
        // A configured provider's neighbours first (RFC-011 decision 7)...
        if engine.is_enabled() {
            let rows = engine.artist(&artist.artist.name).await?;
            if let Some(similar) = self
                .neighbours(user_id, &rows, &canonical, usize::MAX)
                .await?
            {
                names.extend(similar.artists.iter().map(|neighbour| {
                    waveflow_core::scanner::canonical_name(&neighbour.artist.name)
                }));
            }
        }
        let mut connection = self.db.pool().acquire().await?;
        // ...and the catalogue's own when there are none: no provider, one
        // that has not answered yet, or one that knows nobody the caller can
        // see (RFC-024 decision 8).
        if names.len() == 1 {
            names.extend(
                self.local_neighbours(
                    &mut connection,
                    user_id,
                    &canonical,
                    super::local_similar::LOCAL_NEIGHBOURS,
                )
                .await?,
            );
        }
        let names = serde_json::to_string(&names).map_err(|_| ServiceError::Invalid)?;
        // No library narrowing: every library the caller reaches.
        let folders = folder_filter(&[]);
        // Innermost, the pool: every visible track credited to one of the
        // names. Each is filed under the name that put it there — its first
        // track artist among the names, else its album's first album artist
        // among them — and ranked at random inside it. Taking rank 1 of every
        // name, then rank 2, and so on, is the round robin; the outermost
        // shuffle keeps the answer from playing name by name. The filing
        // joins come after the projection in the text, so the user id stays
        // the first bind; each picks one row per track or album through
        // SQLite's bare column beside `MIN`, which is the row holding it.
        let mut songs = sqlx::query(concat!(
            "SELECT * FROM (SELECT * FROM (SELECT s.*, ROW_NUMBER() OVER (\
               PARTITION BY COALESCE(tn.name, an.name) \
               ORDER BY RANDOM()) AS draw_rank FROM (",
            song_select!(),
            song_folder_clause!(),
            artist_credit_clause!(
                "SELECT a.id FROM artist a \
                 WHERE a.canonical_name IN (SELECT value FROM json_each(?))"
            ),
            ") AS s \
             LEFT JOIN (SELECT tp.track_id, a.canonical_name AS name, MIN(tp.position) \
                FROM track_participant tp JOIN artist a ON a.id=tp.artist_id \
               WHERE tp.role='artist' \
                 AND a.canonical_name IN (SELECT value FROM json_each(?)) \
               GROUP BY tp.track_id) tn ON tn.track_id=s.id \
             LEFT JOIN (SELECT ap.album_id, a.canonical_name AS name, MIN(ap.position) \
                FROM album_participant ap JOIN artist a ON a.id=ap.artist_id \
               WHERE ap.role='albumartist' \
                 AND a.canonical_name IN (SELECT value FROM json_each(?)) \
               GROUP BY ap.album_id) an ON an.album_id=s.album_id) \
             ORDER BY draw_rank, RANDOM() LIMIT ?) ORDER BY RANDOM()"
        ))
        .bind(user_id.to_string())
        .bind(folders.as_deref())
        .bind(folders.as_deref())
        .bind(&names)
        .bind(&names)
        .bind(&names)
        .bind(&names)
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        attach_song_relations(&mut connection, user_id, &mut songs).await?;
        Ok(songs)
    }

    /// The artist an artist, album or track id stands for, if the caller can
    /// reach it: `NotFound` when nothing reachable has that id, `None` when an
    /// album or a track has it but credits no artist.
    async fn artist_behind(&self, user_id: Uuid, id: Uuid) -> Result<Option<Uuid>, ServiceError> {
        match self.artist_summary(user_id, id).await {
            Err(ServiceError::NotFound) => {}
            found => return found.map(|_| Some(id)),
        }
        for query in [
            "SELECT al.album_artist_id FROM album al \
             JOIN library_member m ON m.library_id=al.library_id \
             WHERE m.user_id=? AND al.id=?",
            "SELECT (SELECT tp.artist_id FROM track_participant tp \
                     WHERE tp.track_id=t.id AND tp.role='artist' \
                     ORDER BY tp.position LIMIT 1) \
             FROM track t JOIN library_member m ON m.library_id=t.library_id \
             WHERE m.user_id=? AND t.id=?",
        ] {
            let found = sqlx::query_scalar::<_, Option<String>>(query)
                .bind(user_id.to_string())
                .bind(id.to_string())
                .fetch_optional(self.db.pool())
                .await?;
            if let Some(artist) = found {
                return artist
                    .map(|artist| {
                        Uuid::parse_str(&artist).map_err(|error| {
                            ServiceError::Database(sqlx::Error::Decode(error.into()))
                        })
                    })
                    .transpose();
            }
        }
        Err(ServiceError::NotFound)
    }

    /// One provider's cached neighbours for `canonical`, resolved to artists
    /// `user_id` can see, best first, at most `limit`.
    ///
    /// Artist rows are per library, so one name may resolve to several — a
    /// musician in two libraries the caller belongs to. One stands for them:
    /// the one with the most albums, then the lowest id, so the choice is
    /// stable from one request to the next.
    async fn visible_similar(
        &self,
        user_id: Uuid,
        provider: Provider,
        canonical: &str,
        limit: usize,
    ) -> Result<Vec<ArtistSummary>, ServiceError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let ranked = self.db.metadata_artist_similar(provider, canonical).await?;
        let rows = sqlx::query(concat!(
            artist_select!(album_count),
            " AND ar.canonical_name IN (SELECT s.similar_canonical FROM metadata_artist_similar s \
               WHERE s.provider=? AND s.name_canonical=?) \
              ORDER BY album_count DESC, ar.id"
        ))
        .bind(user_id.to_string())
        .bind(provider.as_str())
        .bind(canonical)
        .fetch_all(self.db.pool())
        .await?;
        let mut by_name: std::collections::HashMap<String, ArtistSummary> =
            std::collections::HashMap::new();
        for row in rows {
            let summary = artist_summary_from_row(row)?;
            // The column the query matched on is this function of the name,
            // written together with it (`catalog::upsert_artist`), so it is
            // recomputed rather than projected.
            by_name
                .entry(waveflow_core::scanner::canonical_name(&summary.artist.name))
                .or_insert(summary);
        }
        Ok(ranked
            .into_iter()
            .filter_map(|neighbour| by_name.remove(&neighbour.name_canonical))
            .take(limit)
            .collect())
    }
}

/// The biography: the first row, in the operator's order, that has one. A
/// provider that knows nothing does not hide the next one's.
fn merged_bio(rows: &[CachedArtist]) -> Option<EnrichedBio> {
    rows.iter()
        .find(|row| row.bio_summary.is_some() || row.bio_full.is_some())
        .map(|row| EnrichedBio {
            summary: row.bio_summary.clone(),
            full: row.bio_full.clone(),
            source: source_of(row),
        })
}

fn merged_image(rows: &[CachedArtist]) -> Option<EnrichedImage> {
    rows.iter().find_map(|row| {
        row.image_hash.as_ref().map(|hash| EnrichedImage {
            artwork_hash: hash.clone(),
            source: source_of(row),
        })
    })
}

/// The first configured provider with a banner supplies it (RFC-014).
fn merged_banner(rows: &[CachedArtist]) -> Option<EnrichedImage> {
    rows.iter().find_map(|row| {
        row.fanart_hash.as_ref().map(|hash| EnrichedImage {
            artwork_hash: hash.clone(),
            source: source_of(row),
        })
    })
}

/// Content wins over everything; without it, a provider that has not
/// answered makes the lookup `pending`, and only when every one of them has
/// answered is it `none`.
fn state_of(has_content: bool, answered: usize, configured: usize) -> EnrichmentState {
    if has_content {
        EnrichmentState::Ready
    } else if answered < configured {
        EnrichmentState::Pending
    } else {
        EnrichmentState::None
    }
}

fn source_of(row: &CachedArtist) -> EnrichmentSource {
    EnrichmentSource {
        provider: row.provider.as_str().to_owned(),
        fetched_at: row.fetched_at,
    }
}

#[cfg(test)]
mod tests;
