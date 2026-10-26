//! Artwork fetched from another host: the cover a podcast feed names, and the
//! image an account gives a radio station (RFC-025, amended).
//!
//! The web client cannot show either from where it lives. RFC-019's
//! Content-Security-Policy keeps `img-src` to this origin, and an image loaded
//! from the feed's host would hand that host the address of every listener. So
//! the server fetches it once, through [`crate::portrait::fetch_followed`],
//! stores the bytes in the artwork store beside the scanned covers, and every
//! surface hands out the hash — never the address it came from.
//!
//! **Network and file I/O never hold the writer gate.** The image is fetched
//! and written to the store first; the row naming it is written under the
//! gate, in the caller's transaction, by [`record_artwork`].
//!
//! **Nothing about a fetch is logged but a verdict.** A private feed's cover
//! can carry its subscriber's token as readily as the feed, and a station's
//! image is whatever an account typed.

use super::*;

/// How many station images one account may have fetched in an hour.
///
/// A station image is the one fetch any account can ask for, so it is the one
/// that needs a bound beyond "fetched once per address": without one, an
/// account editing a station in a loop with a new address each time would
/// have the server download up to 10 MiB per request. Ten an hour is far
/// above what a person setting up their stations does, and caps the worst
/// case at 100 MiB an hour per account, which the sweep then collects.
pub const STATION_IMAGE_FETCHES_PER_HOUR: u32 = 10;

const BUDGET_WINDOW_MS: i64 = 60 * 60 * 1000;

/// The hourly fetch budget, per account. In memory: a restart refills it,
/// which costs at most one more hour's worth.
#[derive(Clone, Default)]
pub(super) struct FetchBudget {
    spent: Arc<dashmap::DashMap<Uuid, (i64, u32)>>,
}

impl FetchBudget {
    /// Whether `account` may start one more fetch at `now`, counting it if so.
    pub(super) fn take(&self, account: Uuid, now: i64) -> bool {
        let mut entry = self.spent.entry(account).or_insert((now, 0));
        if now.saturating_sub(entry.0) >= BUDGET_WINDOW_MS {
            *entry = (now, 0);
        }
        if entry.1 >= STATION_IMAGE_FETCHES_PER_HOUR {
            return false;
        }
        entry.1 += 1;
        true
    }
}

/// An image fetched and written to the store, not yet named by a row.
#[derive(Debug, Clone)]
pub(super) struct FetchedArtwork {
    pub(super) hash: String,
    pub(super) format: &'static str,
    pub(super) byte_size: i64,
}

impl DomainServices {
    /// Fetches the image at `url` and puts it in the artwork store.
    ///
    /// `None` when remote artwork is switched off, when the address is not one
    /// the rules admit, when the answer is not an image within the ceiling, or
    /// when the store cannot be written. Each is logged as a verdict, under
    /// `subject` (`podcast cover` or `station image`), and nothing is kept: the
    /// caller tries again on the next refresh or the next save.
    pub(super) async fn fetch_remote_artwork(
        &self,
        url: &str,
        allow_http: bool,
        subject: &'static str,
    ) -> Option<FetchedArtwork> {
        if !self.remote_artwork.enabled {
            return None;
        }
        let Ok(parsed) = url::Url::parse(url) else {
            tracing::warn!(subject, reason = "unparsable", "remote artwork not fetched");
            return None;
        };
        let policy = crate::podcast::FetchPolicy {
            addresses: self.remote_artwork.addresses,
            allow_http,
            timeout: self.remote_artwork.timeout,
        };
        let image = match crate::portrait::fetch_followed(&parsed, policy).await {
            Ok(image) => image,
            Err(refusal) => {
                tracing::warn!(subject, reason = %refusal, "remote artwork not fetched");
                return None;
            }
        };
        if let Err(error) = crate::portrait::store(&self.artwork_dir, &image).await {
            tracing::warn!(subject, %error, "remote artwork could not be stored");
            return None;
        }
        Some(FetchedArtwork {
            byte_size: i64::try_from(image.bytes.len()).unwrap_or(i64::MAX),
            hash: image.hash,
            format: image.format,
        })
    }

    /// Whether the file behind an `artwork` row is in the store.
    ///
    /// Asked before an unchanged address is trusted to have its cover: the
    /// sweep's window (see `artwork.rs`) can carry off a file whose row was
    /// written an instant later, and a cover from another host, unlike a
    /// scanned one, has no scan to write it back. Re-fetching is the repair.
    pub(super) async fn artwork_file_present(&self, hash: &str) -> bool {
        let format: Option<String> = sqlx::query_scalar("SELECT format FROM artwork WHERE hash=?")
            .bind(hash)
            .fetch_optional(self.db.pool())
            .await
            .ok()
            .flatten();
        let Some(format) = format else {
            return false;
        };
        tokio::fs::try_exists(self.artwork_dir.join(format!("{hash}.{format}")))
            .await
            .unwrap_or(false)
    }
}

/// The `artwork` row for a fetched image, in the transaction that names it,
/// so the sweep never sees the row unreferenced.
pub(super) async fn record_artwork(
    tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
    artwork: &FetchedArtwork,
    source: &'static str,
    now: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO artwork (hash, format, source, byte_size, created_at) \
         VALUES (?, ?, ?, ?, ?) ON CONFLICT (hash) DO NOTHING",
    )
    .bind(&artwork.hash)
    .bind(artwork.format)
    .bind(source)
    .bind(artwork.byte_size)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests;
