//! The bounds on what the server keeps on its own disk: files received, loops
//! attached, and how much of a library's change feed survives.

use std::time::Duration;

/// What the server accepts from a client that offers it a file.
///
/// Every one of these is a bound on someone else's disk, which is why they sit
/// together rather than scattered among the tunables: receiving a file is the
/// only thing the server does that cannot be undone by restarting it.
#[derive(Debug, Clone, Copy)]
pub struct UploadLimits {
    /// The largest single file the server will take.
    pub max_file_bytes: i64,
    /// How much of a library's disk received files may occupy in total, open
    /// sessions included — a session reserves what it declared, or two
    /// negotiations racing would each be told there was room for both.
    pub library_quota_bytes: i64,
    /// The size a client should send each fragment at. Advertised by the
    /// negotiation rather than assumed, so it can move without a client
    /// release.
    pub chunk_bytes: i64,
    /// How many offers one negotiation may carry. Bounded because the batch is
    /// the answer to five thousand round trips, and an unbounded array would
    /// trade them for one unbounded body.
    pub batch_limit: usize,
    /// How many sessions one account may hold open at once, across libraries —
    /// the same shape as the per-user transcode limit, and for the same reason:
    /// a client with five thousand files must not open five thousand of
    /// anything.
    pub sessions_per_user: usize,
    /// How long an untouched session survives. Generous, because a large file
    /// on a domestic link is measured in hours, not minutes.
    pub session_ttl: Duration,
}

/// How long a library's change feed keeps what it has written.
///
/// The floor wins over the age: a library below it keeps everything, however
/// old. RFC-007 decision 7.
#[derive(Debug, Clone, Copy)]
pub struct LibraryEventRetention {
    /// Whole days. Only what is **strictly** older is cut, so an event exactly
    /// this old survives — the same exclusive bound `stream_ticket::verify`
    /// uses, and the cautious direction: keeping one event too many breaks
    /// nobody, cutting one too many sends somebody back to the snapshot.
    pub days: u32,
    /// The fewest events a library keeps whatever their age.
    pub min_events: i64,
}

/// What the server accepts when a member attaches a loop to a track.
///
/// Apart from [`UploadLimits`] rather than folded into it: the two magazines
/// can live on different disks, and mixing the quotas would let loops starve
/// the space the upload quota exists to protect — the music.
#[derive(Debug, Clone, Copy)]
pub struct CanvasLimits {
    /// The largest canvas the server will take. The route derives its own body
    /// ceiling from this, so it is also the largest body that route accepts.
    pub max_bytes: i64,
    /// How long a loop may run. Not prudence: without it, "a short loop"
    /// becomes video hosting, which is a different product with different
    /// costs.
    pub max_duration_secs: u32,
    /// How much of a library's disk canvases may occupy. Counted in distinct
    /// blobs a library references, never in links, so an album's shared loop is
    /// billed once however many tracks name it.
    pub library_quota_bytes: i64,
}

/// The upload limits that only make sense against each other.
///
/// Apart here rather than inline so the rules can be exercised without
/// rewriting the process environment, which is global and shared by every test
/// running beside it.
pub(super) fn validate_uploads(uploads: &UploadLimits) -> anyhow::Result<()> {
    if uploads.max_file_bytes > uploads.library_quota_bytes {
        anyhow::bail!(
            "WAVEFLOW_UPLOAD_MAX_FILE_BYTES cannot exceed WAVEFLOW_UPLOAD_LIBRARY_QUOTA_BYTES"
        );
    }
    // The fragment route turns this into a body ceiling, and a value it cannot
    // represent would have to fall back to something. Every fallback for a
    // ceiling is wrong — too small breaks the configured size, too large is no
    // ceiling at all — so it is refused here, where the operator can see why.
    if usize::try_from(uploads.chunk_bytes).is_err() {
        anyhow::bail!("invalid WAVEFLOW_UPLOAD_CHUNK_BYTES: too large for this platform");
    }
    if uploads.chunk_bytes > uploads.max_file_bytes {
        anyhow::bail!("WAVEFLOW_UPLOAD_CHUNK_BYTES cannot exceed WAVEFLOW_UPLOAD_MAX_FILE_BYTES");
    }
    Ok(())
}

/// The canvas limits that only make sense against each other.
pub(super) fn validate_canvas(canvas: &CanvasLimits) -> anyhow::Result<()> {
    if canvas.max_bytes > canvas.library_quota_bytes {
        anyhow::bail!(
            "WAVEFLOW_CANVAS_MAX_BYTES cannot exceed WAVEFLOW_CANVAS_LIBRARY_QUOTA_BYTES"
        );
    }
    // The route turns this into a body ceiling, and a value it cannot represent
    // would have to fall back to something. Every fallback for a ceiling is
    // wrong, so it is refused here where the operator can see why — the same
    // rule the upload chunk size follows.
    if usize::try_from(canvas.max_bytes).is_err() {
        anyhow::bail!("invalid WAVEFLOW_CANVAS_MAX_BYTES: too large for this platform");
    }
    Ok(())
}
