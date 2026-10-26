//! Outbound lookups about the catalogue, each off until the operator names a
//! provider: artist enrichment (RFC-011) and lyrics (RFC-024).

use std::time::Duration;

use super::env::{parse_bool_env, parse_positive_env, read_trimmed_env};

/// Metadata enrichment, as RFC-011 decision 3 lists its settings.
///
/// **Off unless `providers` names one**, and that is the only switch: an empty
/// list makes no outbound request of any kind, and nothing else here can
/// contradict it. Separate from scrobbling on purpose — an operator who placed
/// a Last.fm application for submissions has not thereby agreed to lookups
/// about the whole catalogue, so `lastfm_key` is its own setting even when it
/// holds the same value.
#[derive(Clone)]
pub struct MetadataSettings {
    /// `WAVEFLOW_METADATA_PROVIDERS`: an ordered, comma-separated list, empty
    /// by default, of `lastfm`, `deezer` and `theaudiodb`. The order is the
    /// order fields are merged in when read.
    pub providers: Vec<crate::enrichment::Provider>,
    /// `WAVEFLOW_METADATA_IMAGES`, on by default, `off` to keep the lookups and
    /// refuse decision 8's image fetch. A narrowing only: it switches nothing
    /// on while `providers` is empty.
    pub images: bool,
    /// `WAVEFLOW_METADATA_LASTFM_KEY`, the operator's own application key.
    /// Required when `providers` names `lastfm`: no key ships in this binary.
    pub lastfm_key: Option<String>,
    /// `WAVEFLOW_METADATA_THEAUDIODB_KEY`, the operator's own TheAudioDB key.
    /// Required when `providers` names `theaudiodb` (RFC-014 decision 1): the
    /// public test key `waveflow-core` uses for one desktop user is not one a
    /// server may share with everyone else's.
    pub theaudiodb_key: Option<String>,
    /// `WAVEFLOW_METADATA_TTL_DAYS`: how long an answer with content stands.
    pub ttl: Duration,
    /// `WAVEFLOW_METADATA_MISS_TTL_DAYS`: how long "the provider knows nothing"
    /// stands. Shorter than `ttl`, and a setting of its own, because it is the
    /// answer most likely to change.
    pub miss_ttl: Duration,
    /// `WAVEFLOW_METADATA_RATE_PER_SEC`: requests per second, per provider.
    pub rate_per_sec: u32,
    /// `WAVEFLOW_METADATA_REQUEST_TIMEOUT_SECS`: how long one request to a
    /// provider may take before it counts as unreached.
    pub request_timeout: Duration,
}

impl MetadataSettings {
    /// Enrichment switched off — what a server that is merely upgraded runs.
    pub fn off() -> Self {
        Self {
            providers: Vec::new(),
            images: true,
            lastfm_key: None,
            theaudiodb_key: None,
            ttl: Duration::from_secs(30 * 24 * 60 * 60),
            miss_ttl: Duration::from_secs(7 * 24 * 60 * 60),
            rate_per_sec: 4,
            request_timeout: Duration::from_secs(10),
        }
    }
}

/// Written by hand so the key never reaches a log: a `Config` is printed at
/// startup under `RUST_LOG=debug`, and a Last.fm key travels in a query string
/// where anyone holding it can spend the operator's quota.
impl std::fmt::Debug for MetadataSettings {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MetadataSettings")
            .field("providers", &self.providers)
            .field("images", &self.images)
            .field(
                "lastfm_key",
                &self.lastfm_key.as_ref().map(|_| "[redacted]"),
            )
            .field(
                "theaudiodb_key",
                &self.theaudiodb_key.as_ref().map(|_| "[redacted]"),
            )
            .field("ttl", &self.ttl)
            .field("miss_ttl", &self.miss_ttl)
            .field("rate_per_sec", &self.rate_per_sec)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// Lyrics looked up on LRCLIB, as RFC-024 decision 1 lists its settings.
///
/// **Off unless `providers` names one**, and that is the only switch, exactly
/// as for [`MetadataSettings`]. A separate list rather than a value of
/// `WAVEFLOW_METADATA_PROVIDERS`, because a lyrics lookup sends a track title,
/// its album and its duration at the moment somebody plays it, which is a
/// different consent from asking about an artist's name.
#[derive(Debug, Clone)]
pub struct LyricsSettings {
    /// `WAVEFLOW_LYRICS_PROVIDERS`: empty by default, or `lrclib`.
    pub providers: Vec<crate::lrclib::LyricsProvider>,
    /// `WAVEFLOW_LYRICS_TTL_DAYS`: how long lyrics found are kept.
    pub ttl: Duration,
    /// `WAVEFLOW_LYRICS_MISS_TTL_DAYS`: how long "nothing for this track" is
    /// kept.
    pub miss_ttl: Duration,
    /// `WAVEFLOW_LYRICS_RATE_PER_SEC`: requests per second.
    pub rate_per_sec: u32,
    /// `WAVEFLOW_LYRICS_REQUEST_TIMEOUT_SECS`: how long one request may take
    /// before it counts as unanswered.
    pub request_timeout: Duration,
}

impl LyricsSettings {
    /// Lookups switched off — what a server that is merely upgraded runs.
    pub fn off() -> Self {
        Self {
            providers: Vec::new(),
            ttl: Duration::from_secs(30 * 24 * 60 * 60),
            miss_ttl: Duration::from_secs(7 * 24 * 60 * 60),
            rate_per_sec: 2,
            request_timeout: Duration::from_secs(10),
        }
    }
}

/// Reads RFC-011 decision 3's settings.
pub(super) fn parse_metadata_settings() -> anyhow::Result<MetadataSettings> {
    let defaults = MetadataSettings::off();
    let lastfm_key = read_trimmed_env("WAVEFLOW_METADATA_LASTFM_KEY");
    let theaudiodb_key = read_trimmed_env("WAVEFLOW_METADATA_THEAUDIODB_KEY");
    let providers = parse_metadata_providers(
        &std::env::var("WAVEFLOW_METADATA_PROVIDERS").unwrap_or_default(),
        lastfm_key.is_some(),
        theaudiodb_key.is_some(),
    )?;
    let days = |name: &str, default: Duration| -> anyhow::Result<Duration> {
        let days: u64 = parse_positive_env(name, default.as_secs() / (24 * 60 * 60))?;
        days.checked_mul(24 * 60 * 60)
            .map(Duration::from_secs)
            .ok_or_else(|| anyhow::anyhow!("invalid {name}: too large"))
    };
    Ok(MetadataSettings {
        providers,
        images: parse_bool_env("WAVEFLOW_METADATA_IMAGES", defaults.images)?,
        lastfm_key,
        theaudiodb_key,
        ttl: days("WAVEFLOW_METADATA_TTL_DAYS", defaults.ttl)?,
        miss_ttl: days("WAVEFLOW_METADATA_MISS_TTL_DAYS", defaults.miss_ttl)?,
        rate_per_sec: parse_positive_env("WAVEFLOW_METADATA_RATE_PER_SEC", defaults.rate_per_sec)?,
        request_timeout: Duration::from_secs(parse_positive_env(
            "WAVEFLOW_METADATA_REQUEST_TIMEOUT_SECS",
            defaults.request_timeout.as_secs(),
        )?),
    })
}

/// Reads RFC-024 decision 1's settings.
pub(super) fn parse_lyrics_settings() -> anyhow::Result<LyricsSettings> {
    let defaults = LyricsSettings::off();
    let providers =
        parse_lyrics_providers(&std::env::var("WAVEFLOW_LYRICS_PROVIDERS").unwrap_or_default())?;
    let days = |name: &str, default: Duration| -> anyhow::Result<Duration> {
        let days: u64 = parse_positive_env(name, default.as_secs() / (24 * 60 * 60))?;
        days.checked_mul(24 * 60 * 60)
            .map(Duration::from_secs)
            .ok_or_else(|| anyhow::anyhow!("invalid {name}: too large"))
    };
    Ok(LyricsSettings {
        providers,
        ttl: days("WAVEFLOW_LYRICS_TTL_DAYS", defaults.ttl)?,
        miss_ttl: days("WAVEFLOW_LYRICS_MISS_TTL_DAYS", defaults.miss_ttl)?,
        rate_per_sec: parse_positive_env("WAVEFLOW_LYRICS_RATE_PER_SEC", defaults.rate_per_sec)?,
        request_timeout: Duration::from_secs(parse_positive_env(
            "WAVEFLOW_LYRICS_REQUEST_TIMEOUT_SECS",
            defaults.request_timeout.as_secs(),
        )?),
    })
}

/// `WAVEFLOW_LYRICS_PROVIDERS`. An unknown name and a name given twice refuse
/// to boot, for the reason [`parse_metadata_providers`] gives.
pub(super) fn parse_lyrics_providers(
    list: &str,
) -> anyhow::Result<Vec<crate::lrclib::LyricsProvider>> {
    let mut providers = Vec::new();
    for name in list
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let provider = crate::lrclib::LyricsProvider::parse(name)
            .ok_or_else(|| anyhow::anyhow!("invalid WAVEFLOW_LYRICS_PROVIDERS entry {name:?}"))?;
        if providers.contains(&provider) {
            anyhow::bail!("WAVEFLOW_LYRICS_PROVIDERS names {name:?} twice");
        }
        providers.push(provider);
    }
    Ok(providers)
}

/// `WAVEFLOW_METADATA_PROVIDERS`, in the operator's order.
///
/// A provider named without its key, an unknown name and a name given twice
/// all refuse to boot. Each is a typo, and a server that started with
/// enrichment quietly narrower than its operator wrote is the silent failure
/// these settings must not have — the operator learns it here, not from an
/// artist page that never fills.
pub(super) fn parse_metadata_providers(
    list: &str,
    has_lastfm_key: bool,
    has_theaudiodb_key: bool,
) -> anyhow::Result<Vec<crate::enrichment::Provider>> {
    let mut providers = Vec::new();
    for name in list
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let provider = crate::enrichment::Provider::parse(name)
            .ok_or_else(|| anyhow::anyhow!("invalid WAVEFLOW_METADATA_PROVIDERS entry {name:?}"))?;
        if providers.contains(&provider) {
            anyhow::bail!("WAVEFLOW_METADATA_PROVIDERS names {name:?} twice");
        }
        if provider == crate::enrichment::Provider::LastFm && !has_lastfm_key {
            anyhow::bail!(
                "WAVEFLOW_METADATA_PROVIDERS names lastfm without WAVEFLOW_METADATA_LASTFM_KEY"
            );
        }
        if provider == crate::enrichment::Provider::TheAudioDb && !has_theaudiodb_key {
            anyhow::bail!(
                "WAVEFLOW_METADATA_PROVIDERS names theaudiodb without WAVEFLOW_METADATA_THEAUDIODB_KEY"
            );
        }
        providers.push(provider);
    }
    Ok(providers)
}
