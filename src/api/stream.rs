//! Streaming endpoints. Two surfaces:
//!
//! - **Mint** (`POST /api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{track_id}/stream-url`):
//!   JWT-authed. Verifies tenant ownership of the track, then signs
//!   a short-lived URL the browser can drop into `<audio src>` with
//!   no header. Returns `{ url, expires_at }`.
//! - **Stream** (`GET /api/v1/stream/{token}`): NOT behind the JWT
//!   middleware — the token IS the auth. Verifies the HMAC, resolves
//!   the path against `WAVEFLOW_MUSIC_ROOT` with a canonical-prefix
//!   check (path-traversal guard), and streams the file. Supports
//!   `Range: bytes=N-M` for browser scrubbing.
//!
//! The signing material is `(file_path, exp)`. The path is enough on
//! the verifying side because the mint endpoint already proved tenant
//! ownership; lookup-by-token-then-tenant-check would be redundant
//! plus a DB hit on every range byte.
//!
//! Range handling is hand-rolled (not via `tower-http::services::ServeFile`)
//! so we can stream a single canonical file resolved per-token rather
//! than expose an entire directory.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use waveflow_core::repository::postgres::PostgresTrackRepository;

use crate::{
    middleware::UserId,
    stream_token::{self, StreamClaim, MAX_LIFETIME_SECS},
    AppState,
};

/// Default expiry the mint endpoint vends. Caps the leak window of
/// any URL the browser might log or cache.
const DEFAULT_LIFETIME_SECS: u64 = 60;

/// Read buffer / range chunk size — 64 KiB matches what
/// `tokio::io::BufReader` defaults to and what most HTTP/2 frames
/// expect.
const STREAM_CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug, Serialize, ToSchema)]
pub struct MintResponse {
    /// Server-relative URL the browser can drop into `<audio src>`.
    /// Includes the host-relative prefix `/api/v1/stream/...`.
    pub url: String,
    /// Unix epoch second past which the token will be rejected.
    #[schema(example = 1735603260)]
    pub expires_at: u64,
}

pub fn auth_router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(mint_stream_url))
        .with_state(state)
}

pub fn public_router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(stream_audio))
        .with_state(state)
}

/// Mint a signed streaming URL for a track the caller owns. Issues
/// 503 when streaming is disabled at boot (`WAVEFLOW_MUSIC_ROOT` +
/// `WAVEFLOW_STREAM_SECRET` not set).
#[utoipa::path(
    post,
    path = "/api/v1/profiles/{profile_id}/libraries/{library_id}/tracks/{track_id}/stream-url",
    tag = "stream",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("profile_id" = i64, Path, description = "Owning profile id"),
        ("library_id" = i64, Path, description = "Owning library id"),
        ("track_id" = i64, Path, description = "Track to stream"),
    ),
    responses(
        (status = 200, description = "Signed URL minted", body = MintResponse),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 404, description = "Track does not belong to the calling user"),
        (status = 503, description = "Streaming disabled at boot"),
    ),
)]
async fn mint_stream_url(
    State(state): State<AppState>,
    Extension(UserId(user_id)): Extension<UserId>,
    Path((profile_id, library_id, track_id)): Path<(i64, i64, i64)>,
) -> Result<Json<MintResponse>, StatusCode> {
    let ctx = state
        .stream_ctx
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Tenant-scoped lookup — non-owners get 404 (same no-leak rule
    // as the rest of the resource endpoints).
    let repo = PostgresTrackRepository::new(state.db.clone());
    let track = repo
        .get_for_library(track_id, library_id, profile_id, user_id)
        .await
        .map_err(|err| {
            tracing::error!(error = %err, "stream mint lookup failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let now = unix_now();
    let exp = now.saturating_add(DEFAULT_LIFETIME_SECS.min(MAX_LIFETIME_SECS));
    let claim = StreamClaim {
        p: track.file_path,
        exp,
    };

    let token = stream_token::mint(&ctx.secret, &claim).map_err(|err| {
        tracing::error!(error = %err, "stream token mint failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(MintResponse {
        url: format!("/api/v1/stream/{token}"),
        expires_at: exp,
    }))
}

/// Stream a file referenced by a signed token. Verifies the HMAC,
/// canonicalises the path against the configured music root (path-
/// traversal guard), then serves the bytes with `Range` support.
#[utoipa::path(
    get,
    path = "/api/v1/stream/{token}",
    tag = "stream",
    params(
        ("token" = String, Path, description = "HMAC-signed token vended by the mint endpoint"),
    ),
    responses(
        (status = 200, description = "Full file body", content_type = "application/octet-stream"),
        (status = 206, description = "Range response (browser seeking)", content_type = "application/octet-stream"),
        (status = 401, description = "Token signature mismatch or expired"),
        (status = 404, description = "File not found under the configured music root"),
        (status = 416, description = "Requested range is unsatisfiable"),
        (status = 503, description = "Streaming disabled at boot"),
    ),
)]
async fn stream_audio(
    State(state): State<AppState>,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<Response, StatusCode> {
    let ctx = state
        .stream_ctx
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    let claim = stream_token::verify(&ctx.secret, &token, unix_now()).map_err(|err| {
        tracing::warn!(error = %err, "stream token rejected");
        StatusCode::UNAUTHORIZED
    })?;

    let resolved = resolve_path(&ctx.music_root, &claim.p)?;

    // `.metadata` to learn the total length up-front — needed for
    // both the `Content-Length` header on a full response and the
    // `Content-Range` denominator on a partial one.
    let metadata = tokio::fs::metadata(&resolved).await.map_err(|err| {
        tracing::warn!(error = %err, "stream file metadata failed");
        StatusCode::NOT_FOUND
    })?;
    let total = metadata.len();

    let content_type = guess_mime(&resolved);

    let range_header = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    match range_header.as_deref() {
        None => serve_full(&resolved, total, content_type).await,
        Some(raw) => match parse_range(raw, total) {
            Some((start, end)) => serve_partial(&resolved, start, end, total, content_type).await,
            None => Err(StatusCode::RANGE_NOT_SATISFIABLE),
        },
    }
}

async fn serve_full(
    path: &PathBuf,
    total: u64,
    content_type: &'static str,
) -> Result<Response, StatusCode> {
    let file = tokio::fs::File::open(path).await.map_err(|err| {
        tracing::warn!(error = %err, "stream file open failed");
        StatusCode::NOT_FOUND
    })?;

    let stream = tokio_util::io::ReaderStream::with_capacity(file, STREAM_CHUNK_SIZE);
    let body = axum::body::Body::from_stream(stream);

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(header::CONTENT_LENGTH, total.into());
    Ok((StatusCode::OK, headers, body).into_response())
}

async fn serve_partial(
    path: &PathBuf,
    start: u64,
    end: u64,
    total: u64,
    content_type: &'static str,
) -> Result<Response, StatusCode> {
    let mut file = tokio::fs::File::open(path).await.map_err(|err| {
        tracing::warn!(error = %err, "stream file open failed");
        StatusCode::NOT_FOUND
    })?;
    file.seek(SeekFrom::Start(start)).await.map_err(|err| {
        tracing::warn!(error = %err, "stream file seek failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Read the requested slice into memory. A streaming variant
    // would clamp `ReaderStream` to `end - start + 1` bytes, but
    // the chunk size for audio scrubbing is typically <= a few MB
    // — well within what we can buffer cheaply. Revisit if range
    // sizes grow.
    let len = end - start + 1;
    let mut buf = vec![0u8; len as usize];
    file.read_exact(&mut buf).await.map_err(|err| {
        tracing::warn!(error = %err, "stream file read failed");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(header::CONTENT_LENGTH, len.into());
    let content_range = HeaderValue::from_str(&format!("bytes {start}-{end}/{total}"))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    headers.insert(header::CONTENT_RANGE, content_range);

    Ok((StatusCode::PARTIAL_CONTENT, headers, buf).into_response())
}

/// Parse a `Range` header of the form `bytes=N-M`, `bytes=N-` or
/// `bytes=-N`. Returns `(start, end_inclusive)` clamped to the file
/// size; `None` if the header is malformed or the resulting range
/// is empty (`start > end`).
fn parse_range(raw: &str, total: u64) -> Option<(u64, u64)> {
    let spec = raw.strip_prefix("bytes=")?;
    // We don't honour multi-range requests — the spec allows
    // comma-separated ranges but no browser audio player needs them.
    if spec.contains(',') {
        return None;
    }
    let (start_s, end_s) = spec.split_once('-')?;
    if total == 0 {
        return None;
    }
    let last = total - 1;

    let (start, end) = match (start_s.trim(), end_s.trim()) {
        ("", "") => return None,
        ("", suffix) => {
            // `bytes=-N` — the last N bytes.
            let n: u64 = suffix.parse().ok()?;
            if n == 0 {
                return None;
            }
            let n = n.min(total);
            (total - n, last)
        }
        (start_s, "") => {
            // `bytes=N-` — from N to EOF.
            let start: u64 = start_s.parse().ok()?;
            if start > last {
                return None;
            }
            (start, last)
        }
        (start_s, end_s) => {
            let start: u64 = start_s.parse().ok()?;
            let end: u64 = end_s.parse().ok()?;
            if start > end || start > last {
                return None;
            }
            (start, end.min(last))
        }
    };

    Some((start, end))
}

/// Join `music_root + claim.p`, then canonicalise and verify the
/// result still lives under `music_root`. Defends against `..`
/// traversal AND symlinks pointing outside the root.
fn resolve_path(music_root: &PathBuf, rel: &str) -> Result<PathBuf, StatusCode> {
    // Strip an accidental leading separator so `music_root.join` doesn't
    // treat the relative path as absolute and discard the root.
    let trimmed = rel.trim_start_matches(['/', '\\']);
    let candidate = music_root.join(trimmed);

    // std::fs::canonicalize is sync, but the per-stream call is
    // amortised against the streaming itself — running it on the
    // tokio runtime is fine in practice. A blocking-task hop would
    // be an optimisation, not a correctness requirement.
    let canonical = match std::fs::canonicalize(&candidate) {
        Ok(p) => p,
        Err(err) => {
            tracing::warn!(error = %err, path = %candidate.display(), "stream path canonicalize failed");
            return Err(StatusCode::NOT_FOUND);
        }
    };

    if !canonical.starts_with(music_root) {
        tracing::warn!(
            requested = %rel,
            resolved = %canonical.display(),
            "stream path resolved outside music root — rejecting"
        );
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(canonical)
}

fn guess_mime(path: &PathBuf) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("ogg" | "oga") => "audio/ogg",
        Some("opus") => "audio/opus",
        Some("m4a" | "mp4") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("aiff" | "aif") => "audio/aiff",
        // Default for unknown extensions — the browser still plays
        // most formats from `application/octet-stream`, just without
        // any container-specific UA optimisation.
        _ => "application/octet-stream",
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::parse_range;

    #[test]
    fn parses_an_open_range() {
        assert_eq!(parse_range("bytes=0-", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=500-", 1000), Some((500, 999)));
    }

    #[test]
    fn parses_a_closed_range() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=500-700", 1000), Some((500, 700)));
    }

    #[test]
    fn clamps_a_closed_range_to_file_end() {
        assert_eq!(parse_range("bytes=900-9999", 1000), Some((900, 999)));
    }

    #[test]
    fn parses_a_suffix_range() {
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        // Suffix wider than the file → return the whole file.
        assert_eq!(parse_range("bytes=-9999", 1000), Some((0, 999)));
    }

    #[test]
    fn rejects_malformed_ranges() {
        assert!(parse_range("0-99", 1000).is_none(), "missing bytes= prefix");
        assert!(parse_range("bytes=", 1000).is_none(), "empty range");
        assert!(parse_range("bytes=abc-def", 1000).is_none(), "non-numeric");
        assert!(parse_range("bytes=500-100", 1000).is_none(), "start > end");
        assert!(
            parse_range("bytes=2000-2999", 1000).is_none(),
            "start past EOF"
        );
        assert!(
            parse_range("bytes=0-99,200-299", 1000).is_none(),
            "multi-range unsupported"
        );
    }

    #[test]
    fn rejects_ranges_against_an_empty_file() {
        assert!(parse_range("bytes=0-", 0).is_none());
    }
}
