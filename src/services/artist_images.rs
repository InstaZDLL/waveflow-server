//! An artist's portrait and banner, chosen by hand (RFC-013).
//!
//! A choice lives in `artist_image_choice`, never in `artist.artwork_hash`,
//! which RFC-011's portrait fill rewrites. It belongs to the library: set by
//! those who may correct its metadata, seen by every member. Every write takes
//! the writer gate and records the artist library event (RFC-007), as a
//! fetched portrait does, so clients re-read the artist on their own.

use std::str::FromStr;

use sqlx::{Row, Sqlite, Transaction};
use uuid::Uuid;

use super::{DomainServices, ImageChoice, ImageChoices, ServiceError};
use crate::authentication::now_ms;
use crate::database::LibraryRole;
use crate::portrait::Portrait;

/// Which of the two images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtistImageKind {
    Portrait,
    Banner,
}

impl ArtistImageKind {
    /// `None` for anything but `portrait` and `banner`, so a route can answer
    /// an unknown kind as it answers an unknown path.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "portrait" => Some(Self::Portrait),
            "banner" => Some(Self::Banner),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Banner => "banner",
        }
    }
}

/// The artist's library, if the caller may choose its images: `NotFound` for
/// an artist they cannot see and `Forbidden` for one they may only read, both
/// answered as the same `404`.
pub(super) async fn chooser_library(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: Uuid,
    artist_id: Uuid,
) -> Result<Uuid, ServiceError> {
    let row = sqlx::query(
        "SELECT ar.library_id, m.role FROM artist ar \
         JOIN library_member m ON m.library_id=ar.library_id \
         WHERE m.user_id=? AND ar.id=?",
    )
    .bind(user_id.to_string())
    .bind(artist_id.to_string())
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(ServiceError::NotFound)?;
    let role = LibraryRole::from_str(row.try_get::<&str, _>("role")?)
        .map_err(|_| ServiceError::Invalid)?;
    if !role.may_write_metadata() {
        return Err(ServiceError::Forbidden);
    }
    Uuid::parse_str(row.try_get::<&str, _>("library_id")?).map_err(|_| ServiceError::Invalid)
}

pub(super) async fn record_artist_change(
    tx: &mut Transaction<'_, Sqlite>,
    library_id: Uuid,
    artist_id: Uuid,
    now: i64,
) -> Result<(), sqlx::Error> {
    crate::catalog::record_library_event(
        tx,
        crate::catalog::LibraryChange {
            library_id,
            entity_type: "artist",
            entity_id: artist_id,
            action: "upsert",
            payload: serde_json::json!({}),
            changed_at: now,
            origin_device_id: None,
        },
    )
    .await
}

/// Writes the choice, replacing whatever was chosen before for that kind.
/// `source` is `upload` or `deezer`, as the table's check allows.
pub(super) async fn write_choice(
    tx: &mut Transaction<'_, Sqlite>,
    artist_id: Uuid,
    kind: ArtistImageKind,
    artwork_hash: &str,
    source: &str,
    user_id: Uuid,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO artist_image_choice \
           (artist_id, kind, artwork_hash, source, set_by, set_at) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(artist_id, kind) DO UPDATE SET \
           artwork_hash=excluded.artwork_hash, source=excluded.source, \
           set_by=excluded.set_by, set_at=excluded.set_at",
    )
    .bind(artist_id.to_string())
    .bind(kind.as_str())
    .bind(artwork_hash)
    .bind(source)
    .bind(user_id.to_string())
    .bind(now)
    .execute(&mut **tx)
    .await
    .map(drop)
}

impl DomainServices {
    /// The banner shown and what was chosen, for the artist page: the chosen
    /// banner, or else the provider's fanart (RFC-014 decision 4). Read after
    /// access to the artist was established by the caller.
    pub(super) async fn artist_image_choices(
        &self,
        artist_id: Uuid,
    ) -> Result<(Option<String>, ImageChoices), ServiceError> {
        let rows = sqlx::query(
            "SELECT kind, artwork_hash, source, set_at FROM artist_image_choice \
             WHERE artist_id=?",
        )
        .bind(artist_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let mut banner = None;
        let mut choices = ImageChoices::default();
        for row in rows {
            let choice = ImageChoice {
                source: row.try_get("source")?,
                set_at: row.try_get("set_at")?,
            };
            match row.try_get::<&str, _>("kind")? {
                "portrait" => choices.portrait = Some(choice),
                "banner" => {
                    banner = Some(row.try_get("artwork_hash")?);
                    choices.banner = Some(choice);
                }
                _ => {}
            }
        }
        if banner.is_none() {
            banner = sqlx::query_scalar("SELECT fanart_hash FROM artist WHERE id=?")
                .bind(artist_id.to_string())
                .fetch_optional(self.db.pool())
                .await?
                .flatten();
        }
        Ok((banner, choices))
    }

    /// Choose an artist's portrait or banner from an image someone sent.
    ///
    /// `Invalid` for bytes that are not JPEG, PNG or WebP within 10 MiB —
    /// recognised from the bytes, never from a name. The file is stored once
    /// per content, before the writer gate is taken (it is file I/O); the row
    /// and the choice are written under it, with the artist event.
    pub async fn choose_artist_image(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
        kind: ArtistImageKind,
        bytes: Vec<u8>,
    ) -> Result<(), ServiceError> {
        let image = Portrait::from_bytes(bytes).ok_or(ServiceError::Invalid)?;
        // Asked before the file is written, so someone who may not choose
        // cannot fill the store with images nothing will ever name. Asked again
        // under the gate below, where the answer is the one the write keeps.
        {
            let mut read = self.db.pool().begin().await?;
            chooser_library(&mut read, user_id, artist_id).await?;
        }
        crate::portrait::store(&self.artwork_dir, &image)
            .await
            .map_err(|_| ServiceError::Unavailable)?;
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let library_id = chooser_library(&mut tx, user_id, artist_id).await?;
        let now = now_ms();
        sqlx::query(
            "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
             VALUES (?, ?, 'upload', ?, ?) ON CONFLICT (hash) DO NOTHING",
        )
        .bind(&image.hash)
        .bind(image.format)
        .bind(image.bytes.len() as i64)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        write_choice(
            &mut tx,
            artist_id,
            kind,
            &image.hash,
            "upload",
            user_id,
            now,
        )
        .await?;
        record_artist_change(&mut tx, library_id, artist_id, now).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Go back to the automatic image: for the portrait, whatever the files or
    /// a provider gave; for the banner, none. The image itself stays in the
    /// store until the sweep finds nothing naming it. `NotFound` when nothing
    /// was chosen.
    pub async fn remove_artist_image(
        &self,
        user_id: Uuid,
        artist_id: Uuid,
        kind: ArtistImageKind,
    ) -> Result<(), ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.pool().begin().await?;
        let library_id = chooser_library(&mut tx, user_id, artist_id).await?;
        let removed = sqlx::query("DELETE FROM artist_image_choice WHERE artist_id=? AND kind=?")
            .bind(artist_id.to_string())
            .bind(kind.as_str())
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if removed == 0 {
            return Err(ServiceError::NotFound);
        }
        record_artist_change(&mut tx, library_id, artist_id, now_ms()).await?;
        tx.commit().await?;
        Ok(())
    }
}
