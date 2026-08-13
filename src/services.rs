//! Shared v2 domain services and tenant-filtered read models.

use std::{collections::HashMap, path::PathBuf, str::FromStr, sync::Arc};

use serde::Serialize;
use sqlx::{Row, SqliteConnection};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{
    authentication::now_ms,
    database::{AccountRecord, AccountRole, Database},
    security::{self, EncryptedSecret, SecretBox},
    sync::{MutationContext, MutationIntent, MutationReceipt, OperationClaim, SyncService},
};

/// Tenant-filtered projections shared by the Subsonic facade and the native
/// browse endpoints. Each expands to a literal ending at `WHERE m.user_id=?` so
/// callers `concat!` their own predicates onto it — sqlx only accepts static SQL,
/// which keeps these compositions injection-proof by construction. The first
/// bind is always the user id.
macro_rules! song_select {
    () => {
        "SELECT t.id, t.library_id, t.album_id, t.title, t.album_title, t.artist_display, \
                t.genre_display, t.year, t.track_number, t.disc_number, t.duration_ms, t.bitrate, \
                t.codec, t.relative_path, t.file_size, t.artwork_hash, t.full_hash, t.created_at, \
                us.starred_at, ur.rating AS user_rating \
         FROM track t JOIN library_member m ON m.library_id=t.library_id \
         LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='track' AND us.entity_id=t.id \
         LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='track' AND ur.entity_id=t.id \
         WHERE m.user_id=? AND t.is_available=1"
    };
}

macro_rules! album_select {
    () => {
        "SELECT al.id, al.library_id, al.title, al.album_artist_name, al.album_artist_id, \
                al.artwork_hash, al.year, al.created_at, us.starred_at, ur.rating AS user_rating, \
                (SELECT COUNT(*) FROM play_event pe JOIN track pt ON pt.id=pe.track_id \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pt.album_id=al.id) AS play_count, \
                (SELECT MAX(pe.played_at) FROM play_event pe JOIN track pt ON pt.id=pe.track_id \
                 WHERE pe.user_id=m.user_id AND pe.submission=1 AND pt.album_id=al.id) AS last_played_at \
         FROM album al JOIN library_member m ON m.library_id=al.library_id \
         LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='album' AND us.entity_id=al.id \
         LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='album' AND ur.entity_id=al.id \
         WHERE m.user_id=?"
    };
}

macro_rules! artist_select {
    () => {
        "SELECT ar.id, ar.library_id, ar.name, ar.artwork_hash, us.starred_at, \
                ur.rating AS user_rating, \
                (SELECT COUNT(*) FROM album al WHERE al.album_artist_id=ar.id) AS album_count \
         FROM artist ar JOIN library_member m ON m.library_id=ar.library_id \
         LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='artist' AND us.entity_id=ar.id \
         LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='artist' AND ur.entity_id=ar.id \
         WHERE m.user_id=?"
    };
}

pub struct SubsonicCredentialRecord {
    pub account: AccountRecord,
    pub encrypted_password: EncryptedSecret,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MusicFolderItem {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtistItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub name: String,
    pub artwork_hash: Option<String>,
    pub starred_at: Option<i64>,
    pub user_rating: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AlbumItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub artist_id: Option<Uuid>,
    pub artwork_hash: Option<String>,
    pub year: Option<i64>,
    pub created_at: i64,
    pub starred_at: Option<i64>,
    pub user_rating: Option<i64>,
    pub play_count: i64,
    pub last_played_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SongItem {
    pub id: Uuid,
    pub library_id: Uuid,
    pub album_id: Option<Uuid>,
    pub title: String,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<i64>,
    pub track: Option<i64>,
    pub disc: Option<i64>,
    pub duration_ms: i64,
    pub bitrate: Option<i64>,
    pub codec: Option<String>,
    pub suffix: String,
    pub size: i64,
    pub artwork_hash: Option<String>,
    /// Content fingerprint: **BLAKE3, unkeyed, hexadecimal, over the whole
    /// file** — 64 characters. A client can compute the same value locally and
    /// compare, which is the only automatic link M5 accepts (a unique full-hash
    /// match; MBID stays a suggestion to confirm).
    ///
    /// It fingerprints the *file*, not the decoded audio: two copies of one
    /// recording with different tags do not match. The algorithm is part of the
    /// contract — changing it means adding a field, never redefining this one.
    pub full_hash: String,
    pub created_at: i64,
    pub starred_at: Option<i64>,
    pub user_rating: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct CatalogSnapshot {
    pub folders: Vec<MusicFolderItem>,
    pub artists: Vec<ArtistItem>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
}

/// Result of a Subsonic `search3`, backed by the FTS5 index.
#[derive(Debug, Clone)]
pub struct CatalogSearch {
    pub artists: Vec<ArtistItem>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
    /// Every track of every returned album — not only the matching ones.
    ///
    /// `album` nodes carry `songCount` and `duration` derived from the track
    /// list they are rendered with. Handing them only the matches would make an
    /// album of twelve tracks report two, wrongly and silently, whenever the
    /// query happened to hit two of them.
    pub album_tracks: Vec<SongItem>,
}

/// Upper bound on a native browse page. It matches the Subsonic contract's
/// 500-item cap so both surfaces expose the same paging ceiling.
pub const MAX_BROWSE_LIMIT: i64 = 500;
const DEFAULT_BROWSE_LIMIT: i64 = 100;
pub const MAX_HISTORY_LIMIT: i64 = 500;
/// Fits a UUID-only queue request below the server's 16 KiB body limit while
/// also bounding the work performed under the global SQLite writer gate.
pub const MAX_QUEUE_TRACKS: usize = 400;
/// Applies the same request-size and writer-gate bound to public shares.
pub const MAX_SHARE_TRACKS: usize = MAX_QUEUE_TRACKS;

/// Offset/limit pair validated once, at the HTTP boundary, so the SQL layer can
/// bind it without re-checking bounds.
#[derive(Debug, Clone, Copy)]
pub struct BrowsePage {
    offset: i64,
    limit: i64,
}

impl BrowsePage {
    pub fn new(offset: Option<i64>, limit: Option<i64>) -> Result<Self, ServiceError> {
        let offset = offset.unwrap_or(0);
        let limit = limit.unwrap_or(DEFAULT_BROWSE_LIMIT);
        if offset < 0 || limit <= 0 || limit > MAX_BROWSE_LIMIT {
            return Err(ServiceError::Invalid);
        }
        Ok(Self { offset, limit })
    }
}

impl Default for BrowsePage {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: DEFAULT_BROWSE_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtistSummary {
    #[serde(flatten)]
    pub artist: ArtistItem,
    pub album_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AlbumDetail {
    #[serde(flatten)]
    pub album: AlbumItem,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ArtistDetail {
    #[serde(flatten)]
    pub artist: ArtistItem,
    /// Same field the list endpoint returns. `albums` below is unpaginated, so
    /// its length matches — but that is an unwritten guarantee, and a client
    /// should not have to depend on one.
    pub album_count: i64,
    pub albums: Vec<AlbumItem>,
}

/// Inputs for a native client's authorization request.
#[derive(Debug, Clone, Copy)]
pub struct AuthorizationRequest<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub code_challenge: &'a str,
    pub code_challenge_method: &'a str,
    pub device_name: &'a str,
    pub state: Option<&'a str>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SearchResult {
    pub artists: Vec<ArtistItem>,
    pub albums: Vec<AlbumItem>,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PlaylistItem {
    pub id: Uuid,
    pub name: String,
    pub comment: Option<String>,
    pub public: bool,
    pub created_at: i64,
    pub updated_at: i64,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct QueueItem {
    pub current: Option<Uuid>,
    pub position_ms: i64,
    pub changed_by: Option<String>,
    pub updated_at: i64,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RatingItem {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub rating: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HistoryItem {
    pub track_id: Uuid,
    pub submission: bool,
    pub played_at: i64,
}

/// Optional fields a share update may blank out.
///
/// `COALESCE(?, column)` cannot express this: an absent field and an explicit
/// null arrive as the same bind, so "leave it alone" and "remove it" collapse.
/// The consequence was not cosmetic — an expiry set by mistake could never be
/// lifted, and the owner's only recourse was to delete the share and mint a new
/// URL. Clearing is opt-in and named, so a client that merely omits a field can
/// never erase one by accident.
#[derive(Debug, Clone, Copy, Default)]
pub struct ShareClear {
    pub description: bool,
    pub expires_at: bool,
}

/// Optional fields a playlist update may blank out. See [`ShareClear`].
#[derive(Debug, Clone, Copy, Default)]
pub struct PlaylistClear {
    pub comment: bool,
}

#[derive(Debug, Clone)]
pub struct ShareItem {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// Present only in the result of a newly-created share. Persistent reads
    /// deliberately cannot recover the bearer token from its lookup hash.
    pub url_token: Option<String>,
    pub description: Option<String>,
    pub expires_at: Option<i64>,
    pub created_at: i64,
    pub visit_count: i64,
    pub songs: Vec<SongItem>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct UserItem {
    pub id: Uuid,
    pub username: String,
    pub role: AccountRole,
    pub disabled: bool,
    pub has_subsonic_credential: bool,
    pub folder_ids: Vec<Uuid>,
}

pub struct UserUpdate<'a> {
    pub admin: Option<bool>,
    pub disabled: Option<bool>,
    pub folder_ids: Option<&'a [Uuid]>,
    pub subsonic_password: Option<&'a str>,
    pub web_password: Option<&'a str>,
}

pub struct SyncSnapshotData {
    pub cursor: i64,
    pub playlists: Vec<PlaylistItem>,
    pub favorites: Vec<(String, Uuid, i64)>,
    pub ratings: Vec<RatingItem>,
    pub queue: Option<QueueItem>,
    pub history: Vec<HistoryItem>,
    pub shares: Vec<ShareItem>,
}

#[derive(Clone)]
pub struct DomainServices {
    db: Database,
    secret_box: Arc<SecretBox>,
    sync: SyncService,
}

#[derive(Debug, thiserror::Error)]
pub enum ServiceError {
    #[error("resource not found")]
    NotFound,
    #[error("operation is forbidden")]
    Forbidden,
    #[error("invalid input")]
    Invalid,
    #[error("conflict")]
    Conflict,
    #[error("service unavailable")]
    Unavailable,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Security(#[from] security::SecurityError),
}

impl From<crate::sync::SyncError> for ServiceError {
    fn from(error: crate::sync::SyncError) -> Self {
        match error {
            crate::sync::SyncError::Invalid => Self::Invalid,
            // Mutations claim operations; they never read the journal, so
            // CursorExpired cannot reach this conversion. Folded into Conflict
            // rather than given a domain variant nothing would ever construct.
            crate::sync::SyncError::Conflict | crate::sync::SyncError::CursorExpired => {
                Self::Conflict
            }
            crate::sync::SyncError::Database(error) => Self::Database(error),
        }
    }
}

impl DomainServices {
    pub fn new(db: Database, secret_box: Arc<SecretBox>, sync: SyncService) -> Self {
        Self {
            db,
            secret_box,
            sync,
        }
    }

    pub async fn bootstrap_admin(
        &self,
        username: &str,
        password: &str,
    ) -> Result<Uuid, ServiceError> {
        validate_username(username)?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || security::hash_password(&password))
            .await
            .map_err(|_| ServiceError::Unavailable)??;
        self.db
            .bootstrap_admin(username, &password_hash, now_ms())
            .await?
            .ok_or(ServiceError::Conflict)
    }

    pub async fn credential_by_username(
        &self,
        username: &str,
    ) -> Result<Option<SubsonicCredentialRecord>, ServiceError> {
        let row = sqlx::query(
            "SELECT a.id, a.username, a.password_hash, a.role, a.disabled, \
                    c.password_nonce, c.password_ciphertext \
             FROM account a JOIN subsonic_credential c ON c.user_id=a.id \
             WHERE a.username=? COLLATE NOCASE AND a.disabled=0",
        )
        .bind(username)
        .fetch_optional(self.db.pool())
        .await?;
        row.map(credential_from_row).transpose().map_err(Into::into)
    }

    pub async fn credential_by_api_key(
        &self,
        api_key: &str,
    ) -> Result<Option<SubsonicCredentialRecord>, ServiceError> {
        let hash = security::token_hash(api_key);
        let row = sqlx::query(
            "SELECT a.id, a.username, a.password_hash, a.role, a.disabled, \
                    c.password_nonce, c.password_ciphertext \
             FROM account a JOIN subsonic_credential c ON c.user_id=a.id \
             WHERE c.api_key_hash=? AND a.disabled=0",
        )
        .bind(hash.as_slice())
        .fetch_optional(self.db.pool())
        .await?;
        row.map(credential_from_row).transpose().map_err(Into::into)
    }

    pub fn decrypt_subsonic_password(
        &self,
        credential: &SubsonicCredentialRecord,
    ) -> Result<Vec<u8>, ServiceError> {
        self.secret_box
            .decrypt(
                &credential.encrypted_password.nonce,
                &credential.encrypted_password.ciphertext,
            )
            .map_err(Into::into)
    }

    pub async fn catalog_snapshot(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
    ) -> Result<CatalogSnapshot, ServiceError> {
        let folder_filter = (!folder_ids.is_empty()).then(|| {
            serde_json::to_string(folder_ids).expect("UUID list serialization cannot fail")
        });
        let folders = sqlx::query(
            "SELECT l.id, l.name FROM library l JOIN library_member m ON m.library_id=l.id \
             WHERE m.user_id=? AND (? IS NULL OR l.id IN (SELECT value FROM json_each(?))) \
             ORDER BY l.name COLLATE NOCASE",
        )
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(|row| {
            Ok(MusicFolderItem {
                id: parse_uuid(row.try_get("id")?)?,
                name: row.try_get("name")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
        let artists = sqlx::query(
            "SELECT ar.id, ar.library_id, ar.name, ar.artwork_hash, \
                    us.starred_at, ur.rating AS user_rating FROM artist ar \
             JOIN library_member m ON m.library_id=ar.library_id \
             LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='artist' AND us.entity_id=ar.id \
             LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='artist' AND ur.entity_id=ar.id \
             WHERE m.user_id=? AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
             ORDER BY ar.name COLLATE NOCASE",
        )
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let albums = sqlx::query(
            "SELECT al.id, al.library_id, al.title, al.album_artist_name, al.album_artist_id, \
                    al.artwork_hash, al.year, al.created_at, us.starred_at, ur.rating AS user_rating, \
                    (SELECT COUNT(*) FROM play_event pe JOIN track pt ON pt.id=pe.track_id \
                     WHERE pe.user_id=m.user_id AND pe.submission=1 AND pt.album_id=al.id) AS play_count, \
                    (SELECT MAX(pe.played_at) FROM play_event pe JOIN track pt ON pt.id=pe.track_id \
                     WHERE pe.user_id=m.user_id AND pe.submission=1 AND pt.album_id=al.id) AS last_played_at \
             FROM album al \
             JOIN library_member m ON m.library_id=al.library_id \
             LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='album' AND us.entity_id=al.id \
             LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='album' AND ur.entity_id=al.id \
             WHERE m.user_id=? AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
             ORDER BY al.title COLLATE NOCASE",
        )
        .bind(user_id.to_string())
        .bind(folder_filter.as_deref())
        .bind(folder_filter.as_deref())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let songs = fetch_songs(&self.db, user_id, folder_filter.as_deref(), None).await?;
        Ok(CatalogSnapshot {
            folders,
            artists,
            albums,
            songs,
        })
    }

    /// Backs Subsonic `search3` with the FTS5 index instead of materialising the
    /// whole catalogue and filtering it in memory.
    ///
    /// `track_fts` indexes title, album, artists and genres per track, so
    /// selecting matching tracks and deriving their albums and artists covers
    /// the same ground the in-memory pass did — a matching album title reaches
    /// its own tracks through the `album` column.
    ///
    /// Its tokenizer folds case *and* diacritics, so "echo" now finds "Écho",
    /// which the previous lowercase substring test did not. What it gives up is
    /// matching inside a word: "cho" no longer finds "Echo". The trailing term
    /// is treated as a prefix so search-as-you-type still works.
    pub async fn catalog_search(
        &self,
        user_id: Uuid,
        folder_ids: &[Uuid],
        query: &str,
    ) -> Result<CatalogSearch, ServiceError> {
        let Some(fts) = crate::catalog::fts_prefix_query(query) else {
            return Ok(CatalogSearch {
                artists: Vec::new(),
                albums: Vec::new(),
                songs: Vec::new(),
                album_tracks: Vec::new(),
            });
        };
        let folder_filter = (!folder_ids.is_empty()).then(|| {
            serde_json::to_string(folder_ids).expect("UUID list serialization cannot fail")
        });
        let folder_filter = folder_filter.as_deref();

        let songs = sqlx::query(concat!(
            song_select!(),
            " AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
               AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
             ORDER BY t.title COLLATE NOCASE, t.id"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;

        let albums = sqlx::query(concat!(
            album_select!(),
            " AND (? IS NULL OR al.library_id IN (SELECT value FROM json_each(?))) \
               AND al.id IN (SELECT t.album_id FROM track t WHERE t.album_id IS NOT NULL \
                 AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?)) \
             ORDER BY al.title COLLATE NOCASE, al.id"
        ))
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;

        let artists = sqlx::query(
            "SELECT ar.id, ar.library_id, ar.name, ar.artwork_hash, \
                    us.starred_at, ur.rating AS user_rating FROM artist ar \
             JOIN library_member m ON m.library_id=ar.library_id \
             LEFT JOIN user_star us ON us.user_id=m.user_id AND us.entity_type='artist' AND us.entity_id=ar.id \
             LEFT JOIN user_rating ur ON ur.user_id=m.user_id AND ur.entity_type='artist' AND ur.entity_id=ar.id \
             WHERE m.user_id=? AND (? IS NULL OR ar.library_id IN (SELECT value FROM json_each(?))) \
               AND ar.id IN ( \
                 SELECT al.album_artist_id FROM album al JOIN track t ON t.album_id=al.id \
                 WHERE al.album_artist_id IS NOT NULL \
                   AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
                 UNION \
                 SELECT ta.artist_id FROM track_artist ta \
                 WHERE ta.track_id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
               ) \
             ORDER BY ar.name COLLATE NOCASE",
        )
        .bind(user_id.to_string())
        .bind(folder_filter)
        .bind(folder_filter)
        .bind(&fts)
        .bind(&fts)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_from_row)
        .collect::<Result<Vec<_>, _>>()?;

        // Reload every track of the returned albums, so their songCount and
        // duration describe the album rather than the query.
        let album_tracks = if albums.is_empty() {
            Vec::new()
        } else {
            let ids =
                serde_json::to_string(&albums.iter().map(|album| album.id).collect::<Vec<_>>())
                    .expect("UUID list serialization cannot fail");
            sqlx::query(concat!(
                song_select!(),
                " AND t.album_id IN (SELECT value FROM json_each(?)) \
                 ORDER BY t.title COLLATE NOCASE, t.id"
            ))
            .bind(user_id.to_string())
            .bind(&ids)
            .fetch_all(self.db.pool())
            .await?
            .into_iter()
            .map(song_from_row)
            .collect::<Result<Vec<_>, _>>()?
        };

        Ok(CatalogSearch {
            artists,
            albums,
            songs,
            album_tracks,
        })
    }

    /// Albums visible to the user, paginated. Unlike [`Self::catalog_snapshot`],
    /// which materialises the whole catalogue for the Subsonic surface, this
    /// pages in SQL so a large library never loads in full to render one screen.
    pub async fn list_albums(
        &self,
        user_id: Uuid,
        library_id: Option<Uuid>,
        page: BrowsePage,
    ) -> Result<Vec<AlbumItem>, ServiceError> {
        let library = library_id.map(|id| id.to_string());
        Ok(sqlx::query(concat!(
            album_select!(),
            " AND (? IS NULL OR al.library_id=?) \
              ORDER BY al.title COLLATE NOCASE, al.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(library.as_deref())
        .bind(library.as_deref())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?)
    }

    /// One album with its tracks in sleeve order. Returns [`ServiceError::NotFound`]
    /// both when the album does not exist and when it belongs to a library the
    /// user cannot see, so the surface never leaks another tenant's catalogue.
    pub async fn album(&self, user_id: Uuid, album_id: Uuid) -> Result<AlbumDetail, ServiceError> {
        let album = sqlx::query(concat!(album_select!(), " AND al.id=?"))
            .bind(user_id.to_string())
            .bind(album_id.to_string())
            .fetch_optional(self.db.pool())
            .await?
            .map(album_from_row)
            .transpose()?
            .ok_or(ServiceError::NotFound)?;
        let songs = sqlx::query(concat!(
            song_select!(),
            // SQLite orders NULL first, which would put an untagged track ahead
            // of track 1. Incomplete disc/track tags are common in real
            // libraries, so unnumbered tracks sort to the end instead.
            " AND t.album_id=? \
              ORDER BY t.disc_number NULLS LAST, t.track_number NULLS LAST, \
                       t.title COLLATE NOCASE, t.id"
        ))
        .bind(user_id.to_string())
        .bind(album_id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(AlbumDetail { album, songs })
    }

    /// Artists visible to the user, paginated, each with its album count.
    pub async fn list_artists(
        &self,
        user_id: Uuid,
        library_id: Option<Uuid>,
        page: BrowsePage,
    ) -> Result<Vec<ArtistSummary>, ServiceError> {
        let library = library_id.map(|id| id.to_string());
        Ok(sqlx::query(concat!(
            artist_select!(),
            " AND (? IS NULL OR ar.library_id=?) \
              ORDER BY ar.name COLLATE NOCASE, ar.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(library.as_deref())
        .bind(library.as_deref())
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_summary_from_row)
        .collect::<Result<Vec<_>, _>>()?)
    }

    /// One artist with the albums it is credited on as album artist.
    pub async fn artist(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
    ) -> Result<ArtistDetail, ServiceError> {
        let summary = sqlx::query(concat!(artist_select!(), " AND ar.id=?"))
            .bind(user_id.to_string())
            .bind(artist_id.to_string())
            .fetch_optional(self.db.pool())
            .await?
            .map(artist_summary_from_row)
            .transpose()?
            .ok_or(ServiceError::NotFound)?;
        let albums = sqlx::query(concat!(
            album_select!(),
            " AND al.album_artist_id=? \
              ORDER BY al.year NULLS LAST, al.title COLLATE NOCASE, al.id"
        ))
        .bind(user_id.to_string())
        .bind(artist_id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(ArtistDetail {
            artist: summary.artist,
            album_count: summary.album_count,
            albums,
        })
    }

    /// Full-text search across the user's visible catalogue. Tracks are matched
    /// through the FTS5 index built in M1, which folds case and diacritics, so
    /// "echo" finds "Écho". Albums and artists are derived from the same index
    /// rather than a second scan, keeping one source of truth for relevance.
    pub async fn search(
        &self,
        user_id: Uuid,
        query: &str,
        page: BrowsePage,
    ) -> Result<SearchResult, ServiceError> {
        let Some(fts) = crate::catalog::fts_match_query(query) else {
            return Ok(SearchResult {
                artists: Vec::new(),
                albums: Vec::new(),
                songs: Vec::new(),
            });
        };
        let songs = sqlx::query(concat!(
            song_select!(),
            " AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?) \
              ORDER BY t.title COLLATE NOCASE, t.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(song_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let albums = sqlx::query(concat!(
            album_select!(),
            " AND al.id IN (SELECT t.album_id FROM track t \
                WHERE t.album_id IS NOT NULL \
                  AND t.id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?)) \
              ORDER BY al.title COLLATE NOCASE, al.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(album_from_row)
        .collect::<Result<Vec<_>, _>>()?;
        let artists = sqlx::query(concat!(
            artist_select!(),
            " AND ar.id IN (SELECT ta.artist_id FROM track_artist ta \
                WHERE ta.track_id IN (SELECT track_id FROM track_fts WHERE track_fts MATCH ?)) \
              ORDER BY ar.name COLLATE NOCASE, ar.id LIMIT ? OFFSET ?"
        ))
        .bind(user_id.to_string())
        .bind(&fts)
        .bind(page.limit)
        .bind(page.offset)
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(artist_summary_from_row)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .map(|summary| summary.artist)
        .collect();
        Ok(SearchResult {
            artists,
            albums,
            songs,
        })
    }

    /// Issues an authorization code for a native client.
    ///
    /// Validation, credential generation and persistence live here rather than
    /// in the handler so the grant rules hold for every surface that ever
    /// issues one, and so they can be exercised without an HTTP request.
    /// Returns the URL the consent screen must send the user agent to.
    pub async fn authorize_native_client(
        &self,
        user_id: Uuid,
        request: AuthorizationRequest<'_>,
    ) -> Result<String, ServiceError> {
        crate::oauth::validate_redirect_uri(request.redirect_uri)
            .map_err(|_| ServiceError::Invalid)?;
        crate::oauth::validate_challenge(request.code_challenge_method, request.code_challenge)
            .map_err(|_| ServiceError::Invalid)?;
        let client_id = request.client_id.trim();
        let device_name = request.device_name.trim();
        // Checked before the code exists: a name the session issuer would
        // reject must not burn a grant the client can never redeem.
        if client_id.is_empty() || device_name.is_empty() || device_name.len() > 120 {
            return Err(ServiceError::Invalid);
        }

        let code = security::generate_token("wfc_");
        let now = now_ms();
        self.db
            .create_authorization(crate::database::NewAuthorization {
                code_hash: security::token_hash(&code),
                user_id,
                client_id,
                redirect_uri: request.redirect_uri,
                code_challenge: request.code_challenge,
                device_name,
                now_ms: now,
                expires_at: now + crate::oauth::AUTHORIZATION_CODE_TTL_MS,
            })
            .await?;
        Ok(crate::oauth::redirect_with_code(
            request.redirect_uri,
            &code,
            request.state,
        ))
    }

    pub async fn songs_by_ids(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.songs_by_ids_on(&mut connection, user_id, ids).await
    }

    pub async fn sync_snapshot(
        &self,
        user_id: Uuid,
        history_limit: i64,
    ) -> Result<SyncSnapshotData, ServiceError> {
        let mut tx = self.db.pool().begin().await?;
        let cursor =
            sqlx::query_scalar("SELECT COALESCE(MAX(cursor), 0) FROM sync_event WHERE user_id=?")
                .bind(user_id.to_string())
                .fetch_one(&mut *tx)
                .await?;
        let playlists = self.playlists_on(&mut tx, user_id).await?;
        let favorites = self.starred_ids_on(&mut tx, user_id).await?;
        let ratings = self.ratings_on(&mut tx, user_id).await?;
        let queue = self.queue_on(&mut tx, user_id).await?;
        let history = self.history_on(&mut tx, user_id, history_limit).await?;
        let shares = self.shares_on(&mut tx, user_id).await?;
        tx.commit().await?;
        Ok(SyncSnapshotData {
            cursor,
            playlists,
            favorites,
            ratings,
            queue,
            history,
            shares,
        })
    }

    async fn songs_by_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        let songs = self
            .songs_by_ids_lenient_on(connection, user_id, ids)
            .await?;
        if songs.len() == ids.len() {
            Ok(songs)
        } else {
            Err(ServiceError::NotFound)
        }
    }

    async fn songs_by_ids_lenient_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<SongItem>, ServiceError> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let ids_json = serde_json::to_string(ids).map_err(|_| ServiceError::Invalid)?;
        let rows = sqlx::query(concat!(
            song_select!(),
            " AND t.id IN (SELECT value FROM json_each(?))"
        ))
        .bind(user_id.to_string())
        .bind(ids_json)
        .fetch_all(&mut *connection)
        .await?;
        let available = rows
            .into_iter()
            .map(song_from_row)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        Ok(ids
            .iter()
            .filter_map(|id| available.get(id).cloned())
            .collect())
    }

    pub async fn artwork_for_user(
        &self,
        user_id: Uuid,
        id: &str,
    ) -> Result<Option<(String, String)>, ServiceError> {
        let row = sqlx::query(
            "SELECT a.hash, a.format FROM artwork a WHERE a.hash=? AND EXISTS ( \
               SELECT 1 FROM track t JOIN library_member m ON m.library_id=t.library_id WHERE t.artwork_hash=a.hash AND m.user_id=? \
               UNION SELECT 1 FROM album al JOIN library_member m ON m.library_id=al.library_id WHERE al.artwork_hash=a.hash AND m.user_id=? \
               UNION SELECT 1 FROM artist ar JOIN library_member m ON m.library_id=ar.library_id WHERE ar.artwork_hash=a.hash AND m.user_id=? \
             ) UNION ALL SELECT a.hash, a.format FROM track t JOIN library_member m ON m.library_id=t.library_id JOIN artwork a ON a.hash=t.artwork_hash WHERE t.id=? AND m.user_id=? \
             UNION ALL SELECT a.hash, a.format FROM album al JOIN library_member m ON m.library_id=al.library_id JOIN artwork a ON a.hash=al.artwork_hash WHERE al.id=? AND m.user_id=? \
             UNION ALL SELECT a.hash, a.format FROM artist ar JOIN library_member m ON m.library_id=ar.library_id JOIN artwork a ON a.hash=ar.artwork_hash WHERE ar.id=? AND m.user_id=? LIMIT 1",
        )
        .bind(id).bind(user_id.to_string()).bind(user_id.to_string()).bind(user_id.to_string())
        .bind(id).bind(user_id.to_string()).bind(id).bind(user_id.to_string()).bind(id).bind(user_id.to_string())
        .fetch_optional(self.db.pool()).await?;
        row.map(|row| Ok::<_, sqlx::Error>((row.try_get("hash")?, row.try_get("format")?)))
            .transpose()
            .map_err(Into::into)
    }

    pub async fn playlists(&self, user_id: Uuid) -> Result<Vec<PlaylistItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlists_on(&mut connection, user_id).await
    }

    async fn playlists_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<PlaylistItem>, ServiceError> {
        let rows = sqlx::query(
            "SELECT id, name, comment, public, created_at, updated_at FROM playlist \
             WHERE owner_user_id=? ORDER BY updated_at DESC, id",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut result = Vec::with_capacity(rows.len());
        for row in rows {
            let id = parse_uuid(row.try_get("id")?)?;
            result.push(PlaylistItem {
                id,
                name: row.try_get("name")?,
                comment: row.try_get("comment")?,
                public: row.try_get::<i64, _>("public")? != 0,
                created_at: row.try_get("created_at")?,
                updated_at: row.try_get("updated_at")?,
                songs: self.playlist_songs_on(connection, user_id, id).await?,
            });
        }
        Ok(result)
    }

    pub async fn playlist(&self, user_id: Uuid, id: Uuid) -> Result<PlaylistItem, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.playlist_on(&mut connection, user_id, id).await
    }

    async fn playlist_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        id: Uuid,
    ) -> Result<PlaylistItem, ServiceError> {
        let row = sqlx::query(
            "SELECT id, name, comment, public, created_at, updated_at FROM playlist \
             WHERE id=? AND owner_user_id=?",
        )
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *connection)
        .await?
        .ok_or(ServiceError::NotFound)?;
        Ok(PlaylistItem {
            id,
            name: row.try_get("name")?,
            comment: row.try_get("comment")?,
            public: row.try_get::<i64, _>("public")? != 0,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            songs: self.playlist_songs_on(connection, user_id, id).await?,
        })
    }

    async fn playlist_songs_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<SongItem>, ServiceError> {
        let ids = self
            .playlist_track_ids_on(connection, user_id, playlist_id)
            .await?;
        self.songs_by_ids_lenient_on(connection, user_id, &ids)
            .await
    }

    async fn playlist_track_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        playlist_id: Uuid,
    ) -> Result<Vec<Uuid>, ServiceError> {
        sqlx::query_scalar::<_, String>(
            "SELECT pt.track_id FROM playlist_track pt JOIN playlist p ON p.id=pt.playlist_id \
             WHERE p.id=? AND p.owner_user_id=? ORDER BY pt.position",
        )
        .bind(playlist_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()
        .map_err(Into::into)
    }

    pub async fn create_playlist(
        &self,
        user_id: Uuid,
        name: &str,
        track_ids: &[Uuid],
    ) -> Result<PlaylistItem, ServiceError> {
        self.create_playlist_with_context(
            user_id,
            name,
            track_ids,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn create_playlist_with_context(
        &self,
        user_id: Uuid,
        name: &str,
        track_ids: &[Uuid],
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let intent = MutationIntent::new(
            "create",
            "playlist",
            &serde_json::json!({ "name": name.trim(), "track_ids": track_ids }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            let id = receipt.result_entity_id.ok_or(ServiceError::Conflict)?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        validate_name(name)?;
        self.songs_by_ids_on(&mut tx, user_id, track_ids).await?;
        let id = Uuid::new_v4();
        let now = now_ms();
        sqlx::query("INSERT INTO playlist (id, owner_user_id, name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
            .bind(id.to_string()).bind(user_id.to_string()).bind(name.trim()).bind(now).bind(now)
            .execute(&mut *tx).await?;
        replace_playlist_tracks(&mut tx, id, track_ids, now).await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": name.trim(),
                    "track_ids": track_ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        add: &[Uuid],
        remove_indexes: &[usize],
        clear: PlaylistClear,
    ) -> Result<PlaylistItem, ServiceError> {
        self.update_playlist_with_context(
            user_id,
            id,
            name,
            comment,
            public,
            add,
            remove_indexes,
            clear,
            MutationContext::server_generated(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn update_playlist_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        name: Option<&str>,
        comment: Option<&str>,
        public: Option<bool>,
        add: &[Uuid],
        remove_indexes: &[usize],
        clear: PlaylistClear,
        context: MutationContext,
    ) -> Result<PlaylistItem, ServiceError> {
        let mut removes = remove_indexes.to_vec();
        removes.sort_unstable_by(|a, b| b.cmp(a));
        removes.dedup();
        let intent = MutationIntent::new(
            "update",
            &format!("playlist:{id}"),
            &serde_json::json!({
                "name": name.map(str::trim),
                "comment": comment,
                "public": public,
                "add": add,
                "remove_indexes": &removes,
                "clear_comment": clear.comment,
            }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            drop(_writer);
            return self.playlist(user_id, id).await;
        }
        let current = self.playlist_on(&mut tx, user_id, id).await?;
        if let Some(name) = name {
            validate_name(name)?;
        }
        self.songs_by_ids_on(&mut tx, user_id, add).await?;
        let mut ids = self.playlist_track_ids_on(&mut tx, user_id, id).await?;
        for index in removes {
            if index >= ids.len() {
                return Err(ServiceError::Invalid);
            }
            ids.remove(index);
        }
        ids.extend_from_slice(add);
        let changed_at = now_ms();
        sqlx::query(
            "UPDATE playlist SET name=COALESCE(?, name), \
             comment=CASE WHEN ? THEN NULL ELSE COALESCE(?, comment) END, \
             public=COALESCE(?, public), updated_at=? WHERE id=? AND owner_user_id=?",
        )
        .bind(name.map(str::trim))
        .bind(clear.comment)
        .bind(comment)
        .bind(public.map(i64::from))
        .bind(changed_at)
        .bind(id.to_string())
        .bind(user_id.to_string())
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM playlist_track WHERE playlist_id=?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;
        replace_playlist_tracks(&mut tx, id, &ids, changed_at).await?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "playlist",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "name": name.map(str::trim).unwrap_or(&current.name),
                    "comment": comment.or(current.comment.as_deref()),
                    "public": public.unwrap_or(current.public),
                    "track_ids": ids,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.playlist(user_id, id).await
    }

    pub async fn delete_playlist(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        self.delete_playlist_with_context(user_id, id, MutationContext::server_generated())
            .await
    }

    pub async fn delete_playlist_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent =
            MutationIntent::new("delete", &format!("playlist:{id}"), &serde_json::json!({}));
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "playlist")?;
            return Ok(());
        }
        let changed = sqlx::query("DELETE FROM playlist WHERE id=? AND owner_user_id=?")
            .bind(id.to_string())
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if changed == 0 {
            tx.rollback().await?;
            Err(ServiceError::NotFound)
        } else {
            let receipt = self
                .sync
                .complete_operation(
                    &mut tx,
                    user_id,
                    context,
                    "playlist",
                    id,
                    "delete",
                    &serde_json::json!({}),
                    Some(id),
                )
                .await?;
            tx.commit().await?;
            self.sync.publish(user_id, receipt);
            Ok(())
        }
    }

    pub async fn set_star(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        starred: bool,
    ) -> Result<(), ServiceError> {
        self.set_star_with_context(
            user_id,
            entity_type,
            entity_id,
            starred,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn set_star_with_context(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        starred: bool,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            if starred { "star" } else { "unstar" },
            &format!("{entity_type}:{entity_id}"),
            &serde_json::json!({ "starred": starred }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "favorite")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, entity_type, entity_id)
            .await?;
        if starred {
            sqlx::query("INSERT INTO user_star (user_id, entity_type, entity_id, starred_at) VALUES (?, ?, ?, ?) ON CONFLICT DO UPDATE SET starred_at=excluded.starred_at")
                .bind(user_id.to_string()).bind(entity_type).bind(entity_id.to_string()).bind(now_ms())
                .execute(&mut *tx).await?;
        } else {
            sqlx::query("DELETE FROM user_star WHERE user_id=? AND entity_type=? AND entity_id=?")
                .bind(user_id.to_string())
                .bind(entity_type)
                .bind(entity_id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "favorite",
                entity_id,
                if starred { "upsert" } else { "delete" },
                &serde_json::json!({
                    "entity_type": entity_type,
                    "entity_id": entity_id,
                    "starred": starred,
                }),
                Some(entity_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn entity_kind(
        &self,
        user_id: Uuid,
        entity_id: Uuid,
    ) -> Result<Option<&'static str>, ServiceError> {
        let kinds: Vec<String> = sqlx::query_scalar(
            "SELECT entity_type FROM (\
               SELECT 'track' AS entity_type FROM track t \
                 JOIN library_member m ON m.library_id=t.library_id \
                 WHERE t.id=? AND m.user_id=? \
               UNION ALL \
               SELECT 'album' FROM album a \
                 JOIN library_member m ON m.library_id=a.library_id \
                 WHERE a.id=? AND m.user_id=? \
               UNION ALL \
               SELECT 'artist' FROM artist ar \
                 JOIN library_member m ON m.library_id=ar.library_id \
                 WHERE ar.id=? AND m.user_id=? \
             )",
        )
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .bind(entity_id.to_string())
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        if kinds.len() != 1 {
            return Ok(None);
        }
        Ok(match kinds[0].as_str() {
            "track" => Some("track"),
            "album" => Some("album"),
            "artist" => Some("artist"),
            _ => None,
        })
    }

    pub async fn starred_ids(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<(String, Uuid, i64)>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.starred_ids_on(&mut connection, user_id).await
    }

    async fn starred_ids_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<(String, Uuid, i64)>, ServiceError> {
        sqlx::query(
            "SELECT s.entity_type, s.entity_id, s.starred_at FROM user_star s \
             WHERE s.user_id=? AND ( \
               (s.entity_type='track' AND EXISTS (SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) OR \
               (s.entity_type='album' AND EXISTS (SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) OR \
               (s.entity_type='artist' AND EXISTS (SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=s.entity_id AND m.user_id=s.user_id)) \
             ) ORDER BY s.starred_at DESC",
        )
            .bind(user_id.to_string()).fetch_all(&mut *connection).await?
            .into_iter().map(|row| Ok((row.try_get("entity_type")?, parse_uuid(row.try_get("entity_id")?)?, row.try_get("starred_at")?)))
            .collect::<Result<Vec<_>, sqlx::Error>>().map_err(Into::into)
    }

    pub async fn ratings(&self, user_id: Uuid) -> Result<Vec<RatingItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.ratings_on(&mut connection, user_id).await
    }

    async fn ratings_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<RatingItem>, ServiceError> {
        sqlx::query(
            "SELECT r.entity_type, r.entity_id, r.rating, r.updated_at FROM user_rating r \
             WHERE r.user_id=? AND ( \
               (r.entity_type='track' AND EXISTS (SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) OR \
               (r.entity_type='album' AND EXISTS (SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) OR \
               (r.entity_type='artist' AND EXISTS (SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=r.entity_id AND m.user_id=r.user_id)) \
             ) ORDER BY r.updated_at DESC, r.entity_type, r.entity_id",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok(RatingItem {
                entity_type: row.try_get("entity_type")?,
                entity_id: parse_uuid(row.try_get("entity_id")?)?,
                rating: row.try_get("rating")?,
                updated_at: row.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
    }

    pub async fn set_rating(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        rating: i64,
    ) -> Result<(), ServiceError> {
        self.set_rating_with_context(
            user_id,
            entity_type,
            entity_id,
            rating,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn set_rating_with_context(
        &self,
        user_id: Uuid,
        entity_type: &str,
        entity_id: Uuid,
        rating: i64,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            "set-rating",
            &format!("{entity_type}:{entity_id}"),
            &serde_json::json!({ "rating": rating }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "rating")?;
            return Ok(());
        }
        if !(0..=5).contains(&rating) {
            return Err(ServiceError::Invalid);
        }
        self.authorize_entity_on(&mut tx, user_id, entity_type, entity_id)
            .await?;
        if rating == 0 {
            sqlx::query(
                "DELETE FROM user_rating WHERE user_id=? AND entity_type=? AND entity_id=?",
            )
            .bind(user_id.to_string())
            .bind(entity_type)
            .bind(entity_id.to_string())
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query("INSERT INTO user_rating (user_id, entity_type, entity_id, rating, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT DO UPDATE SET rating=excluded.rating, updated_at=excluded.updated_at")
                .bind(user_id.to_string()).bind(entity_type).bind(entity_id.to_string()).bind(rating).bind(now_ms()).execute(&mut *tx).await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "rating",
                entity_id,
                if rating == 0 { "delete" } else { "upsert" },
                &serde_json::json!({
                    "entity_type": entity_type,
                    "entity_id": entity_id,
                    "rating": rating,
                }),
                Some(entity_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn scrobble(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        submission: bool,
        time: Option<i64>,
    ) -> Result<(), ServiceError> {
        self.scrobble_with_context(
            user_id,
            track_id,
            submission,
            time,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn scrobble_with_context(
        &self,
        user_id: Uuid,
        track_id: Uuid,
        submission: bool,
        time: Option<i64>,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            if submission {
                "scrobble"
            } else {
                "now-playing"
            },
            &format!("track:{track_id}"),
            &serde_json::json!({ "submission": submission, "time": time }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "scrobble")?;
            return Ok(());
        }
        self.authorize_entity_on(&mut tx, user_id, "track", track_id)
            .await?;
        let current_time = now_ms();
        let now = time.unwrap_or(current_time);
        const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1_000;
        if now < 0 || now > current_time.saturating_add(MAX_FUTURE_SKEW_MS) {
            return Err(ServiceError::Invalid);
        }
        sqlx::query(
            "INSERT INTO play_event (user_id, track_id, submission, played_at) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(track_id.to_string())
        .bind(i64::from(submission))
        .bind(now)
        .execute(&mut *tx)
        .await?;
        if submission {
            sqlx::query("DELETE FROM now_playing WHERE user_id=?")
                .bind(user_id.to_string())
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("INSERT INTO now_playing (user_id, track_id, started_at, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT (user_id) DO UPDATE SET track_id=excluded.track_id, started_at=excluded.started_at, updated_at=excluded.updated_at")
                .bind(user_id.to_string()).bind(track_id.to_string()).bind(now).bind(now_ms()).execute(&mut *tx).await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "scrobble",
                track_id,
                if submission { "append" } else { "upsert" },
                &serde_json::json!({
                    "track_id": track_id,
                    "submission": submission,
                    "played_at": now,
                }),
                Some(track_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn now_playing(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<(String, SongItem, i64)>, ServiceError> {
        let rows = sqlx::query(
            "SELECT a.username, n.track_id, n.started_at FROM now_playing n \
             JOIN account a ON a.id=n.user_id WHERE a.disabled=0 ORDER BY n.started_at DESC",
        )
        .fetch_all(self.db.pool())
        .await?;
        let mut result = Vec::new();
        for row in rows {
            let id = parse_uuid(row.try_get("track_id")?)?;
            match self.songs_by_ids(user_id, &[id]).await {
                Ok(mut songs) => {
                    if let Some(song) = songs.pop() {
                        result.push((row.try_get("username")?, song, row.try_get("started_at")?));
                    }
                }
                Err(ServiceError::NotFound) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }

    pub async fn history(
        &self,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<HistoryItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.history_on(&mut connection, user_id, limit).await
    }

    async fn history_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        limit: i64,
    ) -> Result<Vec<HistoryItem>, ServiceError> {
        if !(0..=MAX_HISTORY_LIMIT).contains(&limit) {
            return Err(ServiceError::Invalid);
        }
        sqlx::query(
            "SELECT p.track_id, p.submission, p.played_at FROM play_event p \
             JOIN track t ON t.id=p.track_id JOIN library_member m ON m.library_id=t.library_id \
             WHERE p.user_id=? AND m.user_id=? ORDER BY p.played_at DESC, p.id DESC LIMIT ?",
        )
        .bind(user_id.to_string())
        .bind(user_id.to_string())
        .bind(limit)
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(|row| {
            Ok(HistoryItem {
                track_id: parse_uuid(row.try_get("track_id")?)?,
                submission: row.try_get::<i64, _>("submission")? != 0,
                played_at: row.try_get("played_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(Into::into)
    }

    pub async fn save_queue(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        current: Option<Uuid>,
        position_ms: i64,
        client: Option<&str>,
    ) -> Result<(), ServiceError> {
        self.save_queue_with_context(
            user_id,
            ids,
            current,
            position_ms,
            client,
            MutationContext::server_generated(),
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn save_queue_with_context(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        current: Option<Uuid>,
        position_ms: i64,
        client: Option<&str>,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new(
            "save",
            &format!("queue:{user_id}"),
            &serde_json::json!({
                "track_ids": ids,
                "current": current,
                "position_ms": position_ms,
                "client": client,
            }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "queue")?;
            return Ok(());
        }
        if ids.len() > MAX_QUEUE_TRACKS {
            return Err(ServiceError::Invalid);
        }
        if position_ms < 0 {
            return Err(ServiceError::Invalid);
        }
        self.songs_by_ids_on(&mut tx, user_id, ids).await?;
        if let Some(current) = current {
            self.songs_by_ids_on(&mut tx, user_id, &[current]).await?;
        }
        sqlx::query("INSERT INTO play_queue (user_id, current_track_id, position_ms, changed_by, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (user_id) DO UPDATE SET current_track_id=excluded.current_track_id, position_ms=excluded.position_ms, changed_by=excluded.changed_by, updated_at=excluded.updated_at")
            .bind(user_id.to_string()).bind(current.map(|id| id.to_string())).bind(position_ms).bind(client).bind(now_ms()).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM play_queue_track WHERE user_id=?")
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?;
        if !ids.is_empty() {
            let ids_json = serde_json::to_string(ids).map_err(|_| ServiceError::Invalid)?;
            sqlx::query(
                "INSERT INTO play_queue_track (user_id, track_id, position) \
                 SELECT ?, value, CAST(key AS INTEGER) FROM json_each(?)",
            )
            .bind(user_id.to_string())
            .bind(ids_json)
            .execute(&mut *tx)
            .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "queue",
                user_id,
                "upsert",
                &serde_json::json!({
                    "track_ids": ids,
                    "current": current,
                    "position_ms": position_ms,
                    "client": client,
                }),
                Some(user_id),
            )
            .await?;
        tx.commit().await?;
        self.sync.publish(user_id, receipt);
        Ok(())
    }

    pub async fn queue(&self, user_id: Uuid) -> Result<Option<QueueItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.queue_on(&mut connection, user_id).await
    }

    async fn queue_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Option<QueueItem>, ServiceError> {
        let row = sqlx::query("SELECT current_track_id, position_ms, changed_by, updated_at FROM play_queue WHERE user_id=?")
            .bind(user_id.to_string()).fetch_optional(&mut *connection).await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT track_id FROM play_queue_track WHERE user_id=? ORDER BY position",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(QueueItem {
            current: row
                .try_get::<Option<String>, _>("current_track_id")?
                .map(parse_uuid)
                .transpose()?,
            position_ms: row.try_get("position_ms")?,
            changed_by: row.try_get("changed_by")?,
            updated_at: row.try_get("updated_at")?,
            songs: self
                .songs_by_ids_lenient_on(connection, user_id, &ids)
                .await?,
        }))
    }

    pub async fn shares(&self, user_id: Uuid) -> Result<Vec<ShareItem>, ServiceError> {
        let mut connection = self.db.pool().acquire().await?;
        self.shares_on(&mut connection, user_id).await
    }

    async fn shares_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
    ) -> Result<Vec<ShareItem>, ServiceError> {
        let rows = sqlx::query("SELECT id, description, expires_at, created_at, visit_count FROM share WHERE owner_user_id=? ORDER BY created_at DESC")
            .bind(user_id.to_string()).fetch_all(&mut *connection).await?;
        let track_rows = sqlx::query(
            "SELECT st.share_id, st.track_id FROM share_track st \
             JOIN share s ON s.id=st.share_id WHERE s.owner_user_id=? \
             ORDER BY st.share_id, st.position",
        )
        .bind(user_id.to_string())
        .fetch_all(&mut *connection)
        .await?;
        let mut track_owners = Vec::with_capacity(track_rows.len());
        let mut track_ids = Vec::with_capacity(track_rows.len());
        for track_row in track_rows {
            track_owners.push(parse_uuid(track_row.try_get("share_id")?)?);
            track_ids.push(parse_uuid(track_row.try_get("track_id")?)?);
        }
        let songs = self
            .songs_by_ids_lenient_on(connection, user_id, &track_ids)
            .await?
            .into_iter()
            .map(|song| (song.id, song))
            .collect::<HashMap<_, _>>();
        let mut songs_by_share = HashMap::<Uuid, Vec<SongItem>>::new();
        for (share_id, track_id) in track_owners.into_iter().zip(track_ids) {
            if let Some(song) = songs.get(&track_id) {
                songs_by_share
                    .entry(share_id)
                    .or_default()
                    .push(song.clone());
            }
        }

        let mut shares = Vec::with_capacity(rows.len());
        for row in rows {
            let id = parse_uuid(row.try_get("id")?)?;
            shares.push(ShareItem {
                id,
                owner_id: user_id,
                url_token: None,
                description: row.try_get("description")?,
                expires_at: row.try_get("expires_at")?,
                created_at: row.try_get("created_at")?,
                visit_count: row.try_get("visit_count")?,
                songs: songs_by_share.remove(&id).unwrap_or_default(),
            });
        }
        Ok(shares)
    }

    pub async fn create_share(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        description: Option<&str>,
        expires_at: Option<i64>,
    ) -> Result<ShareItem, ServiceError> {
        self.create_share_with_context(
            user_id,
            ids,
            description,
            expires_at,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn create_share_with_context(
        &self,
        user_id: Uuid,
        ids: &[Uuid],
        description: Option<&str>,
        expires_at: Option<i64>,
        context: MutationContext,
    ) -> Result<ShareItem, ServiceError> {
        let intent = MutationIntent::new(
            "create",
            "share",
            &serde_json::json!({
                "track_ids": ids,
                "description": description,
                "expires_at": expires_at,
            }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "share")?;
            let id = receipt.result_entity_id.ok_or(ServiceError::Conflict)?;
            drop(_writer);
            let mut share = self
                .shares(user_id)
                .await?
                .into_iter()
                .find(|share| share.id == id)
                .ok_or(ServiceError::NotFound)?;
            share.url_token = Some(self.secret_box.derive_share_token(id));
            return Ok(share);
        }
        if ids.is_empty() || ids.len() > MAX_SHARE_TRACKS {
            return Err(ServiceError::Invalid);
        }
        let songs = self.songs_by_ids_on(&mut tx, user_id, ids).await?;
        let id = Uuid::new_v4();
        let token = self.secret_box.derive_share_token(id);
        let token_hash = security::token_hash(&token);
        let now = now_ms();
        sqlx::query("INSERT INTO share (id, owner_user_id, token_hash, description, expires_at, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?)")
            .bind(id.to_string()).bind(user_id.to_string()).bind(token_hash.as_slice()).bind(description).bind(expires_at).bind(now).bind(now).execute(&mut *tx).await?;
        for (position, track) in ids.iter().enumerate() {
            sqlx::query("INSERT INTO share_track (share_id, track_id, position) VALUES (?, ?, ?)")
                .bind(id.to_string())
                .bind(track.to_string())
                .bind(position as i64)
                .execute(&mut *tx)
                .await?;
        }
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "share",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "track_ids": ids,
                    "description": description,
                    "expires_at": expires_at,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        Ok(ShareItem {
            id,
            owner_id: user_id,
            url_token: Some(token),
            description: description.map(str::to_owned),
            expires_at,
            created_at: now,
            visit_count: 0,
            songs,
        })
    }

    pub async fn public_share(&self, token: &str) -> Result<ShareItem, ServiceError> {
        let hash = security::token_hash(token);
        let row = sqlx::query("SELECT id, owner_user_id, description, expires_at, created_at, visit_count FROM share WHERE token_hash=? AND (expires_at IS NULL OR expires_at>?)")
            .bind(hash.as_slice()).bind(now_ms()).fetch_optional(self.db.pool()).await?.ok_or(ServiceError::NotFound)?;
        let id = parse_uuid(row.try_get("id")?)?;
        let owner = parse_uuid(row.try_get("owner_user_id")?)?;
        let ids = sqlx::query_scalar::<_, String>(
            "SELECT track_id FROM share_track WHERE share_id=? ORDER BY position",
        )
        .bind(id.to_string())
        .fetch_all(self.db.pool())
        .await?
        .into_iter()
        .map(parse_uuid)
        .collect::<Result<Vec<_>, _>>()?;
        // The rows above were read outside the writer gate, and acquiring it can
        // block behind a scan. Re-check both revocation and expiry at write
        // time: no affected row means the share died during that wait, and a
        // visitor must not see what an owner deleted or let expire.
        let _writer = self.db.writer_guard().await;
        let visited_at = now_ms();
        let visited = sqlx::query(
            "UPDATE share SET visit_count=visit_count+1, last_visited_at=? \
             WHERE id=? AND (expires_at IS NULL OR expires_at>?)",
        )
        .bind(visited_at)
        .bind(id.to_string())
        .bind(visited_at)
        .execute(self.db.pool())
        .await?
        .rows_affected();
        drop(_writer);
        if visited == 0 {
            return Err(ServiceError::NotFound);
        }
        Ok(ShareItem {
            id,
            owner_id: owner,
            url_token: None,
            description: row.try_get("description")?,
            expires_at: row.try_get("expires_at")?,
            created_at: row.try_get("created_at")?,
            visit_count: row.try_get::<i64, _>("visit_count")? + 1,
            songs: self.songs_by_ids(owner, &ids).await?,
        })
    }

    pub async fn update_share(
        &self,
        user_id: Uuid,
        id: Uuid,
        description: Option<&str>,
        expires_at: Option<i64>,
        clear: ShareClear,
    ) -> Result<ShareItem, ServiceError> {
        self.update_share_with_context(
            user_id,
            id,
            description,
            expires_at,
            clear,
            MutationContext::server_generated(),
        )
        .await
    }

    pub async fn update_share_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        description: Option<&str>,
        expires_at: Option<i64>,
        clear: ShareClear,
        context: MutationContext,
    ) -> Result<ShareItem, ServiceError> {
        // Clearing must be part of the intent: "set expiry to X" and "remove the
        // expiry" are different mutations, and an operation id replayed across
        // both has to be rejected rather than silently treated as the same.
        let intent = MutationIntent::new(
            "update",
            &format!("share:{id}"),
            &serde_json::json!({
                "description": description,
                "expires_at": expires_at,
                "clear_description": clear.description,
                "clear_expires_at": clear.expires_at,
            }),
        );
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "share")?;
            drop(_writer);
            return self
                .shares(user_id)
                .await?
                .into_iter()
                .find(|share| share.id == id)
                .ok_or(ServiceError::NotFound);
        }
        let persisted = sqlx::query(
            "UPDATE share SET \
               description=CASE WHEN ? THEN NULL ELSE COALESCE(?, description) END, \
               expires_at=CASE WHEN ? THEN NULL ELSE COALESCE(?, expires_at) END, \
               updated_at=? \
             WHERE id=? AND owner_user_id=? RETURNING description, expires_at",
        )
        .bind(clear.description)
        .bind(description)
        .bind(clear.expires_at)
        .bind(expires_at)
        .bind(now_ms())
        .bind(id.to_string())
        .bind(user_id.to_string())
        .fetch_optional(&mut *tx)
        .await?;
        let Some(persisted) = persisted else {
            tx.rollback().await?;
            return Err(ServiceError::NotFound);
        };
        let persisted_description: Option<String> = persisted.try_get("description")?;
        let persisted_expires_at: Option<i64> = persisted.try_get("expires_at")?;
        let receipt = self
            .sync
            .complete_operation(
                &mut tx,
                user_id,
                context,
                "share",
                id,
                "upsert",
                &serde_json::json!({
                    "id": id,
                    "description": persisted_description,
                    "expires_at": persisted_expires_at,
                }),
                Some(id),
            )
            .await?;
        tx.commit().await?;
        drop(_writer);
        self.sync.publish(user_id, receipt);
        self.shares(user_id)
            .await?
            .into_iter()
            .find(|share| share.id == id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn delete_share(&self, user_id: Uuid, id: Uuid) -> Result<(), ServiceError> {
        self.delete_share_with_context(user_id, id, MutationContext::server_generated())
            .await
    }

    pub async fn delete_share_with_context(
        &self,
        user_id: Uuid,
        id: Uuid,
        context: MutationContext,
    ) -> Result<(), ServiceError> {
        let intent = MutationIntent::new("delete", &format!("share:{id}"), &serde_json::json!({}));
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        if let OperationClaim::Replayed(receipt) = self
            .sync
            .claim_operation(&_writer, &mut tx, user_id, context, intent)
            .await?
        {
            tx.rollback().await?;
            validate_replay_type(&receipt, "share")?;
            return Ok(());
        }
        let changed = sqlx::query("DELETE FROM share WHERE id=? AND owner_user_id=?")
            .bind(id.to_string())
            .bind(user_id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if changed == 0 {
            tx.rollback().await?;
            Err(ServiceError::NotFound)
        } else {
            let receipt = self
                .sync
                .complete_operation(
                    &mut tx,
                    user_id,
                    context,
                    "share",
                    id,
                    "delete",
                    &serde_json::json!({}),
                    Some(id),
                )
                .await?;
            tx.commit().await?;
            self.sync.publish(user_id, receipt);
            Ok(())
        }
    }

    pub async fn users(&self, actor_id: Uuid) -> Result<Vec<UserItem>, ServiceError> {
        self.require_admin(actor_id).await?;
        let mut users = sqlx::query("SELECT a.id, a.username, a.role, a.disabled, c.user_id IS NOT NULL AS has_credential FROM account a LEFT JOIN subsonic_credential c ON c.user_id=a.id ORDER BY a.username COLLATE NOCASE")
            .fetch_all(self.db.pool()).await?.into_iter().map(|row| Ok(UserItem { id: parse_uuid(row.try_get("id")?)?, username: row.try_get("username")?, role: AccountRole::from_str(row.try_get::<&str, _>("role")?).map_err(|error| sqlx::Error::Decode(error.into()))?, disabled: row.try_get::<i64, _>("disabled")? != 0, has_subsonic_credential: row.try_get::<i64, _>("has_credential")? != 0, folder_ids: Vec::new() })).collect::<Result<Vec<_>, sqlx::Error>>()?;
        let memberships = sqlx::query(
            "SELECT user_id, library_id FROM library_member ORDER BY user_id, library_id",
        )
        .fetch_all(self.db.pool())
        .await?;
        for row in memberships {
            let user_id = parse_uuid(row.try_get("user_id")?)?;
            let library_id = parse_uuid(row.try_get("library_id")?)?;
            if let Some(user) = users.iter_mut().find(|user| user.id == user_id) {
                user.folder_ids.push(library_id);
            }
        }
        Ok(users)
    }

    pub async fn create_web_user(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
        role: AccountRole,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        validate_username(username)?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let password = password.to_owned();
        let password_hash = tokio::task::spawn_blocking(move || security::hash_password(&password))
            .await
            .map_err(|_| ServiceError::Unavailable)??;
        let id = self
            .db
            .create_account(username.trim(), &password_hash, role, now_ms())
            .await
            .map_err(|error| {
                if matches!(error, sqlx::Error::Database(ref db) if db.is_unique_violation()) {
                    ServiceError::Conflict
                } else {
                    ServiceError::Database(error)
                }
            })?;
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == id)
            .ok_or(ServiceError::NotFound)
    }

    /// Sets a dedicated Subsonic password and rotates the API key. The clear
    /// API key is returned once; only its hash is persisted.
    pub async fn set_subsonic_credential(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
    ) -> Result<String, ServiceError> {
        self.require_admin(actor_id).await?;
        if password.len() < 12 {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let api_key = security::generate_token("wfsk_");
        let api_key_hash = security::token_hash(&api_key);
        self.db
            .set_subsonic_credential(actor_id, account.id, &encrypted, &api_key_hash, now_ms())
            .await?;
        Ok(api_key)
    }

    pub async fn revoke_subsonic_credential(
        &self,
        actor_id: Uuid,
        username: &str,
    ) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if self
            .db
            .revoke_subsonic_credential(actor_id, account.id, now_ms())
            .await?
        {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }

    pub async fn create_subsonic_user(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
        admin: bool,
        folder_ids: Option<&[Uuid]>,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        validate_name(username)?;
        if password.is_empty() {
            return Err(ServiceError::Invalid);
        }
        let placeholder = security::generate_token("web-disabled-");
        let password_hash =
            tokio::task::spawn_blocking(move || security::hash_password(&placeholder))
                .await
                .map_err(|_| ServiceError::Unavailable)??;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let api_key = security::generate_token("wfsk_");
        let api_key_hash = security::token_hash(&api_key);
        let requested_folders = self.resolve_library_ids(folder_ids).await?;
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let user_id = Uuid::new_v4();
        let now = now_ms();
        let insert = sqlx::query(
            "INSERT INTO account (id, username, password_hash, role, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(username.trim())
        .bind(password_hash)
        .bind(if admin { "admin" } else { "user" })
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await;
        if let Err(error) = insert {
            return Err(
                if matches!(error, sqlx::Error::Database(ref db) if db.is_unique_violation()) {
                    ServiceError::Conflict
                } else {
                    ServiceError::Database(error)
                },
            );
        }
        sqlx::query(
            "INSERT INTO subsonic_credential \
             (user_id, password_nonce, password_ciphertext, api_key_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(user_id.to_string())
        .bind(encrypted.nonce.as_slice())
        .bind(encrypted.ciphertext)
        .bind(api_key_hash.as_slice())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        for library_id in requested_folders {
            sqlx::query(
                "INSERT INTO library_member (library_id, user_id, role, created_at) \
                 VALUES (?, ?, 'listener', ?)",
            )
            .bind(library_id.to_string())
            .bind(user_id.to_string())
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO audit_event (actor_user_id, kind, subject_id, occurred_at) \
             VALUES (?, 'subsonic.user_created', ?, ?)",
        )
        .bind(actor_id.to_string())
        .bind(user_id.to_string())
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        drop(_writer);
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == user_id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn update_user(
        &self,
        actor_id: Uuid,
        username: &str,
        update: UserUpdate<'_>,
    ) -> Result<UserItem, ServiceError> {
        self.require_admin(actor_id).await?;
        if update.subsonic_password.is_some_and(str::is_empty)
            || update
                .web_password
                .is_some_and(|password| password.len() < 12)
        {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if account.id == actor_id && (update.admin == Some(false) || update.disabled == Some(true))
        {
            return Err(ServiceError::Forbidden);
        }
        let requested_folders = match update.folder_ids {
            Some(ids) => Some(self.resolve_library_ids(Some(ids)).await?),
            None => None,
        };
        let encrypted = update
            .subsonic_password
            .map(|password| self.secret_box.encrypt(password.as_bytes()))
            .transpose()?;
        let web_password_hash = if let Some(password) = update.web_password {
            let password = password.to_owned();
            Some(
                tokio::task::spawn_blocking(move || security::hash_password(&password))
                    .await
                    .map_err(|_| ServiceError::Unavailable)??,
            )
        } else {
            None
        };
        let revoke_sessions = web_password_hash.is_some();
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        sqlx::query("UPDATE account SET role=COALESCE(?, role), disabled=COALESCE(?, disabled), password_hash=COALESCE(?, password_hash), updated_at=? WHERE id=?")
            .bind(update.admin.map(|value| if value { "admin" } else { "user" })).bind(update.disabled.map(i64::from)).bind(web_password_hash.as_deref()).bind(now_ms()).bind(account.id.to_string()).execute(&mut *tx).await?;
        if revoke_sessions {
            sqlx::query("UPDATE session SET revoked_at=? WHERE user_id=? AND revoked_at IS NULL")
                .bind(now_ms())
                .bind(account.id.to_string())
                .execute(&mut *tx)
                .await?;
        }
        if let Some(encrypted) = encrypted {
            let changed = sqlx::query(
                "UPDATE subsonic_credential SET password_nonce=?, password_ciphertext=?, updated_at=? WHERE user_id=?",
            )
            .bind(encrypted.nonce.as_slice())
            .bind(encrypted.ciphertext)
            .bind(now_ms())
            .bind(account.id.to_string())
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if changed == 0 {
                return Err(ServiceError::NotFound);
            }
        }
        if let Some(folder_ids) = requested_folders {
            sqlx::query("DELETE FROM library_member WHERE user_id=? AND role='listener'")
                .bind(account.id.to_string())
                .execute(&mut *tx)
                .await?;
            for library_id in folder_ids {
                sqlx::query(
                    "INSERT INTO library_member (library_id, user_id, role, created_at) \
                     VALUES (?, ?, 'listener', ?) \
                     ON CONFLICT (library_id, user_id) DO NOTHING",
                )
                .bind(library_id.to_string())
                .bind(account.id.to_string())
                .bind(now_ms())
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        drop(_writer);
        self.users(actor_id)
            .await?
            .into_iter()
            .find(|user| user.id == account.id)
            .ok_or(ServiceError::NotFound)
    }

    pub async fn delete_user(&self, actor_id: Uuid, username: &str) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        if account.id == actor_id {
            return Err(ServiceError::Forbidden);
        }
        let _writer = self.db.writer_guard().await;
        sqlx::query("DELETE FROM account WHERE id=?")
            .bind(account.id.to_string())
            .execute(self.db.pool())
            .await?;
        Ok(())
    }

    pub async fn change_subsonic_password(
        &self,
        actor_id: Uuid,
        username: &str,
        password: &str,
    ) -> Result<(), ServiceError> {
        self.require_admin(actor_id).await?;
        if password.is_empty() {
            return Err(ServiceError::Invalid);
        }
        let account = self
            .db
            .account_by_username(username)
            .await?
            .ok_or(ServiceError::NotFound)?;
        let encrypted = self.secret_box.encrypt(password.as_bytes())?;
        let _writer = self.db.writer_guard().await;
        let changed = sqlx::query("UPDATE subsonic_credential SET password_nonce=?, password_ciphertext=?, updated_at=? WHERE user_id=?")
            .bind(encrypted.nonce.as_slice()).bind(encrypted.ciphertext).bind(now_ms()).bind(account.id.to_string()).execute(self.db.pool()).await?.rows_affected();
        if changed == 0 {
            Err(ServiceError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn require_admin(&self, actor_id: Uuid) -> Result<(), ServiceError> {
        let account = self
            .db
            .account_by_id(actor_id)
            .await?
            .ok_or(ServiceError::Forbidden)?;
        if account.role == AccountRole::Admin && !account.disabled {
            Ok(())
        } else {
            Err(ServiceError::Forbidden)
        }
    }

    async fn resolve_library_ids(
        &self,
        requested: Option<&[Uuid]>,
    ) -> Result<Vec<Uuid>, ServiceError> {
        let available = sqlx::query_scalar::<_, String>("SELECT id FROM library ORDER BY id")
            .fetch_all(self.db.pool())
            .await?
            .into_iter()
            .map(parse_uuid)
            .collect::<Result<Vec<_>, _>>()?;
        let Some(requested) = requested else {
            return Ok(available);
        };
        let mut unique = Vec::new();
        for id in requested {
            if !available.contains(id) {
                return Err(ServiceError::NotFound);
            }
            if !unique.contains(id) {
                unique.push(*id);
            }
        }
        Ok(unique)
    }

    async fn authorize_entity_on(
        &self,
        connection: &mut SqliteConnection,
        user_id: Uuid,
        kind: &str,
        id: Uuid,
    ) -> Result<(), ServiceError> {
        let query = match kind {
            "track" => "SELECT 1 FROM track e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=?",
            "album" => "SELECT 1 FROM album e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=?",
            "artist" => "SELECT 1 FROM artist e JOIN library_member m ON m.library_id=e.library_id WHERE e.id=? AND m.user_id=?",
            _ => return Err(ServiceError::Invalid),
        };
        let exists = sqlx::query_scalar::<_, i64>(query)
            .bind(id.to_string())
            .bind(user_id.to_string())
            .fetch_optional(&mut *connection)
            .await?;
        if exists.is_some() {
            Ok(())
        } else {
            Err(ServiceError::NotFound)
        }
    }
}

async fn fetch_songs(
    db: &Database,
    user_id: Uuid,
    folder_filter: Option<&str>,
    id: Option<Uuid>,
) -> Result<Vec<SongItem>, sqlx::Error> {
    let id = id.map(|id| id.to_string());
    sqlx::query(concat!(
        song_select!(),
        " AND (? IS NULL OR t.library_id IN (SELECT value FROM json_each(?))) \
           AND (? IS NULL OR t.id=?) ORDER BY t.title COLLATE NOCASE"
    ))
    .bind(user_id.to_string())
    .bind(folder_filter)
    .bind(folder_filter)
    .bind(id.as_deref())
    .bind(id.as_deref())
    .fetch_all(db.pool())
    .await?
    .into_iter()
    .map(song_from_row)
    .collect()
}

fn credential_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<SubsonicCredentialRecord, sqlx::Error> {
    let nonce = row.try_get::<Vec<u8>, _>("password_nonce")?;
    let nonce: [u8; 12] = nonce.try_into().map_err(|value: Vec<u8>| {
        sqlx::Error::Decode(format!("invalid credential nonce length: {}", value.len()).into())
    })?;
    Ok(SubsonicCredentialRecord {
        account: AccountRecord {
            id: parse_uuid(row.try_get("id")?)?,
            username: row.try_get("username")?,
            password_hash: row.try_get("password_hash")?,
            role: AccountRole::from_str(row.try_get::<&str, _>("role")?)
                .map_err(|error| sqlx::Error::Decode(error.into()))?,
            disabled: row.try_get::<i64, _>("disabled")? != 0,
        },
        encrypted_password: EncryptedSecret {
            nonce,
            ciphertext: row.try_get("password_ciphertext")?,
        },
    })
}

fn artist_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ArtistItem, sqlx::Error> {
    Ok(ArtistItem {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        name: row.try_get("name")?,
        artwork_hash: row.try_get("artwork_hash")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
    })
}

fn artist_summary_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ArtistSummary, sqlx::Error> {
    let album_count = row.try_get("album_count")?;
    Ok(ArtistSummary {
        artist: artist_from_row(row)?,
        album_count,
    })
}

fn album_from_row(row: sqlx::sqlite::SqliteRow) -> Result<AlbumItem, sqlx::Error> {
    Ok(AlbumItem {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        title: row.try_get("title")?,
        artist: row.try_get("album_artist_name")?,
        artist_id: row
            .try_get::<Option<String>, _>("album_artist_id")?
            .map(parse_uuid)
            .transpose()?,
        artwork_hash: row.try_get("artwork_hash")?,
        year: row.try_get("year")?,
        created_at: row.try_get("created_at")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
        play_count: row.try_get("play_count")?,
        last_played_at: row.try_get("last_played_at")?,
    })
}

fn song_from_row(row: sqlx::sqlite::SqliteRow) -> Result<SongItem, sqlx::Error> {
    let relative: String = row.try_get("relative_path")?;
    Ok(SongItem {
        id: parse_uuid(row.try_get("id")?)?,
        library_id: parse_uuid(row.try_get("library_id")?)?,
        album_id: row
            .try_get::<Option<String>, _>("album_id")?
            .map(parse_uuid)
            .transpose()?,
        title: row.try_get("title")?,
        album: row.try_get("album_title")?,
        artist: row.try_get("artist_display")?,
        genre: row.try_get("genre_display")?,
        year: row.try_get("year")?,
        track: row.try_get("track_number")?,
        disc: row.try_get("disc_number")?,
        duration_ms: row.try_get("duration_ms")?,
        bitrate: row.try_get("bitrate")?,
        codec: row.try_get("codec")?,
        suffix: PathBuf::from(relative)
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("")
            .to_ascii_lowercase(),
        size: row.try_get("file_size")?,
        artwork_hash: row.try_get("artwork_hash")?,
        full_hash: row.try_get("full_hash")?,
        created_at: row.try_get("created_at")?,
        starred_at: row.try_get("starred_at")?,
        user_rating: row.try_get("user_rating")?,
    })
}

async fn replace_playlist_tracks(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    playlist: Uuid,
    ids: &[Uuid],
    now: i64,
) -> Result<(), sqlx::Error> {
    for (position, id) in ids.iter().enumerate() {
        sqlx::query("INSERT INTO playlist_track (playlist_id, track_id, position, added_at) VALUES (?, ?, ?, ?)")
            .bind(playlist.to_string()).bind(id.to_string()).bind(position as i64).bind(now).execute(&mut **tx).await?;
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), ServiceError> {
    if (1..=200).contains(&name.trim().chars().count()) {
        Ok(())
    } else {
        Err(ServiceError::Invalid)
    }
}

fn validate_replay_type(receipt: &MutationReceipt, expected: &str) -> Result<(), ServiceError> {
    if receipt.entity_type == expected {
        Ok(())
    } else {
        Err(ServiceError::Conflict)
    }
}

fn validate_username(username: &str) -> Result<(), ServiceError> {
    let username = username.trim();
    if !(3..=64).contains(&username.len())
        || !username.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        Err(ServiceError::Invalid)
    } else {
        Ok(())
    }
}

fn parse_uuid(value: String) -> Result<Uuid, sqlx::Error> {
    Uuid::from_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
}
