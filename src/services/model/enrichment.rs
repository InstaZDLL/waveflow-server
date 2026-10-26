//! What metadata enrichment and the hand-chosen images say about an artist
//! (RFC-011, RFC-013).

use serde::Serialize;
use utoipa::ToSchema;

use super::catalog::ArtistSummary;

/// The hand-chosen images of one artist (RFC-013).
#[derive(Debug, Clone, Default, Serialize, ToSchema)]
pub struct ImageChoices {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portrait: Option<ImageChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub banner: Option<ImageChoice>,
}

impl ImageChoices {
    pub fn is_empty(&self) -> bool {
        self.portrait.is_none() && self.banner.is_none()
    }
}

/// Where a chosen image came from and when.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImageChoice {
    /// `upload` or `deezer`.
    pub source: String,
    pub set_at: i64,
}

/// What enrichment holds about one artist, merged across providers in the
/// operator's order, field by field (RFC-011 decisions 4 and 11).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtistEnrichment {
    pub state: EnrichmentState,
    /// The first configured provider with a biography supplies it.
    pub bio: Option<EnrichedBio>,
    /// The first configured provider with a portrait supplies it.
    pub image: Option<EnrichedImage>,
    /// The automatic banner and where it came from (RFC-014 decision 4).
    /// Present only while it is the banner shown: a banner chosen by hand
    /// came from no provider, and this must not say otherwise.
    pub banner: Option<EnrichedImage>,
    /// The first configured provider whose neighbours include artists the
    /// caller can see supplies the list — those artists only, best first.
    pub similar: Option<EnrichedSimilar>,
}

impl ArtistEnrichment {
    pub(crate) fn disabled() -> Self {
        Self {
            state: EnrichmentState::Disabled,
            bio: None,
            image: None,
            banner: None,
            similar: None,
        }
    }
}

/// Four situations that absence alone cannot tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EnrichmentState {
    /// No provider is configured: nothing has been asked and nothing will be.
    Disabled,
    /// A provider has not answered yet — scheduled, in flight, or unreached
    /// last time, which writes nothing and is retried on the next request.
    Pending,
    /// There is something to show.
    Ready,
    /// Every configured provider answered, and none of them has anything this
    /// caller can be shown.
    None,
}

/// Where one field came from.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnrichmentSource {
    /// The provider's name as `WAVEFLOW_METADATA_PROVIDERS` spells it.
    pub provider: String,
    /// When the provider gave this answer, in epoch milliseconds.
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnrichedBio {
    pub summary: Option<String>,
    pub full: Option<String>,
    pub source: EnrichmentSource,
}

/// A portrait or a banner, as a key into the artwork store.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnrichedImage {
    /// Served by the artwork route, as every `artwork_hash` is.
    pub artwork_hash: String,
    pub source: EnrichmentSource,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EnrichedSimilar {
    pub artists: Vec<ArtistSummary>,
    pub source: EnrichmentSource,
}
