use super::{merged_bio, state_of};
use crate::remote::enrichment::{CachedArtist, Outcome, Provider};
use crate::services::EnrichmentState;

fn row(outcome: Outcome, bio: Option<&str>, fetched_at: i64) -> CachedArtist {
    CachedArtist {
        provider: Provider::LastFm,
        outcome,
        bio_summary: bio.map(str::to_owned),
        bio_full: None,
        image_hash: None,
        fanart_hash: None,
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
