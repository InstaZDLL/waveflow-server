//! Podcasts, and the remote artwork that podcast covers and station images
//! are fetched under (RFC-025).

use std::{path::PathBuf, time::Duration};

use super::env::{parse_bool_env, parse_env, parse_positive_env};

/// Podcasts (RFC-025 decisions 7 to 12).
///
/// **Off by default, and an upgrade leaves it off**: subscribing to a feed is
/// outbound traffic to a host a person typed, and nothing of the kind happens
/// until an operator says so. With the switch off, no feed is fetched, no
/// episode is downloaded, and `podcastRole` is false for everyone; what was
/// stored while it was on stays readable, and a downloaded episode stays
/// playable from disk.
#[derive(Debug, Clone)]
pub struct PodcastSettings {
    /// `WAVEFLOW_PODCASTS`, off by default.
    pub enabled: bool,
    /// `WAVEFLOW_PODCAST_REFRESH_INTERVAL_SECS`: how often every channel is
    /// fetched again, six hours by default; `0` for never on its own — a
    /// refresh then happens only when somebody asks for one.
    pub refresh_interval: Option<Duration>,
    /// `WAVEFLOW_PODCAST_ALLOW_HTTP`: whether a feed or an enclosure may be
    /// plain http. On by default, because a good share of podcast enclosures
    /// are still served that way and a feed is public content; the address
    /// rules apply either way.
    pub allow_http: bool,
    /// `WAVEFLOW_PODCAST_MAX_FEED_BYTES`: the largest feed read, 16 MiB.
    pub max_feed_bytes: usize,
    /// `WAVEFLOW_PODCAST_MAX_EPISODE_BYTES`: the largest episode stored,
    /// 1 GiB.
    pub max_episode_bytes: u64,
    /// `WAVEFLOW_PODCAST_QUOTA_BYTES`: how much the downloaded episodes may
    /// occupy together, 20 GiB.
    pub quota_bytes: u64,
    /// `WAVEFLOW_PODCAST_FEED_TIMEOUT_SECS`: one feed fetch, redirects
    /// included, 20 seconds.
    pub feed_timeout: Duration,
    /// `WAVEFLOW_PODCAST_DOWNLOAD_TIMEOUT_SECS`: one episode download, 30
    /// minutes.
    pub download_timeout: Duration,
    /// Where episodes are stored: `<data dir>/podcasts`, derived and not set.
    pub dir: PathBuf,
    /// Which addresses a fetch may reach. Public ones in production, always;
    /// not an environment variable. The test suite's feeds live on loopback,
    /// and its fixture admits loopback for them alone.
    pub addresses: crate::remote::portrait::AddressPolicy,
}

/// Remote artwork: the cover a podcast feed names, and the image an account
/// gives a radio station (RFC-025, amended).
///
/// **On by default**, unlike podcasts, because nothing here happens on its own:
/// a station image is fetched once when an account sets or changes one, and a
/// podcast cover only while `WAVEFLOW_PODCASTS` is on and a channel is
/// subscribed or refreshed. Off, an address is still stored and nothing is
/// fetched; covers already stored stay shown.
#[derive(Debug, Clone, Copy)]
pub struct RemoteArtworkSettings {
    /// `WAVEFLOW_REMOTE_ARTWORK`, on by default.
    pub enabled: bool,
    /// One image fetch, redirects included. Derived, not set: the figure the
    /// feed fetch defaults to, for a body the size of a cover.
    pub timeout: Duration,
    /// Which addresses a fetch may reach, as for podcasts: public ones in
    /// production, always, and loopback as well for the test suite's hosts.
    pub addresses: crate::remote::portrait::AddressPolicy,
}

/// The podcast switch and its bounds (RFC-025 decision 7).
///
/// Every bound refuses zero at startup, as the other bounds here do; the
/// interval is the exception, because zero is how "never on its own" is said.
pub(super) fn parse_podcast_settings(
    data_dir: &std::path::Path,
) -> anyhow::Result<PodcastSettings> {
    let refresh_secs = parse_env::<u64>("WAVEFLOW_PODCAST_REFRESH_INTERVAL_SECS", "21600")?;
    let max_episode_bytes = parse_positive_env("WAVEFLOW_PODCAST_MAX_EPISODE_BYTES", 1u64 << 30)?;
    let quota_bytes = parse_positive_env("WAVEFLOW_PODCAST_QUOTA_BYTES", 20u64 << 30)?;
    if max_episode_bytes > quota_bytes {
        anyhow::bail!(
            "WAVEFLOW_PODCAST_MAX_EPISODE_BYTES cannot exceed WAVEFLOW_PODCAST_QUOTA_BYTES"
        );
    }
    Ok(PodcastSettings {
        enabled: parse_bool_env("WAVEFLOW_PODCASTS", false)?,
        // Never more often than every five minutes: a refresh fetches every
        // channel, and an interval of one second would be a crawler.
        refresh_interval: (refresh_secs > 0).then(|| Duration::from_secs(refresh_secs.max(300))),
        allow_http: parse_bool_env("WAVEFLOW_PODCAST_ALLOW_HTTP", true)?,
        max_feed_bytes: parse_positive_env("WAVEFLOW_PODCAST_MAX_FEED_BYTES", 16usize << 20)?,
        max_episode_bytes,
        quota_bytes,
        feed_timeout: Duration::from_secs(parse_positive_env(
            "WAVEFLOW_PODCAST_FEED_TIMEOUT_SECS",
            20u64,
        )?),
        download_timeout: Duration::from_secs(parse_positive_env(
            "WAVEFLOW_PODCAST_DOWNLOAD_TIMEOUT_SECS",
            1_800u64,
        )?),
        dir: data_dir.join("podcasts"),
        addresses: crate::remote::portrait::AddressPolicy::Public,
    })
}
