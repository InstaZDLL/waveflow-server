//! Episodes: reading them, downloading and deleting their files, streaming
//! them, and the bookmarks kept in them.

use super::*;

impl DomainServices {
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
    pub(super) async fn remove_episode_file(&self, file_name: &str) {
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
    ) -> Result<Option<crate::storage::catalog::StreamTrack>, ServiceError> {
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
        Ok(Some(crate::storage::catalog::StreamTrack {
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
            link_policy: crate::library::link_roots::LinkPolicy::default(),
            replay_gain: crate::media::replay_gain::ReplayGainTags::default(),
        }))
    }

    /// What a stream route plays for an id: the track this account can read,
    /// or else a stored podcast episode.
    pub async fn stream_source_for_user(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<Option<crate::storage::catalog::StreamTrack>, ServiceError> {
        if let Some(track) = self.db.stream_track_for_user(user_id, id).await? {
            return Ok(Some(track));
        }
        self.podcast_stream_source(id).await
    }

    /// Whether `id` names an episode that has not been deleted. The bookmark
    /// surfaces ask before treating an id as a track.
    pub(in crate::services) async fn is_podcast_episode(
        &self,
        id: Uuid,
    ) -> Result<bool, ServiceError> {
        let found: Option<i64> =
            sqlx::query_scalar("SELECT 1 FROM podcast_episode WHERE id=? AND status<>'deleted'")
                .bind(id.to_string())
                .fetch_optional(self.db.pool())
                .await?;
        Ok(found.is_some())
    }

    pub(in crate::services) async fn set_episode_bookmark(
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

    pub(in crate::services) async fn delete_episode_bookmark(
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
