//! Channels: reading them, subscribing and unsubscribing, refreshing their
//! feeds, and the background refresher.

use super::covers::{write_cover, CoverWrite, KnownCover};
use super::*;

impl DomainServices {
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
            // One statement for every channel, ranked within each, rather than
            // one per channel.
            let rows = sqlx::query(concat!(
                episode_columns!(),
                "JOIN (SELECT id AS ranked_id, ROW_NUMBER() OVER (PARTITION BY channel_id \
                         ORDER BY published_at DESC NULLS LAST, title COLLATE NOCASE, id) \
                         AS listed_rank \
                       FROM podcast_episode \
                       WHERE status<>'deleted' AND (? IS NULL OR channel_id=?)) r \
                   ON r.ranked_id=e.id \
                 WHERE r.listed_rank<=? OR e.status IN ('completed', 'downloading') \
                 ORDER BY e.channel_id, r.listed_rank"
            ))
            .bind(user_id.to_string())
            .bind(only.as_deref())
            .bind(only.as_deref())
            .bind(MAX_EPISODES_LISTED)
            .fetch_all(&mut *connection)
            .await?;
            let mut by_channel: HashMap<Uuid, Vec<PodcastEpisodeItem>> = HashMap::new();
            for row in rows {
                let episode = episode_from_row(row)?;
                by_channel
                    .entry(episode.channel_id)
                    .or_default()
                    .push(episode);
            }
            for channel in &mut channels {
                channel.episodes = Some(by_channel.remove(&channel.id).unwrap_or_default());
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
        // Asked before the feed is fetched as well as under the gate below, so
        // a full instance contacts nobody; the answer that counts is the one
        // the insert is made under.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM podcast_channel")
            .fetch_one(self.db.pool())
            .await?;
        if count >= MAX_PODCAST_CHANNELS {
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
        // Only once the channel is stored: a subscription refused for the
        // ceiling or as a duplicate has fetched no cover and written nothing
        // to the store.
        //
        // **And not in the request.** The feed has already spent up to its own
        // deadline, and a cover host as slow again would hold the answer for
        // the sum of both — past the native route's timeout, after the channel
        // is committed, and without any bound at all on Subsonic's
        // `createPodcastChannel`, which no `TimeoutLayer` covers. So the
        // subscription answers with the cover pending, and the cover follows
        // in the background, under its own deadline. A cover that cannot be
        // fetched leaves the channel without one; the next refresh tries
        // again.
        if let Some(named) = feed.image_url {
            // Taken before the task is spawned, so a caller of
            // `podcast_covers_settled` that returns after this answer cannot
            // miss it.
            let pending = Arc::clone(&self.podcasts.covers).read_owned().await;
            let this = self.clone();
            tokio::spawn(async move {
                let _pending = pending;
                this.fill_first_cover(id, &named).await;
            });
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
        type ChannelRow = (String, String, Option<String>, Option<String>, Option<i64>);
        let channels: Vec<ChannelRow> = match sqlx::query_as(
            "SELECT id, feed_url, artwork_hash, artwork_url, artwork_fetched_at \
             FROM podcast_channel ORDER BY id",
        )
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
        for (id, feed_url, hash, url, fetched_at) in &channels {
            let known = KnownCover {
                hash: hash.clone(),
                url: url.clone(),
                fetched_at: *fetched_at,
            };
            if let Err(error) = self.refresh_channel(id, feed_url, &known).await {
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

    async fn refresh_channel(
        &self,
        id: &str,
        feed_url: &str,
        known: &KnownCover,
    ) -> Result<(), ServiceError> {
        let fetched = match url::Url::parse(feed_url) {
            Ok(url) => self.fetch_feed(&url).await,
            Err(_) => Err("destination_refused".to_owned()),
        };
        // A feed that could not be read says nothing about its cover, which
        // stays as it was.
        let cover = match &fetched {
            Ok(feed) => self.channel_cover(feed.image_url.as_deref(), known).await,
            Err(_) => CoverWrite::Keep,
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
                    write_cover(&mut tx, id, &cover, now).await?;
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
