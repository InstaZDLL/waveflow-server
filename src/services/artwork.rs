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
//! An artist portrait (RFC-011 decision 8) is the exception: its bytes came
//! from a provider, not from a file on disk, and no scan writes them back. The
//! window costs it more — a dead link until the cache entry expires and the
//! next visit downloads it again, up to the hit TTL — and is the same width,
//! since its row is committed in one transaction under this same gate.
//!
//! A podcast cover and a station image (RFC-025, amended) are the same case:
//! their bytes came from another host. Both are repaired on the next write
//! that names them — a refresh of the channel, a save of the station — which
//! asks whether the file is still in the store before it trusts an unchanged
//! address, and fetches it again when it is not.
//!
//! That is an argument for tolerating the window, not for pretending it is
//! shut. It is narrow, it needs a scan and a sweep to interleave at one
//! instant, and no test here forces that ordering.

use std::collections::{HashMap, HashSet};

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
    /// `<64 hex>.bin` — what the core wrote for a picture it could not type
    /// before v2.1.0. The cover only while its row still says `bin`; once the
    /// row names a typed copy that is whole by its hash, a leftover
    /// `retype_bin_artwork` could not remove.
    Bin { hash: String },
    /// Anything else, including a stem that is not a hash.
    Unknown,
}

/// Where a `bin` cover's bytes are now, by the format they turned out to be.
struct TypedCover {
    format: &'static str,
    /// The `bin` file is still there beside the typed one, and goes once the
    /// row names the typed one.
    leftover: bool,
}

/// The first bytes of a file, as many as `detect_image_format` asks about.
async fn leading_bytes(path: &std::path::Path) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut head = Vec::with_capacity(12);
    tokio::fs::File::open(path)
        .await?
        .take(12)
        .read_to_end(&mut head)
        .await?;
    Ok(head)
}

/// Whether `path` holds the cover named `hash`, whole: its bytes hash, under
/// BLAKE3 as `extract_cover` computed it, to the name.
///
/// The name is what a writer meant, not what the disk holds, and comparing
/// two copies says only that they differ, not which one is damaged — the
/// hash does. A copy that does not match is never trusted to replace or
/// outlive one that does.
async fn whole_copy(path: &std::path::Path, hash: &str) -> bool {
    tokio::fs::read(path)
        .await
        .is_ok_and(|bytes| blake3::hash(&bytes).to_hex().as_str() == hash)
}

impl DomainServices {
    /// Walks the artwork store on a timer. The shape `spawn_canvas_sweeper`
    /// uses: a pass at boot, then one per interval.
    pub fn spawn_artwork_sweeper(&self) {
        let services = self.clone();
        tokio::spawn(async move {
            // Before the first sweep, so the walk that follows already sees
            // each repaired cover under its new name.
            match services.retype_bin_artwork().await {
                Ok(0) => {}
                Ok(retyped) => tracing::info!(retyped, "covers stored as bin retyped"),
                Err(error) => tracing::warn!(%error, "could not retype the covers stored as bin"),
            }
            services.sweep_artwork_now().await;
            let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
            ticker.tick().await;
            loop {
                ticker.tick().await;
                services.sweep_artwork_now().await;
            }
        });
    }

    /// Gives every cover stored as `bin` the format its bytes are, and says how
    /// many rows moved. Public so a test can run it rather than restart.
    ///
    /// Before `waveflow-core` v2.1.0, `extract_cover` typed a picture by the
    /// MIME type its tag declared, and lofty has no variant for WebP: a WebP
    /// cover, which is what a video thumbnail usually is, was stored as
    /// `<hash>.bin` under a row saying `bin`. No MIME type answers to that, so
    /// the cover was never served. The core now reads the bytes, but a row
    /// that exists is never written again (`ON CONFLICT (hash) DO NOTHING`),
    /// and an unchanged file is not even read again by an ordinary scan: the
    /// old rows would stay dark until a full rescan, and then still, since the
    /// insert would leave them as they were.
    ///
    /// So the store is repaired where it stands, at every boot, and the pass
    /// costs nothing once it has run: it asks only for the rows still saying
    /// `bin`, and a repaired row no longer does. A `bin` whose bytes are no
    /// image this server can serve stays as it is — the pass never invents a
    /// format — and is asked about again at the next boot, which is a read of
    /// a few bytes per such cover.
    ///
    /// The desktop makes the same repair at its scan, from `bin` only: any
    /// other stored format names a file that exists under that name.
    pub async fn retype_bin_artwork(&self) -> Result<usize, ServiceError> {
        let hashes: Vec<String> =
            sqlx::query_scalar("SELECT hash FROM artwork WHERE format = 'bin'")
                .fetch_all(self.db.pool())
                .await?;
        let mut retyped = 0;
        for hash in hashes {
            // The table checks the length only; the hash is joined into a path.
            if !is_hash(&hash) {
                continue;
            }
            let Some(found) = self.typed_copy_of_bin(&hash).await else {
                continue;
            };
            let moved = {
                let _writer = self.db.writer_guard().await;
                let mut tx = self.db.begin_write(&_writer).await?;
                let moved =
                    sqlx::query("UPDATE artwork SET format = ? WHERE hash = ? AND format = 'bin'")
                        .bind(found.format)
                        .bind(&hash)
                        .execute(&mut *tx)
                        .await?
                        .rows_affected();
                tx.commit().await?;
                moved
            };
            if moved == 0 {
                continue;
            }
            retyped += 1;
            // Only once the row names the typed file: before, the `bin` was
            // still the cover's only name in the database.
            if found.leftover {
                let bin = self.artwork_dir.join(format!("{hash}.bin"));
                if let Err(error) = tokio::fs::remove_file(&bin).await {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        tracing::warn!(%error, "cannot remove a retyped cover's bin file");
                    }
                }
            }
        }
        Ok(retyped)
    }

    /// Puts a `bin` cover's bytes under the name of what they are, and says
    /// which format that is; `None` when they are no image this server serves,
    /// or cannot be read.
    ///
    /// Renamed rather than copied when the typed name is free, which leaves
    /// nothing behind. Every copy is judged by its hash ([`whole_copy`]). When
    /// the typed name holds the cover whole — a scan under the new core wrote
    /// it there — the `bin` is a leftover, removed once the row has followed,
    /// or by the sweep if that removal never happens. When it holds anything
    /// else, the `bin` replaces it, if the `bin` is whole; when neither is,
    /// both stay and the row is not touched. When the `bin`'s header names no
    /// format, a whole typed copy is still the cover, and the `bin` a
    /// leftover. And when the `bin` is gone, a pass stopped between the rename
    /// and the row: the typed file is adopted if it is whole.
    async fn typed_copy_of_bin(&self, hash: &str) -> Option<TypedCover> {
        let bin = self.artwork_dir.join(format!("{hash}.bin"));
        match leading_bytes(&bin).await {
            Ok(head) => {
                let Some(format) = waveflow_core::artwork::image_format::detect_image_format(&head)
                else {
                    // The `bin`'s own header names nothing — empty, damaged,
                    // or a format this server does not serve — but a whole
                    // typed copy beside it is still the cover.
                    return self.whole_typed_copy(hash).await.map(|format| TypedCover {
                        format,
                        leftover: true,
                    });
                };
                let typed = self.artwork_dir.join(format!("{hash}.{format}"));
                // A whole typed copy is the cover, and the `bin` beside it a
                // leftover, damaged or not.
                if whole_copy(&typed, hash).await {
                    return Some(TypedCover {
                        format,
                        leftover: true,
                    });
                }
                // Neither copy is whole: nothing here can say which bytes are
                // the cover, so both stay and the row is left saying `bin`.
                if !whole_copy(&bin, hash).await {
                    tracing::warn!(%hash, "no copy of a cover stored as bin matches its hash");
                    return None;
                }
                // The `bin` is whole and the typed name free or holding
                // something else: the `bin`'s bytes take the name.
                match tokio::fs::rename(&bin, &typed).await {
                    Ok(()) => Some(TypedCover {
                        format,
                        leftover: false,
                    }),
                    Err(error) => {
                        tracing::warn!(%error, "cannot rename a cover stored as bin");
                        None
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.whole_typed_copy(hash).await.map(|format| TypedCover {
                    format,
                    leftover: false,
                })
            }
            Err(error) => {
                tracing::warn!(%error, "cannot read a cover stored as bin");
                None
            }
        }
    }

    /// The format of a typed copy of `hash` that is whole and whose bytes are
    /// what its name says, if the store holds one.
    async fn whole_typed_copy(&self, hash: &str) -> Option<&'static str> {
        for format in ["jpg", "png", "webp"] {
            let typed = self.artwork_dir.join(format!("{hash}.{format}"));
            let Ok(head) = leading_bytes(&typed).await else {
                continue;
            };
            if waveflow_core::artwork::image_format::detect_image_format(&head) == Some(format)
                && whole_copy(&typed, hash).await
            {
                return Some(format);
            }
        }
        None
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
    /// `<64 hex>.bin` as the core wrote before v2.1.0 — a cover while its row
    /// says `bin`, a leftover once it does not — or one of the two thumbnails
    /// derived from such a hash. Everything else is counted and left where it
    /// is.
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

        let formats: HashMap<String, String> =
            sqlx::query_as::<_, (String, String)>("SELECT hash, format FROM artwork")
                .fetch_all(self.db.pool())
                .await?
                .into_iter()
                .collect();
        let named: HashSet<String> = formats.keys().cloned().collect();

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
                // The cover while its row says `bin`. Once the row names a
                // typed file, a leftover — but only when that file is whole by
                // its hash: one missing or damaged may leave the `bin` the only
                // whole copy, kept for whoever repairs the store. An unnamed
                // hash nothing reads.
                StoreEntry::Bin { hash } => match formats.get(&hash).map(String::as_str) {
                    Some("bin") => {
                        seen.insert(hash);
                    }
                    Some(format) => {
                        let typed = self.artwork_dir.join(format!("{hash}.{format}"));
                        if whole_copy(&typed, &hash).await && discard_aged_file(&entry.path()).await
                        {
                            swept.covers_removed += 1;
                        }
                    }
                    None => {
                        if discard_aged_file(&entry.path()).await {
                            swept.covers_removed += 1;
                        }
                    }
                },
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

    /// Drops every `artwork` row no artist, album, track, hand-chosen artist
    /// image (RFC-013), podcast channel or radio station (RFC-025, amended)
    /// names, and says how many went. The choice table's key has no
    /// `ON DELETE`: forgetting it here would fail the sweep rather than strand
    /// a choice. A channel's cover and a station's image are released by
    /// deleting the channel or the station — or by the feed naming another
    /// cover, or the account another image — and collected here.
    ///
    /// Three columns rather than the canvas store's one link table, and all
    /// three are `ON DELETE SET NULL` — which is the reason this is needed at
    /// all. Deleting an album returns its column to `NULL` and tells nobody, so
    /// the row it used to name has been unreferenced ever since with no event
    /// anywhere to notice it. There is no unlink path to hang a reference count
    /// on; there is only asking.
    async fn forget_unreferenced_artwork(&self) -> Result<usize, ServiceError> {
        let _writer = self.db.writer_guard().await;
        let mut tx = self.db.begin_write(&_writer).await?;
        // The spent portrait searches first (RFC-013), in the same
        // transaction: their previews become unreferenced here, and a preview
        // a choice already names is kept by the clause below.
        super::artist_image_search::forget_spent_searches(
            &mut tx,
            crate::auth::authentication::now_ms(),
        )
        .await?;
        let removed = sqlx::query(
            "DELETE FROM artwork WHERE \
               NOT EXISTS (SELECT 1 FROM artist WHERE artist.artwork_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM artist WHERE artist.fanart_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM album WHERE album.artwork_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM track WHERE track.artwork_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM artist_image_choice c WHERE c.artwork_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM artist_image_candidate c WHERE c.artwork_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM podcast_channel p WHERE p.artwork_hash = artwork.hash) \
               AND NOT EXISTS (SELECT 1 FROM radio_station r WHERE r.artwork_hash = artwork.hash)",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
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
    if extension == "bin" && is_hash(stem) {
        return StoreEntry::Bin {
            hash: stem.to_owned(),
        };
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
