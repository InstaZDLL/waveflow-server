//! What the cache holds and what a fill brings back: the providers, their
//! outcomes, the cached rows, and the portraits and candidates on their way to
//! the artwork store.

/// A provider an operator can name in `WAVEFLOW_METADATA_PROVIDERS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Provider {
    LastFm,
    /// Portraits only: Deezer has no biography, and its related artists are
    /// not asked for.
    Deezer,
    /// Banners only (RFC-014): TheAudioDB's fanart. Its biographies wait on a
    /// language this server does not know.
    TheAudioDb,
}

impl Provider {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "lastfm" => Some(Self::LastFm),
            "deezer" => Some(Self::Deezer),
            "theaudiodb" => Some(Self::TheAudioDb),
            _ => None,
        }
    }

    /// The name in the setting, in the cache's `provider` column, and in logs.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LastFm => "lastfm",
            Self::Deezer => "deezer",
            Self::TheAudioDb => "theaudiodb",
        }
    }
}

/// What a provider said, when it said something. Unreached has no variant: it
/// is the absence of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Hit,
    Miss,
}

impl Outcome {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hit" => Some(Self::Hit),
            "miss" => Some(Self::Miss),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::Miss => "miss",
        }
    }
}

/// One provider's cached row for one name.
#[derive(Debug, Clone)]
pub struct CachedArtist {
    pub provider: Provider,
    pub outcome: Outcome,
    pub bio_summary: Option<String>,
    pub bio_full: Option<String>,
    /// The portrait this provider supplied, in the artwork store. Cleared by
    /// the database when the sweeper takes the artwork row.
    pub image_hash: Option<String>,
    /// The banner this provider supplied (RFC-014), cleared the same way.
    pub fanart_hash: Option<String>,
    pub fetched_at: i64,
    pub expires_at: i64,
}

/// A portrait already in the artwork store, waiting for its row.
#[derive(Debug)]
pub(crate) struct StoredPortrait {
    pub(crate) hash: String,
    pub(crate) format: &'static str,
    pub(crate) byte_size: i64,
}

/// A Deezer artist offered as a portrait (RFC-013 decision 4), its preview
/// already in the artwork store. The row naming the preview is the caller's to
/// write, with whatever names it.
#[derive(Debug)]
pub(crate) struct DeezerCandidate {
    pub(crate) deezer_id: i64,
    pub(crate) name: String,
    pub(crate) fans: Option<i64>,
    pub(crate) preview: StoredPortrait,
}

/// Why a search for candidates gave none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateSearchError {
    /// Deezer is not among the operator's providers, or images are off.
    NotConfigured,
    /// Deezer, or a picture host it named, could not be reached, or a preview
    /// could not be stored.
    Unreached,
}

/// One neighbour in a provider's cached list.
#[derive(Debug, Clone)]
pub struct CachedSimilar {
    pub name: String,
    pub name_canonical: String,
    pub score: f64,
}

/// What one fill brought back from one provider, before it is written.
#[derive(Debug, Default)]
pub(crate) struct ArtistAnswer {
    pub(crate) bio_summary: Option<String>,
    pub(crate) bio_full: Option<String>,
    pub(crate) similar: Vec<CachedSimilar>,
    pub(crate) image: Option<StoredPortrait>,
    /// The banner, from TheAudioDB's fanart (RFC-014).
    pub(crate) fanart: Option<StoredPortrait>,
}

impl ArtistAnswer {
    /// A hit carries something; an answer with nothing in it is a miss.
    pub(crate) fn outcome(&self) -> Outcome {
        if self.bio_summary.is_some()
            || self.bio_full.is_some()
            || !self.similar.is_empty()
            || self.image.is_some()
            || self.fanart.is_some()
        {
            Outcome::Hit
        } else {
            Outcome::Miss
        }
    }
}
