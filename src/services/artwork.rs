//! Collecting the covers and thumbnails nothing names any more.
//!
//! `artwork_dir` has grown in one direction since the beginning: `upsert_artwork`
//! inserts `ON CONFLICT DO NOTHING` and no statement anywhere deletes from
//! `artwork`, while `waveflow_core` writes `<hash>.<format>` beside two
//! thumbnails and nothing ever unlinks any of the three. A library that is
//! rescanned after its covers change keeps every cover it has ever held.
//!
//! # Why this is not `sweep_canvas_store` with another directory
//!
//! The canvas store is written by this process, so a placement and a sweep can
//! take the same per-hash lock and the race between them is closed. Covers are
//! written by `waveflow_core::scanner::extract_cover`, inside a blocking task,
//! from a crate that knows nothing about [`DomainServices`] — there is no lock
//! to take, and taking the writer gate is not an answer: file I/O has no
//! business happening while the process-wide gate is held, which is the rule
//! `upload_locks` and `canvas_locks` both exist to follow.
//!
//! So age stands in for the lock, exactly as it does for the canvas *working*
//! files, and for the same stated reason — the name belongs to a writer this
//! module cannot synchronise with. A file younger than [`WRITE_GRACE`] is left
//! alone whatever the database says, which covers the window between
//! `extract_cover` writing its bytes and `apply_catalog_track` committing the
//! row that names them.
//!
//! # The window age does not close, and why it is survivable here
//!
//! `extract_cover` writes only `if !out_path.exists()`, so re-encountering a
//! cover already in the store refreshes no timestamp. A sweep that reads "no
//! row" for an old file, and a scan that commits a row for that same content an
//! instant later, still cross: the unlink then carries off the file of a live
//! row and leaves a dead link.
//!
//! That is the failure the canvas sweep calls unrecoverable, and here it is
//! not. A cover's bytes are not a gift from a client that will never come
//! again — they are in the audio file, which is read-only and still on disk.
//! The store is reconstructible: with the file gone, `out_path.exists()` is
//! false and the next scan to read that track writes it back. So the cost of
//! the race is a cover that may be missing until the next scan reaches it,
//! reported as a dead link meanwhile, rather than bytes that no longer exist
//! anywhere.
//!
//! That is an argument for tolerating the window, not for pretending it is
//! shut. It is narrow, it needs a scan and a sweep to interleave at one
//! instant, and no test here forces that ordering.

use std::collections::HashSet;

use super::{DomainServices, ServiceError};

/// How long a file is left alone regardless of what the database says.
///
/// It has to outlast the gap between `extract_cover` writing bytes and
/// `apply_catalog_track` committing the row that names them — one track's
/// processing, milliseconds in practice. An hour is the same figure the canvas
/// working files use, and is not a tuning knob: nothing legitimate spends it.
const WRITE_GRACE: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How often the store is walked.
///
/// Daily, as the canvas store is. Unlike that one this is housekeeping rather
/// than only repair — covers stop being referenced through ordinary use, when a
/// rescan finds new art or a library is deleted — but the cost is one directory
/// listing either way.
const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// What one pass of the artwork sweep found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArtworkSweep {
    /// `artwork` rows no artist, album or track named any more.
    pub rows_removed: usize,
    /// Cover files whose row is gone, and whose bytes are now gone with it.
    pub covers_removed: usize,
    /// Thumbnails of a cover that is no longer named.
    ///
    /// Counted apart because nothing in the database ever named them: they are
    /// derived files, found by their stem and removed with what they derive
    /// from.
    pub thumbnails_removed: usize,
    /// Rows naming a file that is not there.
    ///
    /// Counted and never repaired, as in the canvas store. Deleting the row to
    /// tidy the count would answer a cover that fails to load by removing the
    /// album's art outright, and the next scan to read the track puts the file
    /// back.
    pub dead_links: usize,
    /// Names the sweep did not recognise, left exactly where they were.
    pub unknown: usize,
}

/// What a name in `artwork_dir` turned out to be.
#[derive(Debug, PartialEq, Eq)]
enum StoreEntry {
    /// `<64 hex>.<format>` — the cover itself, named by an `artwork` row.
    Cover { hash: String },
    /// `<64 hex>_1x.jpg` or `<64 hex>_2x.jpg` — derived, named by nothing.
    Thumbnail { hash: String },
    /// Anything else, including a stem that is not a hash.
    Unknown,
}

impl DomainServices {
    /// Walks the artwork store on a timer. The shape `spawn_canvas_sweeper`
    /// uses: a pass at boot, then one per interval.
    pub fn spawn_artwork_sweeper(&self) {
        let services = self.clone();
        tokio::spawn(async move {
            services.sweep_artwork_now().await;
            let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.sweep_artwork_now().await;
            }
        });
    }

    async fn sweep_artwork_now(&self) {
        match self.sweep_artwork_store().await {
            Ok(swept) if swept == ArtworkSweep::default() => {}
            Ok(swept) => tracing::info!(
                rows = swept.rows_removed,
                covers = swept.covers_removed,
                thumbnails = swept.thumbnails_removed,
                dead_links = swept.dead_links,
                unknown = swept.unknown,
                "artwork store swept"
            ),
            Err(error) => tracing::warn!(%error, "could not sweep the artwork store"),
        }
    }

    /// One pass. Public so a test can run it rather than wait a day for it.
    ///
    /// **Nothing is removed that this does not recognise.** `artwork_dir` lives
    /// under the operator's `data/`, and a sweep that deletes what it cannot
    /// name is a sweep nobody should run. A file is a candidate only if it is
    /// `<64 hex>.<format>` for a format [`crate::media::artwork_mime`] admits,
    /// or one of the two thumbnails derived from such a hash. Everything else is
    /// counted and left where it is.
    pub async fn sweep_artwork_store(&self) -> Result<ArtworkSweep, ServiceError> {
        // The rows first, so the walk below asks a database that no longer
        // names what nothing references. Safe under the writer gate without a
        // grace period of its own: `upsert_artwork` inserts the row and the
        // column that names it inside one transaction, so a committed row is
        // already referenced and there is no window where a live cover looks
        // unreferenced.
        let mut swept = ArtworkSweep {
            rows_removed: self.forget_unreferenced_artwork().await?,
            ..ArtworkSweep::default()
        };

        let named: HashSet<String> = sqlx::query_scalar("SELECT hash FROM artwork")
            .fetch_all(self.db.pool())
            .await?
            .into_iter()
            .collect();

        let mut entries = match tokio::fs::read_dir(&self.artwork_dir).await {
            Ok(entries) => entries,
            // No store yet is not a failure: nothing has been scanned.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(swept),
            Err(error) => {
                tracing::warn!(%error, "cannot read the artwork store");
                return Err(ServiceError::Unavailable);
            }
        };

        let mut seen = HashSet::new();
        while let Some(entry) = entries.next_entry().await.map_err(|error| {
            tracing::warn!(%error, "cannot walk the artwork store");
            ServiceError::Unavailable
        })? {
            let name = entry.file_name().to_string_lossy().into_owned();
            match classify_store_entry(&name) {
                StoreEntry::Cover { hash } => {
                    if named.contains(&hash) {
                        seen.insert(hash);
                    } else if discard_aged_file(&entry.path()).await {
                        swept.covers_removed += 1;
                    }
                }
                StoreEntry::Thumbnail { hash } => {
                    if !named.contains(&hash) && discard_aged_file(&entry.path()).await {
                        swept.thumbnails_removed += 1;
                    }
                }
                StoreEntry::Unknown => {
                    tracing::warn!(file = %name, "unrecognised file left in the artwork store");
                    swept.unknown += 1;
                }
            }
        }

        // The other direction, reported and never repaired.
        for hash in &named {
            if !seen.contains(hash) {
                tracing::warn!(%hash, "an artwork row names a file the store does not hold");
                swept.dead_links += 1;
            }
        }
        Ok(swept)
    }

    /// Drops every `artwork` row no artist, album or track names, and says how
    /// many went.
    ///
    /// Three columns rather than the canvas store's one link table, and all
    /// three are `ON DELETE SET NULL` — which is the reason this is needed at
    /// all. Deleting an album returns its column to `NULL` and tells nobody, so
    /// the row it used to name has been unreferenced ever since with no event
    /// anywhere to notice it. There is no unlink path to hang a reference count
    /// on; there is only asking.
    async fn forget_unreferenced_artwork(&self) -> Result<usize, ServiceError> {
        let _writer = self.db.writer_guard().await;
        let removed = sqlx::query(
            "DELETE FROM artwork WHERE \
               NOT EXISTS (SELECT 1 FROM artist WHERE artist.artwork_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM album WHERE album.artwork_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM track WHERE track.artwork_hash = artwork.hash)",
        )
        .execute(self.db.pool())
        .await?
        .rows_affected();
        Ok(usize::try_from(removed).unwrap_or(usize::MAX))
    }
}

/// Removes a file the database no longer names, once it is old enough that no
/// writer can still be between its bytes and its row.
async fn discard_aged_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = tokio::fs::metadata(path).await else {
        return false;
    };
    let aged = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age > WRITE_GRACE);
    if !aged {
        return false;
    }
    match tokio::fs::remove_file(path).await {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            tracing::warn!(%error, "cannot remove an unreferenced artwork file");
            false
        }
    }
}

/// What a name in the store is, by its shape alone.
///
/// The thumbnail suffixes are `waveflow_core`'s, and they are matched before
/// the hash is read: `<hash>_1x` is not a hash, so a classifier that only knew
/// covers would call both thumbnails unknown and leave them behind forever —
/// two files per cover, growing exactly as the covers do.
fn classify_store_entry(name: &str) -> StoreEntry {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return StoreEntry::Unknown;
    };
    if extension == "jpg" {
        for suffix in ["_1x", "_2x"] {
            if let Some(hash) = stem.strip_suffix(suffix) {
                return if is_hash(hash) {
                    StoreEntry::Thumbnail {
                        hash: hash.to_owned(),
                    }
                } else {
                    StoreEntry::Unknown
                };
            }
        }
    }
    if crate::media::artwork_mime(extension).is_some() && is_hash(stem) {
        return StoreEntry::Cover {
            hash: stem.to_owned(),
        };
    }
    StoreEntry::Unknown
}

/// A BLAKE3 digest as `extract_cover` renders it, and as the `artwork` table
/// checks it: sixty-four lowercase hex characters.
fn is_hash(stem: &str) -> bool {
    stem.len() == 64
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
