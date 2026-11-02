//! Transcoding profiles (RFC-027): what an account wants for a client that does
//! not ask.
//!
//! One default per account and one override per Subsonic client name. Each
//! surface decides whether a request was silent, which only it can tell;
//! [`DomainServices::stream_profile`] answers what a silent request becomes, so
//! the native API and the façade cannot drift apart on it. Tenancy is the
//! `user_id` column, named in every statement here.

use super::*;
use crate::media::OutputFormat;

/// How long a client name may be: a device's name, the nearest thing it is.
const MAX_CLIENT_CHARS: usize = 120;

/// How many rows one account may hold, so that one account cannot grow the
/// table under the writer gate without bound. A real account has a handful.
const MAX_PROFILES: i64 = 32;

/// How many client names are remembered per account, the latest kept.
const MAX_SEEN_CLIENTS: usize = 16;

/// One row of an account's profile, as the routes show it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TranscodingProfileItem {
    /// The client name this row overrides the default for; `null` on the
    /// default.
    pub client: Option<String>,
    /// `mp3`, `opus` or `aac`; on an override also `raw`, which keeps that
    /// client on the original.
    pub format: String,
    /// The ceiling in kbit/s; `null` exactly when `format` is `raw`.
    pub bitrate: Option<u32>,
    pub updated_at: i64,
}

/// An account's profile, and what the section offers for a new override.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TranscodingProfiles {
    /// The default first, then the overrides by client name.
    pub profiles: Vec<TranscodingProfileItem>,
    /// The names this account's Subsonic clients have streamed with without
    /// asking for a format or a bit rate, since the server started. Those are
    /// the clients a profile reaches. In memory only, oldest first.
    pub seen_clients: Vec<String>,
}

/// The client names a silent stream has come with, per account.
#[derive(Clone, Default)]
pub(super) struct SeenClients(Arc<dashmap::DashMap<Uuid, Vec<String>>>);

impl SeenClients {
    fn note(&self, user_id: Uuid, client: &str) {
        let mut names = self.0.entry(user_id).or_default();
        // Moved to the end when already known, so the bound drops the name
        // heard from longest ago rather than the one heard from just now.
        if let Some(known) = names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(client))
        {
            names.remove(known);
        } else if names.len() >= MAX_SEEN_CLIENTS {
            names.remove(0);
        }
        names.push(client.to_owned());
    }

    fn of(&self, user_id: Uuid) -> Vec<String> {
        self.0
            .get(&user_id)
            .map(|names| names.clone())
            .unwrap_or_default()
    }
}

impl DomainServices {
    /// What a stream request that named neither a format nor a bit rate
    /// becomes for this account: the format and bit rate to encode at, or
    /// `None` to serve it as if no profile existed.
    ///
    /// `client` is the name a Subsonic request came with, and `None` on the
    /// native API, which has none. `source_bitrate` is what the scan learned.
    pub async fn stream_profile(
        &self,
        user_id: Uuid,
        client: Option<&str>,
        source_bitrate: Option<i64>,
    ) -> Result<Option<(OutputFormat, u32)>, ServiceError> {
        // A name no row could hold is no name at all, rather than a refusal:
        // the protocol's `c` is free text, and it is not this method's to judge.
        let client = client
            .map(str::trim)
            .filter(|name| !name.is_empty() && name.chars().count() <= MAX_CLIENT_CHARS);
        if let Some(client) = client {
            self.seen_stream_clients.note(user_id, client);
        }
        let row = sqlx::query(
            "SELECT format, bitrate FROM transcoding_profile \
             WHERE user_id=? AND client IN (?, '') ORDER BY client='' LIMIT 1",
        )
        .bind(user_id.to_string())
        .bind(client.unwrap_or(""))
        .fetch_optional(self.db.pool())
        .await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let format: String = row.try_get("format")?;
        let bitrate: Option<i64> = row.try_get("bitrate")?;
        Ok(profile_outcome(&format, bitrate, source_bitrate))
    }

    /// The account's rows, the default first, and the client names seen.
    pub async fn transcoding_profiles(
        &self,
        user_id: Uuid,
    ) -> Result<TranscodingProfiles, ServiceError> {
        let rows = sqlx::query(
            "SELECT client, format, bitrate, updated_at FROM transcoding_profile \
             WHERE user_id=? ORDER BY client",
        )
        .bind(user_id.to_string())
        .fetch_all(self.db.pool())
        .await?;
        let mut profiles = Vec::with_capacity(rows.len());
        for row in &rows {
            profiles.push(profile_from_row(row)?);
        }
        Ok(TranscodingProfiles {
            profiles,
            seen_clients: self.seen_stream_clients.of(user_id),
        })
    }

    /// Creates or replaces a row, whole. `client` is `None` for the default.
    pub async fn set_transcoding_profile(
        &self,
        user_id: Uuid,
        client: Option<&str>,
        format: &str,
        bitrate: Option<u32>,
    ) -> Result<TranscodingProfileItem, ServiceError> {
        let client = client_key(client)?;
        let format = OutputFormat::named(format).ok_or(ServiceError::Invalid)?;
        let bitrate = stored_bitrate(client, format, bitrate)?;
        let now = now_ms();

        let _writer = self.db.writer_guard().await;
        let others: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transcoding_profile WHERE user_id=? AND client<>?",
        )
        .bind(user_id.to_string())
        .bind(client)
        .fetch_one(self.db.pool())
        .await?;
        if others >= MAX_PROFILES {
            return Err(ServiceError::Conflict);
        }
        let row = sqlx::query(
            "INSERT INTO transcoding_profile (user_id, client, format, bitrate, updated_at) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (user_id, client) DO UPDATE SET client=excluded.client, \
             format=excluded.format, bitrate=excluded.bitrate, updated_at=excluded.updated_at \
             RETURNING client, format, bitrate, updated_at",
        )
        .bind(user_id.to_string())
        .bind(client)
        .bind(format.name())
        .bind(bitrate)
        .bind(now)
        .fetch_one(self.db.pool())
        .await?;
        profile_from_row(&row)
    }

    /// Removes a row; removing one that is not there is not an error.
    pub async fn delete_transcoding_profile(
        &self,
        user_id: Uuid,
        client: Option<&str>,
    ) -> Result<(), ServiceError> {
        let client = client_key(client)?;
        let _writer = self.db.writer_guard().await;
        sqlx::query("DELETE FROM transcoding_profile WHERE user_id=? AND client=?")
            .bind(user_id.to_string())
            .bind(client)
            .execute(self.db.pool())
            .await?;
        Ok(())
    }
}

/// What a stored row makes of a silent request.
///
/// **A ceiling, like `maxBitRate`** (RFC-027 decision 2): a source at or below
/// it is served as it is, and one whose bit rate is unknown is encoded. A row
/// this binary cannot read, left by a newer one, is no profile at all: a
/// preference must never be why nothing plays.
fn profile_outcome(
    format: &str,
    bitrate: Option<i64>,
    source_bitrate: Option<i64>,
) -> Option<(OutputFormat, u32)> {
    let Some(named) = OutputFormat::named(format) else {
        tracing::warn!(
            format,
            "ignoring a transcoding profile of an unknown format"
        );
        return None;
    };
    if named == OutputFormat::Raw {
        return None;
    }
    let ceiling = bitrate.and_then(|bitrate| u32::try_from(bitrate).ok())?;
    crate::media::source_exceeds(source_bitrate, ceiling).then_some((named, ceiling))
}

/// The key a row is stored under: the trimmed name, or the empty string for the
/// default. A name that is blank is refused rather than read as the default,
/// which it would then overwrite.
fn client_key(client: Option<&str>) -> Result<&str, ServiceError> {
    let Some(client) = client else {
        return Ok("");
    };
    let client = client.trim();
    if client.is_empty() || client.chars().count() > MAX_CLIENT_CHARS {
        return Err(ServiceError::Invalid);
    }
    Ok(client)
}

/// The bit rate a row stores. `raw` takes none, and is an override's alone: on
/// the default it would say what having no default says.
fn stored_bitrate(
    client: &str,
    format: OutputFormat,
    bitrate: Option<u32>,
) -> Result<Option<u32>, ServiceError> {
    if format == OutputFormat::Raw {
        if client.is_empty() || bitrate.is_some() {
            return Err(ServiceError::Invalid);
        }
        return Ok(None);
    }
    crate::media::validate_bitrate(format, bitrate)
        .map(Some)
        .map_err(|_| ServiceError::Invalid)
}

fn profile_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<TranscodingProfileItem, ServiceError> {
    let client: String = row.try_get("client")?;
    let bitrate: Option<i64> = row.try_get("bitrate")?;
    Ok(TranscodingProfileItem {
        client: (!client.is_empty()).then_some(client),
        format: row.try_get("format")?,
        bitrate: bitrate.and_then(|bitrate| u32::try_from(bitrate).ok()),
        updated_at: row.try_get("updated_at")?,
    })
}

#[cfg(test)]
mod tests;
