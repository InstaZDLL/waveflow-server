//! Metadata enrichment — RFC-011: the cache, and the engine that fills it.
//!
//! **Demand-driven and never on the answering path** (decision 6). A reader asks
//! [`Enrichment::artist`] for what the cache holds; for every configured
//! provider with nothing fresh, that call schedules a fill in the background
//! and returns at once. The first visit to an artist shows nothing; the next
//! one shows what came back.
//!
//! **Three outcomes** (decision 5), written on the provider's own row: a *hit*
//! stands for the long TTL, a *miss* — the provider answered and knows nothing
//! — for the short one, and *unreached* writes nothing at all, so the next
//! visit asks again. A network blip must never become "this artist has no
//! biography" for a month.
//!
//! **Bounded per provider**: requests are spaced to the configured rate, and
//! concurrent first visits for one name collapse onto a single fill. The
//! writer gate is taken for the row write only, after the requests are over.
//!
//! **Nothing about a request is logged but the provider and a verdict.** A
//! Last.fm request carries the operator's key in its query string; neither the
//! URL nor a response body is ever written to a log (decision 9).

use std::time::{Duration, Instant};

mod engine;
mod handle;
mod model;

pub use handle::{Endpoints, Enrichment};
pub(crate) use model::{ArtistAnswer, CandidateSearchError, DeezerCandidate, StoredPortrait};
pub use model::{CachedArtist, CachedSimilar, Outcome, Provider};

/// How many fills may be waiting or running at once, across providers.
///
/// Pacing reserves a slot for every request and refuses none, so without this
/// a page that opens two thousand artists would queue two thousand fills —
/// a quarter of an hour at the default rate — and an outage, which writes no
/// row, would let every further visit add more. Beyond the ceiling nothing is
/// scheduled; the visit after asks again, which is what an unreached lookup
/// does anyway.
pub const MAX_IN_FLIGHT: usize = 64;

/// Spaces one provider's requests to its rate: a token bucket that holds one
/// token, so there is never a burst to spend.
///
/// The reservation is taken under the lock and the wait happens outside it, so
/// callers queue in order without one sleeping caller holding up the rest.
pub(crate) struct Pace {
    interval: Duration,
    next: std::sync::Mutex<Instant>,
}

impl Pace {
    pub(crate) fn new(rate_per_sec: u32) -> Self {
        Self {
            interval: Duration::from_secs(1) / rate_per_sec.max(1),
            next: std::sync::Mutex::new(Instant::now()),
        }
    }

    /// The operator's rate, but never faster than one request per `floor`:
    /// a provider's own ceiling, which the shared setting cannot raise.
    fn with_floor(rate_per_sec: u32, floor: Duration) -> Self {
        let pace = Self::new(rate_per_sec);
        Self {
            interval: pace.interval.max(floor),
            ..pace
        }
    }

    pub(crate) async fn wait(&self) {
        let slot = {
            let mut next = self
                .next
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let slot = (*next).max(Instant::now());
            *next = slot + self.interval;
            slot
        };
        tokio::time::sleep_until(slot.into()).await;
    }
}
