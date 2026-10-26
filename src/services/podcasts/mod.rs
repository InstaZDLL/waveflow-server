//! Podcasts: channels, episodes, the files fetched for them, and where each
//! account stopped (RFC-025 decisions 7 to 14).
//!
//! **One list per instance.** An administrator subscribes and every account
//! reads; a channel is fetched once, however many people listen to it. Who may
//! subscribe, refresh, download and delete is one predicate,
//! [`DomainServices::can_manage_podcasts`]: the switch is on and the account is
//! an enabled administrator. It is what `podcastRole` reports.
//!
//! **Episodes are downloaded, never relayed.** A request to play an episode
//! reads a file under `<data dir>/podcasts`, through the same media service and
//! transcode cache a track uses; the server never opens a connection on a
//! listener's behalf while they listen. The file is named after the episode's
//! own id and a suffix chosen from its bytes, so nothing a feed says can name a
//! path, and the downloads share a quota.
//!
//! **Network I/O never holds the writer gate.** A fetch runs first; the gate is
//! taken for the rows it produced, after.
//!
//! **A channel's cover is fetched by the server** (RFC-025, amended), when the
//! channel is subscribed and on a refresh that finds the feed naming a cover
//! it does not hold — a new address, a failed last attempt, a file gone from
//! the store, or one older than [`COVER_TTL_MS`]. It is stored in the artwork
//! store and handed out by hash; the address is shown to managers only.

use std::path::Path;

use super::*;
use crate::remote::podcast::{self, FetchPolicy};

/// How many episodes `getNewestPodcasts` answers when it is not told, as the
/// specification has it, and the most it answers.
pub const NEWEST_EPISODES_DEFAULT: i64 = 20;
const NEWEST_EPISODES_MAX: i64 = 500;

/// How many channels one instance may hold. A refresh fetches every one.
const MAX_PODCAST_CHANNELS: i64 = 500;

/// How many episodes a channel lists: the newest, as many as one feed may
/// carry. Episodes are never purged — a refresh adds what a feed newly names
/// and keeps the rest — so a daily show would otherwise grow every answer by a
/// year's worth of rows a year, and `getPodcasts` answers every channel with
/// its episodes by default. An episode past the bound that is stored, or on
/// its way, is listed all the same: it holds a share of the quota, and the
/// listing is where an administrator deletes it.
pub const MAX_EPISODES_LISTED: i64 = podcast::MAX_EPISODES_PER_FEED as i64;

/// A channel, as every surface reads it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PodcastChannelItem {
    pub id: Uuid,
    /// The feed address, to an account that may manage podcasts, and `None`
    /// to everyone else (RFC-025 decision 8). A private feed carries its
    /// subscriber's token in its path as often as in its query —
    /// `/rss/<token>.xml`, `/feed/<user>/podcast.xml` — so no part of the
    /// address past its host is shown to an account that did not add it.
    pub url: Option<String>,
    /// The feed's scheme and host, `https://feeds.example.org/`, and nothing
    /// after: what the Subsonic `url` attribute, which the schema requires,
    /// carries for a reader. Not serialized natively.
    #[serde(skip)]
    pub origin: String,
    pub title: String,
    pub description: Option<String>,
    pub site_url: Option<String>,
    /// The cover address the feed names, to an account that may manage
    /// podcasts, and `None` to everyone else: a private feed's cover can carry
    /// its subscriber's token as readily as the feed. Readers are given
    /// `artwork_hash` instead.
    pub image_url: Option<String>,
    /// The cover, fetched by the server into the artwork store
    /// (`/api/v2/artwork/{hash}`, Subsonic `coverArt`). `None` until a fetch
    /// succeeds, or when the feed names none.
    pub artwork_hash: Option<String>,
    /// `completed` when the last fetch succeeded, `error` when it did not.
    pub status: String,
    /// Why the last fetch failed, as a word from a fixed list.
    pub error: Option<String>,
    pub created_at: i64,
    pub refreshed_at: Option<i64>,
    /// Every episode the channel holds, which may be more than `episodes`
    /// lists: see [`MAX_EPISODES_LISTED`].
    pub episode_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episodes: Option<Vec<PodcastEpisodeItem>>,
}

/// An episode, and the reader's own position in it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PodcastEpisodeItem {
    pub id: Uuid,
    pub channel_id: Uuid,
    pub channel_title: String,
    /// The channel's cover: an episode shows its channel's, and an episode's
    /// own `itunes:image` is not fetched.
    pub artwork_hash: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<i64>,
    pub duration_ms: Option<i64>,
    /// `new`, `downloading`, `completed`, `error` or `deleted`. Only a
    /// `completed` episode can be played.
    pub status: String,
    pub error: Option<String>,
    pub size: Option<i64>,
    pub suffix: Option<String>,
    pub content_type: Option<String>,
    /// Where this account stopped, from its bookmark, in milliseconds.
    pub position_ms: Option<i64>,
    pub created_at: i64,
}

/// The podcasts page in one answer: whether the feature is on, whether this
/// account may change it, and the channels.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PodcastOverview {
    pub enabled: bool,
    pub can_manage: bool,
    pub channels: Vec<PodcastChannelItem>,
}

/// An episode bookmark, as Subsonic's `getBookmarks` lists it beside the
/// track ones.
#[derive(Debug, Clone)]
pub struct EpisodeBookmark {
    pub position_ms: i64,
    pub comment: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub episode: PodcastEpisodeItem,
}

/// Why subscribing did not work.
#[derive(Debug)]
pub enum PodcastFailure {
    Service(ServiceError),
    /// The feed could not be fetched or read, as a word from the fixed list
    /// `src/remote/podcast/` defines. Nothing was stored.
    Feed(String),
}

impl From<ServiceError> for PodcastFailure {
    fn from(error: ServiceError) -> Self {
        Self::Service(error)
    }
}

impl From<sqlx::Error> for PodcastFailure {
    fn from(error: sqlx::Error) -> Self {
        Self::Service(ServiceError::Database(error))
    }
}

/// The runtime half of the podcast settings: the bounds, and the one lock and
/// one permit the background work shares.
#[derive(Clone)]
pub(super) struct PodcastRuntime {
    pub(super) settings: crate::config::PodcastSettings,
    /// One download at a time, so the quota is computed against everything
    /// that is on disk and nothing that is half written.
    downloads: Arc<tokio::sync::Semaphore>,
    /// One refresh pass at a time; a second asked for while one runs is
    /// answered by the one that runs.
    refreshing: Arc<tokio::sync::Mutex<()>>,
    /// Held for reading by every cover fetch a subscription starts in the
    /// background, so that taking it for writing waits for all of them.
    covers: Arc<tokio::sync::RwLock<()>>,
}

impl PodcastRuntime {
    pub(super) fn new(settings: crate::config::PodcastSettings) -> Self {
        Self {
            settings,
            downloads: Arc::new(tokio::sync::Semaphore::new(1)),
            refreshing: Arc::new(tokio::sync::Mutex::new(())),
            covers: Arc::new(tokio::sync::RwLock::new(())),
        }
    }
}

macro_rules! episode_columns {
    () => {
        "SELECT e.id, e.channel_id, c.title AS channel_title, c.artwork_hash, e.title, \
                e.description, \
                e.published_at, e.duration_ms, e.status, e.error_code, e.file_size, e.suffix, \
                e.created_at, b.position_ms \
         FROM podcast_episode e JOIN podcast_channel c ON c.id=e.channel_id \
         LEFT JOIN podcast_bookmark b ON b.episode_id=e.id AND b.user_id=? "
    };
}

macro_rules! channel_columns {
    () => {
        "SELECT c.id, c.feed_url, c.title, c.description, c.site_url, c.image_url, \
                c.artwork_hash, c.status, c.error_code, c.created_at, c.refreshed_at, \
                (SELECT COUNT(*) FROM podcast_episode e WHERE e.channel_id=c.id \
                   AND e.status<>'deleted') AS episode_count \
         FROM podcast_channel c "
    };
}

mod channels;
mod covers;
mod episodes;

impl DomainServices {
    fn podcast_policy(&self, timeout: std::time::Duration) -> FetchPolicy {
        FetchPolicy {
            addresses: self.podcasts.settings.addresses,
            allow_http: self.podcasts.settings.allow_http,
            timeout,
        }
    }

    /// Whether this account may subscribe, refresh, download and delete:
    /// what Subsonic's `podcastRole` reports.
    pub async fn can_manage_podcasts(&self, user_id: Uuid) -> Result<bool, ServiceError> {
        if !self.podcasts.settings.enabled {
            return Ok(false);
        }
        Ok(self
            .db
            .account_by_id(user_id)
            .await?
            .is_some_and(|account| account.role == AccountRole::Admin && !account.disabled))
    }

    /// Whether `podcastRole` holds for an account with this role, without a
    /// lookup: for the user listings, which already hold the account.
    pub fn podcast_role(&self, role: AccountRole, disabled: bool) -> bool {
        self.podcasts.settings.enabled && role == AccountRole::Admin && !disabled
    }

    async fn require_podcast_manager(&self, user_id: Uuid) -> Result<(), ServiceError> {
        if self.can_manage_podcasts(user_id).await? {
            Ok(())
        } else {
            Err(ServiceError::Forbidden)
        }
    }
}

/// The scheme, host and port of a feed address, and nothing that may be a
/// credential: no user information, no path, no query, no fragment. A
/// private feed's token is in its path as often as in its query.
fn feed_origin(feed_url: &str) -> String {
    match url::Url::parse(feed_url) {
        Ok(url) => match url.host_str() {
            Some(host) => match url.port() {
                Some(port) => format!("{}://{host}:{port}/", url.scheme()),
                None => format!("{}://{host}/", url.scheme()),
            },
            None => String::new(),
        },
        Err(_) => String::new(),
    }
}

fn channel_from_row(
    row: &sqlx::sqlite::SqliteRow,
    can_manage: bool,
) -> Result<PodcastChannelItem, sqlx::Error> {
    let feed_url: String = row.try_get("feed_url")?;
    Ok(PodcastChannelItem {
        id: parse_uuid(row.try_get("id")?)?,
        origin: feed_origin(&feed_url),
        url: can_manage.then_some(feed_url),
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        site_url: row.try_get("site_url")?,
        image_url: row
            .try_get::<Option<String>, _>("image_url")?
            .filter(|_| can_manage),
        artwork_hash: row.try_get("artwork_hash")?,
        status: row.try_get("status")?,
        error: row.try_get("error_code")?,
        created_at: row.try_get("created_at")?,
        refreshed_at: row.try_get("refreshed_at")?,
        episode_count: row.try_get("episode_count")?,
        episodes: None,
    })
}

fn episode_from_row(row: sqlx::sqlite::SqliteRow) -> Result<PodcastEpisodeItem, sqlx::Error> {
    let suffix: Option<String> = row.try_get("suffix")?;
    Ok(PodcastEpisodeItem {
        id: parse_uuid(row.try_get("id")?)?,
        channel_id: parse_uuid(row.try_get("channel_id")?)?,
        channel_title: row.try_get("channel_title")?,
        artwork_hash: row.try_get("artwork_hash")?,
        title: row.try_get("title")?,
        description: row.try_get("description")?,
        published_at: row.try_get("published_at")?,
        duration_ms: row.try_get("duration_ms")?,
        status: row.try_get("status")?,
        error: row.try_get("error_code")?,
        size: row.try_get("file_size")?,
        content_type: suffix
            .as_deref()
            .map(|suffix| episode_content_type(suffix).to_owned()),
        suffix,
        position_ms: row.try_get("position_ms")?,
        created_at: row.try_get("created_at")?,
    })
}

/// The content type of a stored episode, by the suffix this code chose for it.
fn episode_content_type(suffix: &str) -> &'static str {
    match suffix {
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "wav" => "audio/wav",
        "aac" => "audio/aac",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests;
