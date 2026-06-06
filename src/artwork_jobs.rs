//! Background self-heal scanner for the artwork cache (Phase 1.i.1).
//!
//! The 1.h.3 upload handler self-heals a partial variant cache on
//! the re-upload path, but only when a client happens to re-POST the
//! same bytes. A partial cache that nobody re-uploads stays partial
//! forever, and the public read endpoints would keep advertising an
//! incomplete `variants[]` to every reader.
//!
//! This module spawns a tokio task at boot — same pattern as the
//! sync compaction loop in [`crate::sync::SyncHub::spawn`] — that
//! periodically scans `metadata_artwork` for parents whose variant
//! row count is below [`EXPECTED_VARIANT_COUNT`], fetches the
//! source bytes from object_store, re-runs the pipeline, and
//! inserts only the variants still missing. The repair writes go
//! through `ON CONFLICT (parent_hash, variant) DO NOTHING` so a
//! concurrent upload-side repair (or a peer scanner instance in a
//! multi-replica deploy) collapses cleanly.
//!
//! We deliberately picked a tokio polling loop over `apalis` for
//! 1.i.1 — the workload is "periodic catch-up", not "queue-driven
//! retries with priorities", and the surrounding infra already has a
//! compaction loop to generalise from. `apalis` (Postgres-backed
//! queue + retries + monitoring) lands when a job type genuinely
//! needs those primitives (e.g. RFC-004 community moderation).

use std::time::Duration;

use sqlx::PgPool;
use tokio::task::JoinHandle;

use crate::artwork_pipeline;
use crate::storage::ArtworkStorage;

/// Number of variants the pipeline produces per upload. Pinned in
/// lockstep with [`crate::artwork_pipeline::VariantKind`] (`thumb` +
/// `preview` today) and with the matching constant in
/// [`crate::api::artwork`] — bumping one without the other lets the
/// scanner repeatedly find "missing" rows that aren't actually
/// missing.
pub const EXPECTED_VARIANT_COUNT: i64 = 2;

/// Cadence the spawned scanner ticks at by default. 5 minutes is
/// the sweet spot for the catch-up role: short enough that a
/// partial cache from a transient failure heals before any client
/// notices, long enough that a healthy deployment doesn't burn
/// Postgres round-trips on no-op scans.
pub const DEFAULT_SCAN_INTERVAL: Duration = Duration::from_secs(300);

/// Maximum number of parents repaired per cycle. Bounded so a giant
/// backlog (e.g. after a 2-hour storage outage) doesn't hog the
/// pool — the next tick picks up where this one left off.
pub const DEFAULT_BATCH_SIZE: usize = 50;

/// Knobs loaded from env at boot. Mirrors the `Config::from_env`
/// shape — every tunable funnels through here, no `std::env` reads
/// in the scanner's hot loop.
#[derive(Debug, Clone)]
pub struct ArtworkScannerConfig {
    /// How long between scans. Floor of 1 s applied at boot to keep
    /// a misconfigured `WAVEFLOW_ARTWORK_SCANNER_INTERVAL_SECS=0`
    /// from busy-looping the worker thread.
    pub interval: Duration,
    /// Max parents to repair per cycle. Floor of 1 applied at boot.
    pub batch_size: usize,
}

impl Default for ArtworkScannerConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_SCAN_INTERVAL,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

/// Spawn the background scanner. Returns the JoinHandle so the
/// caller can hold it through the binary's lifetime (dropping it
/// doesn't cancel the task — the task only stops when the runtime
/// shuts down). Caller is responsible for binding the handle to a
/// long-lived scope (`main.rs` uses `let _scanner = …`).
///
/// The loop sleeps `config.interval` BEFORE the first scan so a
/// fresh boot doesn't fight the first wave of REST traffic for the
/// pool. Subsequent cycles tick on the configured cadence;
/// `MissedTickBehavior::Skip` means a hiccup in one cycle doesn't
/// queue up multiple back-to-back catch-up runs.
pub fn spawn(
    pool: PgPool,
    storage: ArtworkStorage,
    config: ArtworkScannerConfig,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // First tick fires immediately under tokio — consume it so
        // the first real scan lands after `interval`, not at boot.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match run_once(&pool, &storage, config.batch_size).await {
                Ok(0) => {
                    tracing::debug!("artwork scanner: no partial caches this cycle");
                }
                Ok(repaired) => {
                    tracing::info!(repaired, "artwork scanner: repaired partial variant caches");
                }
                Err(err) => {
                    tracing::warn!(error = %err, "artwork scanner cycle failed");
                }
            }
        }
    })
}

/// One scan cycle. Public so the integration tests can drive it
/// deterministically without waiting for the ticker. Returns the
/// number of parents successfully repaired.
///
/// Individual repair failures are logged + counted as "not repaired"
/// but don't abort the cycle — one broken parent shouldn't block
/// the queue behind it. The cycle as a whole only errors on a top-
/// level DB failure (the `list_partial_parents` query) so a healthy
/// scanner doesn't surface noise the operator can't action.
pub async fn run_once(
    pool: &PgPool,
    storage: &ArtworkStorage,
    batch_size: usize,
) -> Result<usize, sqlx::Error> {
    let parents =
        crate::db::artwork::list_partial_parents(pool, EXPECTED_VARIANT_COUNT, batch_size as i64)
            .await?;
    let mut repaired = 0usize;
    for parent_hash in parents {
        match repair_one(pool, storage, &parent_hash).await {
            Ok(()) => repaired += 1,
            Err(err) => {
                tracing::warn!(
                    parent_hash = %parent_hash,
                    error = %err,
                    "artwork scanner: parent repair failed",
                );
            }
        }
    }
    Ok(repaired)
}

/// Repair one parent. Pulls the source bytes from object_store, re-
/// runs the pipeline, and inserts the variants still missing from
/// `metadata_artwork_variant`.
///
/// Returns the boxed error rather than a typed variant — the caller
/// only logs + counts the failure, so a richer enum would add code
/// without buying any new behaviour.
async fn repair_one(
    pool: &PgPool,
    storage: &ArtworkStorage,
    parent_hash: &str,
) -> anyhow::Result<()> {
    let source = storage
        .get(parent_hash)
        .await
        .map_err(|err| anyhow::anyhow!("failed to fetch source bytes from storage: {err}"))?;

    let existing = crate::db::artwork::fetch_variants_for_parent(pool, parent_hash).await?;
    let existing_names: std::collections::HashSet<&str> =
        existing.iter().map(|(name, _)| name.as_str()).collect();

    let pipeline_variants = artwork_pipeline::generate_variants(&source)
        .map_err(|err| anyhow::anyhow!("pipeline rejected source bytes: {err}"))?;

    for variant in &pipeline_variants {
        if existing_names.contains(variant.kind.as_str()) {
            continue;
        }
        storage
            .put(&variant.hash, variant.bytes.clone())
            .await
            .map_err(|err| anyhow::anyhow!("failed to write variant bytes: {err}"))?;
        crate::db::artwork::insert_variant_if_absent(
            pool,
            parent_hash,
            variant.kind.as_str(),
            &variant.hash,
            variant.mime,
            variant.byte_size,
            variant.width as i32,
            variant.height as i32,
        )
        .await?;
    }
    Ok(())
}
