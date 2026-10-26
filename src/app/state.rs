//! `AppState`, the singletons every surface shares, and the body ceilings it
//! derives from the upload limits.

use std::sync::Arc;

use crate::{
    auth::authentication, auth::security, auth::setup_token, config, library::scanner, media,
    services, storage::database, storage::sync,
};

#[derive(Clone)]
pub struct AppState {
    pub db: database::Database,
    pub auth: authentication::AuthService,
    pub secret_box: Arc<security::SecretBox>,
    pub scanner: scanner::ScanManager,
    pub media: media::MediaService,
    pub services: services::DomainServices,
    pub sync: sync::SyncService,
    pub artwork_dir: std::path::PathBuf,
    pub instance_key_path: std::path::PathBuf,
    pub public_url: Option<String>,
    pub stream_ticket_ttl: std::time::Duration,
    pub refresh_token_ttl: std::time::Duration,
    /// The body ceiling the upload negotiation route carries, derived from how
    /// many offers a batch may hold.
    ///
    /// The router's global limit is sixteen kilobytes and stays there: an API
    /// whose every route accepts that much cannot be drowned in a request body,
    /// and that property is worth more than the convenience of raising it once.
    /// A batch of two hundred offers does not fit in it, so the route that
    /// needs the room asks for it alone.
    pub upload_batch_body_limit: usize,
    /// The body ceiling one fragment may occupy.
    ///
    /// A little above the advertised fragment size, for the framing rather than
    /// for slack: a client that sends more than it was told to is refused by
    /// the route before the service has to read it.
    pub upload_chunk_body_limit: usize,
    /// The body ceiling the canvas route carries, derived from
    /// `WAVEFLOW_CANVAS_MAX_BYTES` so the two cannot disagree.
    pub canvas_body_limit: usize,
    /// `WAVEFLOW_TRUSTED_PROXIES`: whose `X-Forwarded-For` names the client a
    /// sign-in is counted under (RFC-017 decision 2).
    pub trusted_proxies: Arc<[ipnet::IpNet]>,
    /// The one-time token first-run setup asks for while no account exists
    /// (RFC-019 decision 1).
    pub setup: Arc<setup_token::SetupGate>,
}

/// Roughly one offer's worth of JSON — a sixty-four character hash, a size, a
/// short extension and their punctuation — times the batch, with a fixed
/// allowance for the envelope. Derived rather than configured, so a raised
/// batch limit cannot quietly start rejecting the batches it just permitted.
pub(super) fn negotiation_body_limit(limits: &config::UploadLimits) -> usize {
    limits.batch_limit.saturating_mul(160).saturating_add(1024)
}

/// One fragment, plus room for its framing.
///
/// `Config` refuses a fragment size this platform cannot represent, so the
/// fallback below is unreachable. It is a small finite number rather than
/// `usize::MAX` anyway: a ceiling that falls back to "no ceiling" is the one
/// mistake this value must never make, and an unreachable branch is exactly
/// where that would go unnoticed.
pub(super) fn chunk_body_limit(limits: &config::UploadLimits) -> usize {
    const FALLBACK: usize = 16 * 1024 * 1024;
    usize::try_from(limits.chunk_bytes)
        .unwrap_or(FALLBACK)
        .saturating_add(4096)
}
