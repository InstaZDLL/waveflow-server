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
use crate::enrichment::{CachedArtist, Outcome, Provider};

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
        detail.enrichment = Some(
            self.enrichment_of(user_id, &detail.artist.name, ARTIST_PAGE_SIMILAR)
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
        self.enrichment_of(user_id, &summary.artist.name, similar_limit)
            .await
    }

    /// The part after the tenancy lookup, which both entry points have done.
    async fn enrichment_of(
        &self,
        user_id: Uuid,
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
        let mut similar = None;
        for row in rows.iter().filter(|row| row.outcome == Outcome::Hit) {
            let artists = self
                .visible_similar(user_id, row.provider, &canonical, similar_limit)
                .await?;
            if !artists.is_empty() {
                similar = Some(EnrichedSimilar {
                    artists,
                    source: source_of(row),
                });
                break;
            }
        }
        let bio = merged_bio(&rows);
        let state = state_of(
            bio.is_some() || similar.is_some(),
            rows.len(),
            engine.provider_count(),
        );
        Ok(ArtistEnrichment {
            state,
            bio,
            similar,
        })
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
mod tests {
    use super::{merged_bio, state_of};
    use crate::enrichment::{CachedArtist, Outcome, Provider};
    use crate::services::EnrichmentState;

    fn row(outcome: Outcome, bio: Option<&str>, fetched_at: i64) -> CachedArtist {
        CachedArtist {
            provider: Provider::LastFm,
            outcome,
            bio_summary: bio.map(str::to_owned),
            bio_full: None,
            fetched_at,
            expires_at: fetched_at + 1,
        }
    }

    /// Decision 4: one provider's "never heard of them" is not the world's.
    /// Only Last.fm exists as a provider today, so the two rows here stand in
    /// for two providers by their position, which is all the merge reads.
    #[test]
    fn a_miss_at_the_first_provider_does_not_hide_the_second_biography() {
        let rows = [
            row(Outcome::Miss, None, 1),
            row(Outcome::Hit, Some("Born in Reykjavík."), 2),
        ];
        let bio = merged_bio(&rows).expect("the second provider's biography");
        assert_eq!(bio.summary.as_deref(), Some("Born in Reykjavík."));
        assert_eq!(bio.source.fetched_at, 2);
    }

    /// Decision 11's table, row by row.
    #[test]
    fn each_state_is_said_when_it_holds() {
        // Something to show, even while another provider is still out.
        assert_eq!(state_of(true, 1, 2), EnrichmentState::Ready);
        // Nothing yet, and a provider has not answered.
        assert_eq!(state_of(false, 0, 1), EnrichmentState::Pending);
        assert_eq!(state_of(false, 1, 2), EnrichmentState::Pending);
        // Everyone answered, nobody had anything.
        assert_eq!(state_of(false, 1, 1), EnrichmentState::None);
    }
}
