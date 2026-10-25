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

use std::path::Path;

use super::*;
use crate::podcast::{self, FetchPolicy};

/// How many episodes `getNewestPodcasts` answers when it is not told, as the
/// specification has it, and the most it answers.
pub const NEWEST_EPISODES_DEFAULT: i64 = 20;
const NEWEST_EPISODES_MAX: i64 = 500;

/// How many channels one instance may hold. A refresh fetches every one.
const MAX_PODCAST_CHANNELS: i64 = 500;

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
    /// The cover the feed names. Never fetched by the server.
    pub image_url: Option<String>,
    /// `completed` when the last fetch succeeded, `error` when it did not.
    pub status: String,
    /// Why the last fetch failed, as a word from a fixed list.
    pub error: Option<String>,
    pub created_at: i64,
    pub refreshed_at: Option<i64>,
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
    /// `src/podcast.rs` defines. Nothing was stored.
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
}

impl PodcastRuntime {
    pub(super) fn new(settings: crate::config::PodcastSettings) -> Self {
        Self {
            settings,
            downloads: Arc::new(tokio::sync::Semaphore::new(1)),
            refreshing: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

macro_rules! episode_columns {
    () => {
        "SELECT e.id, e.channel_id, c.title AS channel_title, e.title, e.description, \
                e.published_at, e.duration_ms, e.status, e.error_code, e.file_size, e.suffix, \
                e.created_at, b.position_ms \
         FROM podcast_episode e JOIN podcast_channel c ON c.id=e.channel_id \
         LEFT JOIN podcast_bookmark b ON b.episode_id=e.id AND b.user_id=? "
    };
}

macro_rules! channel_columns {
    () => {
        "SELECT c.id, c.feed_url, c.title, c.description, c.site_url, c.image_url, c.status, \
                c.error_code, c.created_at, c.refreshed_at, \
                (SELECT COUNT(*) FROM podcast_episode e WHERE e.channel_id=c.id \
                   AND e.status<>'deleted') AS episode_count \
         FROM podcast_channel c "
    };
}

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

    /// The podcasts page: the switch, this account's role, the channels.
    pub async fn podcast_overview(&self, user_id: Uuid) -> Result<PodcastOverview, ServiceError> {
        let can_manage = self.can_manage_podcasts(user_id).await?;
        Ok(PodcastOverview {
            enabled: self.podcasts.settings.enabled,
            can_manage,
            channels: self.podcast_channels(user_id, None, false).await?,
        })
    }

    /// Every channel, or one, with its episodes when asked. What `getPodcasts`
    /// answers.
    pub async fn podcast_channels(
        &self,
        user_id: Uuid,
        only: Option<Uuid>,
        with_episodes: bool,
    ) -> Result<Vec<PodcastChannelItem>, ServiceError> {
        let can_manage = self.can_manage_podcasts(user_id).await?;
        let only = only.map(|id| id.to_string());
        let mut connection = self.db.pool().acquire().await?;
        let rows = sqlx::query(concat!(
            channel_columns!(),
            "WHERE (? IS NULL OR c.id=?) ORDER BY c.title COLLATE NOCASE, c.id"
        ))
        .bind(only.as_deref())
        .bind(only.as_deref())
        .fetch_all(&mut *connection)
        .await?;
        let mut channels = rows
            .iter()
            .map(|row| channel_from_row(row, can_manage))
            .collect::<Result<Vec<_>, _>>()?;
        if with_episodes {
            for channel in &mut channels {
                let rows = sqlx::query(concat!(
                    episode_columns!(),
                    "WHERE e.channel_id=? AND e.status<>'deleted' \
                     ORDER BY e.published_at DESC NULLS LAST, e.title COLLATE NOCASE, e.id"
                ))
                .bind(user_id.to_string())
                .bind(channel.id.to_string())
                .fetch_all(&mut *connection)
                .await?;
                channel.episodes = Some(
                    rows.into_iter()
                        .map(episode_from_row)
                        .collect::<Result<Vec<_>, _>>()?,
                );
            }
        }
        Ok(channels)
    }

    /// One channel and its episodes.
    pub async fn podcast_channel(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<PodcastChannelItem, ServiceError> {
        self.podcast_channels(user_id, Some(id), true)
            .await?
            .into_iter()
            .next()
            .ok_or(ServiceError::NotFound)
    }

    /// The newest episodes across every channel.
    pub async fn newest_podcast_episodes(
        &self,
        user_id: Uuid,
        count: i64,
    ) -> Result<Vec<PodcastEpisodeItem>, ServiceError> {
        let rows = sqlx::query(concat!(
            episode_columns!(),
            "WHERE e.status<>'deleted' \
             ORDER BY e.published_at DESC NULLS LAST, e.id LIMIT ?"
        ))
        .bind(user_id.to_string())
        .bind(count.clamp(0, NEWEST_EPISODES_MAX))
        .fetch_all(self.db.pool())
        .await?;
        rows.into_iter()
            .map(episode_from_row)
            .collect::<Result<Vec<_>, _>>()
            .map_err(ServiceError::from)
    }

    pub async fn podcast_episode(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<PodcastEpisodeItem, ServiceError> {
        let row = sqlx::query(concat!(episode_columns!(), "WHERE e.id=?"))
            .bind(user_id.to_string())
            .bind(id.to_string())
            .fetch_optional(self.db.pool())
            .await?
            .ok_or(ServiceError::NotFound)?;
        Ok(episode_from_row(row)?)
    }

    /// Subscribes to a feed: validates the address, fetches and reads the
    /// feed, and stores the channel with its episodes — or stores nothing.
    pub async fn create_podcast_channel(
        &self,
        user_id: Uuid,
        url: &str,
    ) -> Result<PodcastChannelItem, PodcastFailure> {
        self.require_podcast_manager(user_id).await?;
        let url = podcast::validate_feed_url(
            url,
            self.podcasts.settings.allow_http,
            self.podcasts.settings.addresses,
        )
        .ok_or(ServiceError::Invalid)?;
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM podcast_channel WHERE feed_url=?")
                .bind(url.as_str())
                .fetch_optional(self.db.pool())
                .await?;
        if exists.is_some() {
            return Err(ServiceError::Conflict.into());
        }
        let feed = self.fetch_feed(&url).await.map_err(PodcastFailure::Feed)?;
        let id = Uuid::new_v4();
        let now = now_ms();
        {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM podcast_channel")
                .fetch_one(&mut *tx)
                .await?;
            if count >= MAX_PODCAST_CHANNELS {
                tx.rollback().await?;
                return Err(ServiceError::Conflict.into());
            }
            let inserted = sqlx::query(
                "INSERT INTO podcast_channel \
                   (id, feed_url, title, description, site_url, image_url, status, error_code, \
                    created_by, created_at, updated_at, refreshed_at) \
                 VALUES (?, ?, ?, ?, ?, ?, 'completed', NULL, ?, ?, ?, ?) \
                 ON CONFLICT (feed_url) DO NOTHING",
            )
            .bind(id.to_string())
            .bind(url.as_str())
            .bind(&feed.title)
            .bind(feed.description.as_deref())
            .bind(feed.site_url.as_deref())
            .bind(feed.image_url.as_deref())
            .bind(user_id.to_string())
            .bind(now)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            if inserted.rows_affected() == 0 {
                tx.rollback().await?;
                return Err(ServiceError::Conflict.into());
            }
            upsert_episodes(&mut tx, id, &feed.episodes, now).await?;
            tx.commit().await?;
        }
        Ok(self.podcast_channel(user_id, id).await?)
    }

    /// Unsubscribes: the channel, its episodes, the bookmarks on them, and the
    /// files on disk.
    pub async fn delete_podcast_channel(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<(), ServiceError> {
        self.require_podcast_manager(user_id).await?;
        let files: Vec<Option<String>> = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let files = sqlx::query_scalar(
                "SELECT file_name FROM podcast_episode WHERE channel_id=? AND file_name IS NOT NULL",
            )
            .bind(id.to_string())
            .fetch_all(&mut *tx)
            .await?;
            let deleted = sqlx::query("DELETE FROM podcast_channel WHERE id=?")
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;
            if deleted.rows_affected() == 0 {
                tx.rollback().await?;
                return Err(ServiceError::NotFound);
            }
            tx.commit().await?;
            files
        };
        for file in files.into_iter().flatten() {
            self.remove_episode_file(&file).await;
        }
        Ok(())
    }

    /// Fetches every channel again, in the background. Answers at once.
    pub async fn refresh_podcasts(&self, user_id: Uuid) -> Result<(), ServiceError> {
        self.require_podcast_manager(user_id).await?;
        let this = self.clone();
        tokio::spawn(async move {
            this.refresh_all_podcasts().await;
        });
        Ok(())
    }

    /// One pass over every channel. A failure is recorded on its channel and
    /// does not stop the others.
    pub async fn refresh_all_podcasts(&self) {
        if !self.podcasts.settings.enabled {
            return;
        }
        let Ok(_pass) = self.podcasts.refreshing.try_lock() else {
            return;
        };
        let channels: Vec<(String, String)> =
            match sqlx::query_as("SELECT id, feed_url FROM podcast_channel ORDER BY id")
                .fetch_all(self.db.pool())
                .await
            {
                Ok(channels) => channels,
                Err(error) => {
                    tracing::error!(error = %error, "podcast refresh could not list channels");
                    return;
                }
            };
        let mut failed = 0usize;
        for (id, feed_url) in &channels {
            if let Err(error) = self.refresh_channel(id, feed_url).await {
                failed += 1;
                tracing::warn!(error = %error, "a podcast channel could not be refreshed");
            }
        }
        // A count, never an address: a feed URL may carry a private token.
        tracing::info!(
            channels = channels.len(),
            failed,
            "podcast refresh finished"
        );
    }

    async fn refresh_channel(&self, id: &str, feed_url: &str) -> Result<(), ServiceError> {
        let fetched = match url::Url::parse(feed_url) {
            Ok(url) => self.fetch_feed(&url).await,
            Err(_) => Err("destination_refused".to_owned()),
        };
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        match fetched {
            Ok(feed) => {
                let updated = sqlx::query(
                    "UPDATE podcast_channel SET title=?, description=?, site_url=?, image_url=?, \
                       status='completed', error_code=NULL, updated_at=?, refreshed_at=? \
                     WHERE id=?",
                )
                .bind(&feed.title)
                .bind(feed.description.as_deref())
                .bind(feed.site_url.as_deref())
                .bind(feed.image_url.as_deref())
                .bind(now)
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
                // Deleted while it was being fetched: nothing to write.
                if updated.rows_affected() > 0 {
                    upsert_episodes(&mut tx, parse_uuid(id.to_owned())?, &feed.episodes, now)
                        .await?;
                }
            }
            Err(code) => {
                sqlx::query(
                    "UPDATE podcast_channel SET status='error', error_code=?, updated_at=?, \
                       refreshed_at=? WHERE id=?",
                )
                .bind(&code)
                .bind(now)
                .bind(now)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// The feed at `url`, read, or the code saying why not.
    async fn fetch_feed(&self, url: &url::Url) -> Result<podcast::Feed, String> {
        let policy = self.podcast_policy(self.podcasts.settings.feed_timeout);
        let bytes = podcast::fetch_bounded(url, policy, self.podcasts.settings.max_feed_bytes)
            .await
            .map_err(podcast::FetchError::code)?;
        // Parsing is CPU work over up to the feed ceiling, off the runtime's
        // threads.
        let base = url.clone();
        tokio::task::spawn_blocking(move || podcast::parse_feed(&bytes, &base))
            .await
            .map_err(|_| "invalid_feed".to_owned())?
            .map_err(podcast::FeedError::code)
    }

    /// Queues an episode's download and answers at once.
    ///
    /// An episode already stored stays as it is; one that failed, or was
    /// deleted, is fetched again.
    pub async fn download_podcast_episode(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<(), ServiceError> {
        self.require_podcast_manager(user_id).await?;
        let queued = {
            let _writer = self.db.writer_guard().await;
            let mut connection = self.db.pool().acquire().await?;
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM podcast_episode WHERE id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *connection)
                    .await?;
            match status.as_deref() {
                None => return Err(ServiceError::NotFound),
                Some("completed" | "downloading") => false,
                Some(_) => {
                    sqlx::query(
                        "UPDATE podcast_episode SET status='downloading', error_code=NULL, \
                           updated_at=? WHERE id=?",
                    )
                    .bind(now_ms())
                    .bind(id.to_string())
                    .execute(&mut *connection)
                    .await?;
                    true
                }
            }
        };
        if queued {
            let this = self.clone();
            tokio::spawn(async move { this.run_download(id).await });
        }
        Ok(())
    }

    async fn run_download(&self, id: Uuid) {
        let Ok(_permit) = self.podcasts.downloads.clone().acquire_owned().await else {
            return;
        };
        let outcome = self.fetch_episode(id).await;
        let now = now_ms();
        let recorded: Result<(), ServiceError> = async {
            let _writer = self.db.writer_guard().await;
            let mut connection = self.db.pool().acquire().await?;
            match &outcome {
                Ok(stored) => {
                    let updated = sqlx::query(
                        "UPDATE podcast_episode SET status='completed', error_code=NULL, \
                           file_name=?, file_size=?, content_hash=?, suffix=?, updated_at=? \
                         WHERE id=? AND status='downloading'",
                    )
                    .bind(&stored.file_name)
                    .bind(stored.size)
                    .bind(&stored.hash)
                    .bind(stored.suffix)
                    .bind(now)
                    .bind(id.to_string())
                    .execute(&mut *connection)
                    .await?;
                    if updated.rows_affected() == 0 {
                        // Deleted, or its channel was, while it downloaded.
                        drop(connection);
                        drop(_writer);
                        self.remove_episode_file(&stored.file_name).await;
                    }
                }
                Err(code) => {
                    sqlx::query(
                        "UPDATE podcast_episode SET status='error', error_code=?, updated_at=? \
                         WHERE id=? AND status='downloading'",
                    )
                    .bind(code)
                    .bind(now)
                    .bind(id.to_string())
                    .execute(&mut *connection)
                    .await?;
                }
            }
            Ok(())
        }
        .await;
        match (&outcome, recorded) {
            (_, Err(error)) => {
                tracing::error!(error = %error, "a podcast download could not be recorded")
            }
            (Err(code), Ok(())) => {
                tracing::warn!(reason = %code, "a podcast episode could not be downloaded")
            }
            (Ok(_), Ok(())) => {}
        }
    }

    /// The download itself: into a staging file beside its final name, bounded
    /// by the episode ceiling and what is left of the quota, hashed as it is
    /// written, then recognised from its first bytes and renamed.
    async fn fetch_episode(&self, id: Uuid) -> Result<StoredEpisode, String> {
        let settings = &self.podcasts.settings;
        let (enclosure, used): (String, i64) = sqlx::query_as(
            "SELECT e.enclosure_url, \
                    (SELECT COALESCE(SUM(o.file_size), 0) FROM podcast_episode o \
                     WHERE o.status='completed') \
             FROM podcast_episode e WHERE e.id=?",
        )
        .bind(id.to_string())
        .fetch_one(self.db.pool())
        .await
        .map_err(|_| "unavailable".to_owned())?;
        let remaining = settings
            .quota_bytes
            .saturating_sub(u64::try_from(used).unwrap_or(u64::MAX));
        let ceiling = settings.max_episode_bytes.min(remaining);
        let over = if remaining < settings.max_episode_bytes {
            "quota_exceeded"
        } else {
            "too_large"
        };
        if ceiling == 0 {
            return Err("quota_exceeded".to_owned());
        }
        let url = url::Url::parse(&enclosure).map_err(|_| "destination_refused".to_owned())?;
        tokio::fs::create_dir_all(&settings.dir)
            .await
            .map_err(|_| "unavailable".to_owned())?;
        let staging = settings.dir.join(format!("{}.part", id.simple()));
        let policy = self.podcast_policy(settings.download_timeout);
        let written = tokio::time::timeout(
            settings.download_timeout,
            write_bounded(&url, policy, &staging, ceiling, over),
        )
        .await
        .unwrap_or_else(|_| Err("unreached".to_owned()));
        let (size, hash, head) = match written {
            Ok(written) => written,
            Err(code) => {
                let _ = tokio::fs::remove_file(&staging).await;
                return Err(code);
            }
        };
        let Some((suffix, _)) = podcast::sniff_audio(&head) else {
            let _ = tokio::fs::remove_file(&staging).await;
            return Err("not_audio".to_owned());
        };
        // Derived, never read from the feed: the episode's id and a suffix from
        // a fixed list.
        let file_name = format!("{}.{suffix}", id.simple());
        if tokio::fs::rename(&staging, settings.dir.join(&file_name))
            .await
            .is_err()
        {
            let _ = tokio::fs::remove_file(&staging).await;
            return Err("unavailable".to_owned());
        }
        Ok(StoredEpisode {
            file_name,
            size: i64::try_from(size).unwrap_or(i64::MAX),
            hash,
            suffix,
        })
    }

    /// Removes an episode's file and marks it deleted, so the next refresh
    /// does not bring it back.
    pub async fn delete_podcast_episode(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<(), ServiceError> {
        self.require_podcast_manager(user_id).await?;
        let file: Option<String> = {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            let file: Option<Option<String>> =
                sqlx::query_scalar("SELECT file_name FROM podcast_episode WHERE id=?")
                    .bind(id.to_string())
                    .fetch_optional(&mut *tx)
                    .await?;
            let Some(file) = file else {
                tx.rollback().await?;
                return Err(ServiceError::NotFound);
            };
            sqlx::query(
                "UPDATE podcast_episode SET status='deleted', error_code=NULL, file_name=NULL, \
                   file_size=NULL, content_hash=NULL, suffix=NULL, updated_at=? WHERE id=?",
            )
            .bind(now_ms())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            file
        };
        if let Some(file) = file {
            self.remove_episode_file(&file).await;
        }
        Ok(())
    }

    /// Unlinks a stored episode by the name the database holds for it, which
    /// this code derived. Checked again here anyway: a name that is not one
    /// this code could have produced is not unlinked.
    async fn remove_episode_file(&self, file_name: &str) {
        let derived = file_name.split_once('.').is_some_and(|(stem, suffix)| {
            Uuid::parse_str(stem).is_ok()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        });
        if !derived {
            return;
        }
        let _ = tokio::fs::remove_file(self.podcasts.settings.dir.join(file_name)).await;
    }

    /// What the media service streams for an episode id, when the episode is
    /// stored. Any account may play one.
    pub async fn podcast_stream_source(
        &self,
        episode_id: Uuid,
    ) -> Result<Option<crate::catalog::StreamTrack>, ServiceError> {
        let row = sqlx::query(
            "SELECT file_name, content_hash, suffix, duration_ms FROM podcast_episode \
             WHERE id=? AND status='completed'",
        )
        .bind(episode_id.to_string())
        .fetch_optional(self.db.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        Ok(Some(crate::catalog::StreamTrack {
            id: episode_id,
            library_id: Uuid::nil(),
            library_root: self.podcasts.settings.dir.clone(),
            relative_path: row.try_get("file_name")?,
            full_hash: row.try_get("content_hash")?,
            codec: row.try_get("suffix")?,
            bitrate: None,
            duration_ms: row.try_get::<Option<i64>, _>("duration_ms")?.unwrap_or(0),
            available: true,
            // The server's own folder, written by its downloads: no link in
            // it is ever followed (RFC-021), and an episode carries no
            // ReplayGain tags (RFC-022).
            link_policy: crate::link_roots::LinkPolicy::default(),
            replay_gain: crate::replay_gain::ReplayGainTags::default(),
        }))
    }

    /// What a stream route plays for an id: the track this account can read,
    /// or else a stored podcast episode.
    pub async fn stream_source_for_user(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<Option<crate::catalog::StreamTrack>, ServiceError> {
        if let Some(track) = self.db.stream_track_for_user(user_id, id).await? {
            return Ok(Some(track));
        }
        self.podcast_stream_source(id).await
    }

    /// Whether `id` names an episode that has not been deleted. The bookmark
    /// surfaces ask before treating an id as a track.
    pub(super) async fn is_podcast_episode(&self, id: Uuid) -> Result<bool, ServiceError> {
        let found: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM podcast_episode WHERE id=? AND status<>'deleted'")
                .bind(id.to_string())
                .fetch_optional(self.db.pool())
                .await?;
        Ok(found.is_some())
    }

    pub(super) async fn set_episode_bookmark(
        &self,
        user_id: Uuid,
        episode_id: Uuid,
        position_ms: i64,
        comment: Option<&str>,
    ) -> Result<(), ServiceError> {
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut connection = self.db.pool().acquire().await?;
        sqlx::query(
            "INSERT INTO podcast_bookmark \
               (user_id, episode_id, position_ms, comment, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (user_id, episode_id) DO UPDATE SET position_ms=excluded.position_ms, \
               comment=excluded.comment, updated_at=excluded.updated_at",
        )
        .bind(user_id.to_string())
        .bind(episode_id.to_string())
        .bind(position_ms)
        .bind(comment)
        .bind(now)
        .bind(now)
        .execute(&mut *connection)
        .await?;
        Ok(())
    }

    pub(super) async fn delete_episode_bookmark(
        &self,
        user_id: Uuid,
        episode_id: Uuid,
    ) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut connection = self.db.pool().acquire().await?;
        sqlx::query("DELETE FROM podcast_bookmark WHERE user_id=? AND episode_id=?")
            .bind(user_id.to_string())
            .bind(episode_id.to_string())
            .execute(&mut *connection)
            .await?;
        Ok(())
    }

    /// This account's episode bookmarks, most recently changed first.
    pub async fn episode_bookmarks(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<EpisodeBookmark>, ServiceError> {
        let rows = sqlx::query(concat!(
            episode_columns!(),
            "WHERE b.position_ms IS NOT NULL AND e.status<>'deleted' \
             ORDER BY b.updated_at DESC, e.id"
        ))
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let details: HashMap<String, (Option<String>, i64, i64)> = sqlx::query(
            "SELECT episode_id, comment, created_at, updated_at FROM podcast_bookmark \
             WHERE user_id=?",
        )
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("episode_id")?,
                (
                    row.try_get("comment")?,
                    row.try_get("created_at")?,
                    row.try_get("updated_at")?,
                ),
            ))
        })
        .collect::<Result<_, sqlx::Error>>()?;
        let mut bookmarks = Vec::with_capacity(rows.len());
        for row in rows {
            let episode = episode_from_row(row)?;
            let (comment, created_at, updated_at) = details
                .get(&episode.id.to_string())
                .cloned()
                .unwrap_or((None, 0, 0));
            bookmarks.push(EpisodeBookmark {
                position_ms: episode.position_ms.unwrap_or(0),
                comment,
                created_at,
                updated_at,
                episode,
            });
        }
        Ok(bookmarks)
    }

    /// At boot: a download the last process left running is not running any
    /// more. It goes back to `new`, and its staging file is removed.
    pub async fn reconcile_podcasts(&self) -> Result<(), ServiceError> {
        {
            let _writer = self.db.writer_guard().await;
            let mut connection = self.db.pool().acquire().await?;
            sqlx::query(
                "UPDATE podcast_episode SET status='new', updated_at=? WHERE status='downloading'",
            )
            .bind(now_ms())
            .execute(&mut *connection)
            .await?;
        }
        let dir = &self.podcasts.settings.dir;
        if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.path().extension().is_some_and(|ext| ext == "part") {
                    let _ = tokio::fs::remove_file(entry.path()).await;
                }
            }
        }
        Ok(())
    }

    /// The periodic refresh, when the switch is on and an interval is set.
    /// The first pass waits one interval: a boot sends nothing.
    pub fn spawn_podcast_refresher(&self) {
        let Some(interval) = self
            .podcasts
            .settings
            .refresh_interval
            .filter(|_| self.podcasts.settings.enabled)
        else {
            return;
        };
        let this = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                this.refresh_all_podcasts().await;
            }
        });
    }
}

struct StoredEpisode {
    file_name: String,
    size: i64,
    hash: String,
    suffix: &'static str,
}

/// Streams the answer at `url` into `staging`, at most `ceiling` bytes,
/// answering its size, its BLAKE3 and its first bytes.
async fn write_bounded(
    url: &url::Url,
    policy: FetchPolicy,
    staging: &Path,
    ceiling: u64,
    over: &'static str,
) -> Result<(u64, String, Vec<u8>), String> {
    use tokio::io::AsyncWriteExt;

    let mut response = podcast::open(url, policy)
        .await
        .map_err(podcast::FetchError::code)?;
    if response
        .content_length()
        .is_some_and(|length| length > ceiling)
    {
        return Err(over.to_owned());
    }
    let mut file = tokio::fs::File::create(staging)
        .await
        .map_err(|_| "unavailable".to_owned())?;
    let mut hasher = blake3::Hasher::new();
    let mut head = Vec::with_capacity(16);
    let mut total: u64 = 0;
    while let Some(chunk) = response.chunk().await.map_err(|_| "unreached".to_owned())? {
        total += chunk.len() as u64;
        if total > ceiling {
            return Err(over.to_owned());
        }
        if head.len() < 16 {
            let take = (16 - head.len()).min(chunk.len());
            head.extend_from_slice(&chunk[..take]);
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|_| "unavailable".to_owned())?;
    }
    file.flush().await.map_err(|_| "unavailable".to_owned())?;
    file.sync_all()
        .await
        .map_err(|_| "unavailable".to_owned())?;
    if total == 0 {
        return Err("not_audio".to_owned());
    }
    Ok((total, hasher.finalize().to_hex().to_string(), head))
}

/// Writes a feed's episodes onto a channel. A known episode is updated —
/// title, description, dates and enclosure — and keeps its status, so one that
/// was deleted stays deleted and one that is stored keeps its file.
async fn upsert_episodes(
    tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
    channel_id: Uuid,
    episodes: &[podcast::FeedEpisode],
    now: i64,
) -> Result<(), sqlx::Error> {
    for episode in episodes {
        sqlx::query(
            "INSERT INTO podcast_episode \
               (id, channel_id, guid, title, description, published_at, duration_ms, \
                enclosure_url, enclosure_type, enclosure_length, status, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'new', ?, ?) \
             ON CONFLICT (channel_id, guid) DO UPDATE SET title=excluded.title, \
               description=excluded.description, published_at=excluded.published_at, \
               duration_ms=COALESCE(excluded.duration_ms, duration_ms), \
               enclosure_url=excluded.enclosure_url, enclosure_type=excluded.enclosure_type, \
               enclosure_length=excluded.enclosure_length, updated_at=excluded.updated_at",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(channel_id.to_string())
        .bind(&episode.guid)
        .bind(&episode.title)
        .bind(episode.description.as_deref())
        .bind(episode.published_at)
        .bind(episode.duration_ms)
        .bind(&episode.enclosure_url)
        .bind(episode.enclosure_type.as_deref())
        .bind(episode.enclosure_length)
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
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
        image_url: row.try_get("image_url")?,
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
mod tests {
    use super::*;

    #[test]
    fn a_reader_is_shown_the_feeds_host_and_nothing_after_it() {
        assert_eq!(
            feed_origin("https://feeds.example.org/rss/secret-path.xml?token=secret#top"),
            "https://feeds.example.org/"
        );
        assert_eq!(
            feed_origin("http://user:pass@127.0.0.1:8080/feed/token/podcast.xml"),
            "http://127.0.0.1:8080/"
        );
        assert_eq!(feed_origin("not a url"), "");
    }
}
