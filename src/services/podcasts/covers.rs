//! A channel's cover: when it is fetched again, and how it is written.

use super::*;

impl DomainServices {
    /// The cover of a channel just subscribed, fetched and attached.
    pub(super) async fn fill_first_cover(&self, channel_id: Uuid, named: &str) {
        let cover = self
            .channel_cover(Some(named), &KnownCover::default())
            .await;
        if matches!(cover, CoverWrite::Keep) {
            return;
        }
        let recorded: Result<(), ServiceError> = async {
            let _writer = self.db.writer_guard().await;
            let mut tx = self.db.begin_write(&_writer).await?;
            // Unsubscribed meanwhile: the update names no row, and the file
            // is left to the sweep.
            write_cover(&mut tx, &channel_id.to_string(), &cover, now_ms()).await?;
            tx.commit().await?;
            Ok(())
        }
        .await;
        if let Err(error) = recorded {
            tracing::error!(error = %error, "a podcast cover could not be recorded");
        }
    }

    /// Waits until every cover fetch a subscription started so far has
    /// ended. Public for the tests, which otherwise could not tell a cover
    /// refused from one still on its way.
    pub async fn podcast_covers_settled(&self) {
        let _all = self.podcasts.covers.write().await;
    }

    /// What a refresh should do about the cover the feed names: keep the one
    /// held, forget it, or the one it has just fetched.
    ///
    /// A fetch that fails keeps what is held — the old cover, when the feed
    /// moved to a new one that cannot be reached — and stores nothing; the
    /// next refresh tries again, because the address fetched from still
    /// differs from the one named, or because no cover is held at all.
    pub(super) async fn channel_cover(
        &self,
        named: Option<&str>,
        known: &KnownCover,
    ) -> CoverWrite {
        let file_present = match &known.hash {
            Some(hash) => self.artwork_file_present(hash).await,
            None => false,
        };
        match cover_plan(named, known, file_present, now_ms()) {
            CoverPlan::Keep => CoverWrite::Keep,
            CoverPlan::Clear => CoverWrite::Clear,
            CoverPlan::Fetch(url) => {
                let seen = known.fetched_at;
                match self
                    .fetch_remote_artwork(url, self.podcasts.settings.allow_http, "podcast cover")
                    .await
                {
                    Some(artwork) => CoverWrite::Stored {
                        artwork,
                        url: url.to_owned(),
                        seen,
                    },
                    None => CoverWrite::Keep,
                }
            }
        }
    }
}

/// How long a cover fetched from an unchanged address is trusted before it is
/// fetched again: a feed can replace the image behind the same address. A
/// week, against the default refresh every six hours, is one cover fetch per
/// channel per week rather than four a day.
pub(super) const COVER_TTL_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// What a channel holds of its cover.
#[derive(Debug, Default)]
pub(super) struct KnownCover {
    pub(super) hash: Option<String>,
    /// The address `hash` was fetched from.
    pub(super) url: Option<String>,
    pub(super) fetched_at: Option<i64>,
}

/// What [`cover_plan`] decides, before anything is fetched.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum CoverPlan<'a> {
    Keep,
    Clear,
    Fetch(&'a str),
}

/// What a refresh writes about the cover.
pub(super) enum CoverWrite {
    Keep,
    Clear,
    Stored {
        artwork: super::remote_artwork::FetchedArtwork,
        /// The address it was fetched from.
        url: String,
        /// `artwork_fetched_at` as it was read before the fetch began. Any
        /// other value at write time means a cover was recorded meanwhile,
        /// and that one is the newer, whatever order the fetches end in.
        seen: Option<i64>,
    },
}

/// Whether the cover a feed names has to be fetched.
///
/// Only when the feed names one and the channel does not already hold it:
/// fetched from that very address, its file still in the store, and fetched
/// less than [`COVER_TTL_MS`] ago. A feed that names none any more has its
/// cover forgotten, which releases it to the sweep.
pub(super) fn cover_plan<'a>(
    named: Option<&'a str>,
    known: &KnownCover,
    file_present: bool,
    now: i64,
) -> CoverPlan<'a> {
    let Some(named) = named else {
        return if known.hash.is_some() || known.url.is_some() {
            CoverPlan::Clear
        } else {
            CoverPlan::Keep
        };
    };
    let held = known.hash.is_some()
        && file_present
        && known.url.as_deref() == Some(named)
        && known
            .fetched_at
            .is_some_and(|at| now.saturating_sub(at) < COVER_TTL_MS);
    if held {
        CoverPlan::Keep
    } else {
        CoverPlan::Fetch(named)
    }
}

/// Writes a cover decision onto a channel, under the writer gate: in the
/// transaction of a refresh, or in its own after a subscription.
///
/// **Only while the decision still holds.** A cover is fetched outside the
/// gate, and two fetches for one channel can be in flight at once — the
/// subscription's, in the background, and a refresh's. So the write names what
/// the fetch assumed: the channel still exists, it still names the address the
/// cover came from (or, to clear it, names none), and no cover was recorded
/// since this fetch began. Otherwise nothing is written, the newer state
/// stands, and an `artwork` row recorded for the dropped cover names nothing;
/// the sweep collects it with its file.
pub(super) async fn write_cover(
    tx: &mut sqlx::Transaction<'static, sqlx::Sqlite>,
    channel_id: &str,
    cover: &CoverWrite,
    now: i64,
) -> Result<(), sqlx::Error> {
    match cover {
        CoverWrite::Keep => {}
        CoverWrite::Clear => {
            sqlx::query(
                "UPDATE podcast_channel SET artwork_hash=NULL, artwork_url=NULL, \
                   artwork_fetched_at=NULL WHERE id=? AND image_url IS NULL",
            )
            .bind(channel_id)
            .execute(&mut **tx)
            .await?;
        }
        CoverWrite::Stored { artwork, url, seen } => {
            super::remote_artwork::record_artwork(tx, artwork, "podcast", now).await?;
            // `IS`, not `=`: a channel that held no cover reads `NULL`, and
            // must still read `NULL` for this write to be the first.
            sqlx::query(
                "UPDATE podcast_channel SET artwork_hash=?, artwork_url=?, artwork_fetched_at=? \
                 WHERE id=? AND image_url=? AND artwork_fetched_at IS ?",
            )
            .bind(&artwork.hash)
            .bind(url)
            .bind(now)
            .bind(channel_id)
            .bind(url)
            .bind(seen)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}
