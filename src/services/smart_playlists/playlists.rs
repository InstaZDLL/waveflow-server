//! Smart playlists as an account keeps them: listing, reading, previewing,
//! creating, updating and deleting.

use super::*;

/// The longest comment a smart playlist may carry, in characters.
const MAX_SMART_COMMENT_CHARS: usize = 2_000;

/// A smart playlist, as every surface reads it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SmartPlaylistItem {
    pub id: Uuid,
    pub owner_id: Uuid,
    pub owner_username: String,
    pub name: String,
    pub comment: Option<String>,
    /// Readable by every account on the server, through each one's own
    /// libraries; editable by its owner only.
    pub public: bool,
    pub rules: SmartRules,
    pub created_at: i64,
    pub updated_at: i64,
    /// When the counts below were evaluated for this reader. A smart playlist
    /// has no stored track list. On a read of one playlist it is now; in a
    /// listing it is when this reader last opened it, and `None` when they
    /// have not since the server started — a listing evaluates nothing
    /// (decision 4).
    pub evaluated_at: Option<i64>,
    /// What the reader can play of it, and how long that lasts, as of
    /// `evaluated_at`; `None` with it.
    pub song_count: Option<i64>,
    pub duration_ms: Option<i64>,
    /// The tracks, on a read of one playlist; absent from a listing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub songs: Option<Vec<SongItem>>,
}

/// A rule set evaluated without being stored: what the editor shows.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SmartPlaylistPreview {
    pub song_count: i64,
    pub duration_ms: i64,
    pub songs: Vec<SongItem>,
}

/// Which smart playlists a read reaches, as for ordinary ones (RFC-020
/// decision 2): the reader's own, and other enabled accounts' public ones.
macro_rules! smart_playlist_columns {
    () => {
        "SELECT sp.id, sp.owner_user_id, a.username AS owner_username, sp.name, sp.comment, \
                sp.public, sp.rules_json, sp.created_at, sp.updated_at \
         FROM smart_playlist sp JOIN account a ON a.id=sp.owner_user_id "
    };
}

impl DomainServices {
    /// This account's smart playlists, newest first, and — when asked —
    /// every other enabled account's public ones after them.
    pub async fn smart_playlists(
        &self,
        user_id: Uuid,
        include_public: bool,
    ) -> Result<Vec<SmartPlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        let rows = sqlx::query(concat!(
            smart_playlist_columns!(),
            "WHERE sp.owner_user_id=? OR (? = 1 AND sp.public=1 AND a.disabled=0) \
             ORDER BY sp.owner_user_id<>?, sp.updated_at DESC, sp.id"
        ))
        .bind(user_id.to_string())
        .bind(i64::from(include_public))
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut playlists = Vec::with_capacity(rows.len());
        for row in rows {
            let Some(mut playlist) = smart_playlist_from_row(&row)? else {
                continue;
            };
            // **No evaluation here** (decision 4): what this reader was last
            // shown of it, if the rules have not changed since.
            if let Some(known) = self
                .smart
                .extents
                .get(&(playlist.id, user_id))
                .filter(|known| known.updated_at == playlist.updated_at)
            {
                playlist.evaluated_at = Some(known.evaluated_at);
                playlist.song_count = Some(known.song_count);
                playlist.duration_ms = Some(known.duration_ms);
            }
            playlists.push(playlist);
        }
        Ok(playlists)
    }

    /// One smart playlist this account may read, evaluated now, its tracks
    /// read through this account's libraries. Another account's private one
    /// answers like one that does not exist.
    pub async fn smart_playlist(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<SmartPlaylistItem, ServiceError> {
        // No connection is held across the evaluation, which takes one of its
        // own: with every pooled connection held by a reader waiting for a
        // second, the pool would deadlock until its acquire timeout.
        let row = sqlx::query(concat!(
            smart_playlist_columns!(),
            "WHERE sp.id=? AND (sp.owner_user_id=? OR (sp.public=1 AND a.disabled=0))"
        ))
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(self.db.pool())
        .await?
        .ok_or(ServiceError::NotFound)?;
        let mut playlist = smart_playlist_from_row(&row)?.ok_or(ServiceError::NotFound)?;
        let ids = self.evaluate(playlist.owner_id, &playlist.rules).await?;
        let mut connection = self.db.pool().acquire().await?;
        let songs = self
            .songs_by_ids_lenient_on(&mut connection, user_id, &ids)
            .await?;
        playlist.evaluated_at = Some(now_ms());
        playlist.song_count = Some(songs.len() as i64);
        playlist.duration_ms = Some(songs.iter().map(|song| song.duration_ms).sum());
        playlist.songs = Some(songs);
        self.remember_extent(&playlist, user_id);
        Ok(playlist)
    }

    /// What a rule set would answer for this account, without storing it.
    pub async fn preview_smart_playlist(
        &self,
        user_id: Uuid,
        rules: &SmartRules,
    ) -> Result<SmartPlaylistPreview, ServiceError> {
        let ids = self.evaluate(user_id, rules).await?;
        let mut connection = self.db.pool().acquire().await?;
        let songs = self
            .songs_by_ids_lenient_on(&mut connection, user_id, &ids)
            .await?;
        Ok(SmartPlaylistPreview {
            song_count: songs.len() as i64,
            duration_ms: songs.iter().map(|song| song.duration_ms).sum(),
            songs,
        })
    }

    pub async fn create_smart_playlist(
        &self,
        user_id: Uuid,
        name: &str,
        comment: Option<&str>,
        public: bool,
        rules: &SmartRules,
    ) -> Result<SmartPlaylistItem, ServiceError> {
        let name = smart_name(name)?;
        let comment = smart_comment(comment)?;
        let id = Uuid::new_v4();
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        // Counted inside `BEGIN IMMEDIATE`, under the gate: two creations
        // cannot both read 99, as for radio stations.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM smart_playlist WHERE owner_user_id=?")
                .bind(user_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        if count >= MAX_SMART_PLAYLISTS {
            tx.rollback().await?;
            return Err(ServiceError::Conflict);
        }
        sqlx::query(
            "INSERT INTO smart_playlist \
               (id, owner_user_id, name, comment, public, rules_json, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .bind(&name)
        .bind(comment.as_deref())
        .bind(i64::from(public))
        .bind(rules.to_stored())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        drop(_writer);
        self.smart_playlist(user_id, id).await
    }

    /// Changes what the owner names. `rules`, when given, replaces the whole
    /// rule set: rules are not patched one condition at a time.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_smart_playlist(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        clear_comment: bool,
        public: Option<bool>,
        rules: Option<&SmartRules>,
    ) -> Result<SmartPlaylistItem, ServiceError> {
        let name = name.map(smart_name).transpose()?;
        let comment = smart_comment(comment)?;
        let rules = rules.map(SmartRules::to_stored);
        let now = now_ms();
        let _writer = self.db.writer_guard().await;
        let mut connection = self.db.pool().acquire().await?;
        // The owner's own only, in SQL: another account's public playlist
        // answers this exactly as a missing one does.
        let updated = sqlx::query(
            "UPDATE smart_playlist SET \
               name=COALESCE(?, name), \
               comment=CASE WHEN ? = 1 THEN NULL ELSE COALESCE(?, comment) END, \
               public=COALESCE(?, public), \
               rules_json=COALESCE(?, rules_json), \
               updated_at=? \
             WHERE owner_user_id=? AND id=?",
        )
        .bind(name.as_deref())
        .bind(i64::from(clear_comment))
        .bind(comment.as_deref())
        .bind(public.map(i64::from))
        .bind(rules.as_deref())
        .bind(now)
        .bind(user_id.to_string())
        .bind(id.to_string())
        .execute(&mut *connection)
        .await?;
        drop(connection);
        drop(_writer);
        if updated.rows_affected() == 0 {
            return Err(ServiceError::NotFound);
        }
        self.smart_playlist(user_id, id).await
    }

    pub async fn delete_smart_playlist(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut connection = self.db.pool().acquire().await?;
        let deleted = sqlx::query("DELETE FROM smart_playlist WHERE owner_user_id=? AND id=?")
            .bind(user_id.to_string())
            .bind(id.to_string())
            .execute(&mut *connection)
            .await?;
        if deleted.rows_affected() == 0 {
            return Err(ServiceError::NotFound);
        }
        Ok(())
    }
}

/// A row, or `None` when its stored rules no longer parse — a hand edit, or a
/// rule a later version removed. Such a playlist is skipped rather than
/// evaluated on rules this version cannot vouch for.
fn smart_playlist_from_row(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<Option<SmartPlaylistItem>, ServiceError> {
    let stored: String = row.try_get("rules_json")?;
    let Some(rules) = serde_json::from_str(&stored)
        .ok()
        .and_then(|value| SmartRules::parse(&value).ok())
    else {
        tracing::warn!("a smart playlist carries rules this server cannot read; it is skipped");
        return Ok(None);
    };
    Ok(Some(SmartPlaylistItem {
        id: parse_uuid(row.try_get("id")?)?,
        owner_id: parse_uuid(row.try_get("owner_user_id")?)?,
        owner_username: row.try_get("owner_username")?,
        name: row.try_get("name")?,
        comment: row.try_get("comment")?,
        public: row.try_get::<i64, _>("public")? != 0,
        rules,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
        evaluated_at: None,
        song_count: None,
        duration_ms: None,
        songs: None,
    }))
}

fn smart_name(name: &str) -> Result<String, ServiceError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(ServiceError::Invalid);
    }
    Ok(name.to_owned())
}

/// A blank comment is no comment.
fn smart_comment(comment: Option<&str>) -> Result<Option<String>, ServiceError> {
    match comment.map(str::trim) {
        None | Some("") => Ok(None),
        Some(comment) if comment.chars().count() > MAX_SMART_COMMENT_CHARS => {
            Err(ServiceError::Invalid)
        }
        Some(comment) => Ok(Some(comment.to_owned())),
    }
}
