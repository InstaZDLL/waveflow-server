//! `/api/v1/artwork/*` — shared artwork cache (Phase 1.h.1).
//!
//! Two surfaces split along the same auth-vs-public boundary as
//! streaming and share:
//!
//! - **Upload** (`POST /api/v1/artwork`): JWT-authed. Body is the
//!   raw image bytes (no multipart wrapper — keeps the path simple
//!   and Bytes-extractor-friendly). `Content-Type` must be one of
//!   `image/jpeg`, `image/png`, `image/webp`. Hash is computed
//!   server-side (BLAKE3 of the bytes) so the client can't tamper
//!   with the identity; a re-upload of the same content returns
//!   the same hash without re-writing. Caps at 4 MiB to keep the
//!   per-request memory footprint bounded — same ceiling the
//!   desktop's `metadata_artwork::MAX_BYTES` already enforces, so
//!   the two caches stay symmetric.
//! - **Public read** (`GET /api/v1/artwork/{hash}`): NOT behind the
//!   JWT middleware — the hash IS the credential, same model as
//!   the share token. 64 hex chars are validated at the boundary so
//!   `..` or `/` can't slip into the object_store key. Cache
//!   headers mark the response `immutable` since BLAKE3 makes
//!   re-fetching the same hash idempotent forever.
//!
//! Storage layout is flat (`artwork/<hash>` in object_store); the
//! MIME type lives in the `metadata_artwork` row alongside the
//! original byte size, so the GET handler vends the right
//! Content-Type without round-tripping the store for a HEAD.
//!
//! Both routes answer 503 when storage isn't configured (env
//! `WAVEFLOW_ARTWORK_LOCAL_DIR` unset) — same opt-in shape as the
//! streaming surface, so a deploy that doesn't want the feature
//! just leaves the var off.

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, Path, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};

use crate::{middleware::UserId, storage::StorageError, AppState};

/// Hard cap on a single artwork upload. 4 MiB matches the desktop's
/// `metadata_artwork::MAX_BYTES`, which in turn was sized for
/// Deezer's `picture_xl` (~200 KB) with comfortable headroom for
/// arbitrary user uploads (album scans, scanned vinyl sleeves) once
/// the artwork pipeline ships in 1.h.3.
const MAX_UPLOAD_BYTES: usize = 4 * 1024 * 1024;

/// Accepted MIME types. Each is byte-perfect-streamable straight
/// from object_store with the recorded Content-Type — no
/// re-encoding in 1.h.1. PNG is included up front because it's the
/// common pasteboard format on Linux desktops; WebP because modern
/// cover-art services already vend it and re-encoding to JPEG would
/// lose alpha + waste bytes.
const ACCEPTED_MIMES: &[&str] = &["image/jpeg", "image/png", "image/webp"];

#[derive(Debug, Serialize, ToSchema)]
pub struct UploadResponse {
    /// BLAKE3 hex digest of the uploaded bytes — the row identity in
    /// `metadata_artwork` and the path segment for the public read.
    /// Stable for the lifetime of the cache; clients can persist
    /// this as a `cover_hash` / `picture_hash` reference.
    pub hash: String,
    /// Original byte count. Echoed back so the client doesn't have to
    /// re-measure the bytes they just sent.
    pub byte_size: i64,
    /// MIME type the server stored (mirrors the `Content-Type`
    /// header the client sent). Echoed back so a future client can
    /// learn the canonical normalisation (e.g. if we ever map
    /// `image/jpg` → `image/jpeg`).
    pub mime: String,
    /// Path-relative URL the bytes are served at. Combine with the
    /// origin to fetch — kept relative because the server doesn't
    /// know the public origin in a reverse-proxied deploy.
    pub url: String,
}

pub fn auth_router(state: AppState) -> OpenApiRouter {
    // Raise the default axum body limit above our 4 MiB cap so the
    // handler's `body.len() > MAX_UPLOAD_BYTES` check is the
    // authoritative gatekeeper. Without this, axum 0.8's default
    // 2 MiB limit short-circuits the request to 413 before the
    // handler runs — which is technically correct but hides our
    // "exactly 4 MiB allowed" contract. The +1 KiB of slack lets a
    // borderline 4 MiB upload reach the handler so the boundary
    // case lands on our error variant (and the matching test
    // observes a real, handler-issued 413).
    OpenApiRouter::new()
        .routes(routes!(upload_artwork))
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES + 1024))
        .with_state(state)
}

pub fn public_router(state: AppState) -> OpenApiRouter {
    OpenApiRouter::new()
        .routes(routes!(get_artwork))
        .with_state(state)
}

/// Upload an artwork file. Body is raw image bytes (no multipart),
/// `Content-Type` MUST be one of `image/jpeg`, `image/png`,
/// `image/webp`. Idempotent — re-uploading the same content returns
/// the same hash and skips the storage write.
///
/// Responses:
/// - 200: `{ hash, byte_size, mime, url }`
/// - 400: missing or unsupported Content-Type, empty body
/// - 401: missing or invalid bearer
/// - 413: body exceeded 4 MiB cap
/// - 503: artwork storage disabled at boot
#[utoipa::path(
    post,
    path = "/api/v1/artwork",
    tag = "artwork",
    params(
        ("authorization" = String, Header, description = "Bearer JWT issued by Better Auth"),
        ("content-type" = String, Header, description = "image/jpeg | image/png | image/webp"),
    ),
    request_body(
        content_type = "image/jpeg",
        description = "Raw image bytes. Capped at 4 MiB.",
    ),
    responses(
        (status = 200, description = "Bytes accepted (or already cached)", body = UploadResponse),
        (status = 400, description = "Unsupported MIME, empty body, or other client-side validation failure"),
        (status = 401, description = "Missing or invalid bearer token"),
        (status = 413, description = "Body exceeded 4 MiB cap"),
        (status = 500, description = "Storage or database failure"),
        (status = 503, description = "Artwork storage disabled at boot"),
    ),
)]
async fn upload_artwork(
    State(state): State<AppState>,
    // `_user_id` is unused today — uploads aren't tenant-scoped
    // because the artwork cache is dedup'd across the whole
    // deployment by BLAKE3 hash. The Extension binding is still
    // present so the middleware's auth check runs (you only reach
    // this handler with a valid Bearer), and so future per-user
    // quotas can hook in here without an API change.
    Extension(UserId(_user_id)): Extension<UserId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ArtworkError> {
    let storage = state.artwork.as_ref().ok_or(ArtworkError::Disabled)?;

    if body.is_empty() {
        return Err(ArtworkError::EmptyBody);
    }
    if body.len() > MAX_UPLOAD_BYTES {
        return Err(ArtworkError::TooLarge);
    }

    let mime = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        // `Content-Type: image/jpeg; charset=binary` shouldn't be
        // rejected over a parameter we don't care about — split on
        // ';' and trim, then compare the bare media type.
        .map(|s| s.split(';').next().unwrap_or(s).trim().to_ascii_lowercase())
        .ok_or(ArtworkError::UnsupportedMime)?;
    if !ACCEPTED_MIMES.contains(&mime.as_str()) {
        return Err(ArtworkError::UnsupportedMime);
    }

    let hash = blake3::hash(body.as_ref()).to_hex().to_string();
    let byte_size = i64::try_from(body.len()).expect("4 MiB cap fits in i64");

    // Ordering: bytes first, row second. A racing GET that sees the
    // row knows the bytes are already committed (storage.put has
    // returned); a GET that arrives between bytes-written and row-
    // inserted simply 404s on the meta lookup and the client retries.
    // The reverse order would race a GET against the brief window
    // where the row exists but the storage put hasn't returned yet,
    // producing a 500 instead of a clean 404.
    //
    // Optimisation: skip the storage write if the metadata row
    // already exists. A re-upload of the same bytes is a no-op then,
    // saving a backend round-trip in the common case (the desktop
    // bundles cover bytes with sync ops and would otherwise re-send
    // every cached cover on each sync).
    if let Some(existing) = crate::db::artwork::fetch_meta(&state.db, &hash)
        .await
        .map_err(ArtworkError::Db)?
    {
        // Echo the stored row, not the incoming request: BLAKE3
        // collision-resistance means `byte_size` is identical
        // either way, but `mime` could diverge under a future
        // canonicalisation (the DTO already advertises this
        // contract — see `UploadResponse::mime`), and reading
        // through the row is what makes the documented behaviour
        // unconditionally true.
        return Ok((
            StatusCode::OK,
            Json(UploadResponse {
                hash: hash.clone(),
                byte_size: existing.byte_size,
                mime: existing.mime,
                url: format!("/api/v1/artwork/{hash}"),
            }),
        )
            .into_response());
    }

    storage
        .put(&hash, body)
        .await
        .map_err(ArtworkError::Storage)?;

    // Race-safe: another concurrent upload of the same bytes may
    // have inserted the row between our `fetch_meta` and `put`.
    // `insert_if_absent` collapses the duplicate via
    // `ON CONFLICT DO NOTHING` and we treat both outcomes (we won,
    // we lost) as success — the bytes are now durably stored either
    // way.
    let _new_row = crate::db::artwork::insert_if_absent(&state.db, &hash, &mime, byte_size)
        .await
        .map_err(ArtworkError::Db)?;

    Ok((
        StatusCode::OK,
        Json(UploadResponse {
            hash: hash.clone(),
            byte_size,
            mime,
            url: format!("/api/v1/artwork/{hash}"),
        }),
    )
        .into_response())
}

/// Fetch artwork bytes by hash. Public — no JWT, the 64-hex hash
/// IS the credential (same model as `share` tokens).
///
/// Responses:
/// - 200: image bytes with the original Content-Type
/// - 400: malformed hash (wrong length / non-hex)
/// - 404: no row matches
/// - 503: artwork storage disabled at boot
#[utoipa::path(
    get,
    path = "/api/v1/artwork/{hash}",
    tag = "artwork",
    params(
        ("hash" = String, Path, description = "BLAKE3 hex digest of the bytes (64 lowercase hex chars)"),
    ),
    responses(
        (status = 200, description = "Image bytes", content_type = "image/jpeg"),
        (status = 400, description = "Malformed hash"),
        (status = 404, description = "No artwork stored under this hash"),
        (status = 503, description = "Artwork storage disabled at boot"),
    ),
)]
async fn get_artwork(
    State(state): State<AppState>,
    Path(hash): Path<String>,
) -> Result<Response, ArtworkError> {
    let storage = state.artwork.as_ref().ok_or(ArtworkError::Disabled)?;

    if !crate::storage::is_well_shaped_hash(&hash) {
        return Err(ArtworkError::MalformedHash);
    }

    // Metadata lookup first — cheap (covered by PK) and tells us the
    // MIME type for the response. A miss here short-circuits the
    // storage call so we never log a backend NotFound for a hash that
    // was never registered.
    let meta = crate::db::artwork::fetch_meta(&state.db, &hash)
        .await
        .map_err(ArtworkError::Db)?
        .ok_or(ArtworkError::NotFound)?;

    let bytes = storage.get(&hash).await.map_err(|err| match err {
        StorageError::NotFound => ArtworkError::NotFound,
        StorageError::Backend(e) => ArtworkError::Storage(StorageError::Backend(e)),
    })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&meta.mime)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(bytes.len()));
    // BLAKE3-keyed assets are immutable by construction: the hash
    // changes if the bytes change. `immutable` lets browsers skip the
    // revalidation on reload; 1y is the conventional "forever" ceiling
    // for HTTP caches.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    // ETag = the hash itself. Lets a CDN dedupe across regions and
    // a client honour `If-None-Match` for a cheap revalidate.
    headers.insert(
        header::ETAG,
        HeaderValue::from_str(&format!("\"{}\"", meta.hash))
            .unwrap_or_else(|_| HeaderValue::from_static("\"artwork\"")),
    );

    Ok((StatusCode::OK, headers, bytes).into_response())
}

/// Local error type so each variant maps to its own status code in
/// a single `IntoResponse` impl. Keeps the handler bodies free of
/// the match-on-status-code boilerplate.
#[derive(Debug)]
enum ArtworkError {
    Disabled,
    EmptyBody,
    TooLarge,
    UnsupportedMime,
    MalformedHash,
    NotFound,
    Storage(StorageError),
    Db(sqlx::Error),
}

impl IntoResponse for ArtworkError {
    fn into_response(self) -> Response {
        match self {
            Self::Disabled => (
                StatusCode::SERVICE_UNAVAILABLE,
                "artwork storage disabled at boot",
            )
                .into_response(),
            Self::EmptyBody => (StatusCode::BAD_REQUEST, "empty body").into_response(),
            Self::TooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "body exceeds 4 MiB").into_response(),
            Self::UnsupportedMime => (
                StatusCode::BAD_REQUEST,
                "Content-Type must be image/jpeg, image/png, or image/webp",
            )
                .into_response(),
            Self::MalformedHash => (
                StatusCode::BAD_REQUEST,
                "hash must be 64 lowercase hex characters",
            )
                .into_response(),
            Self::NotFound => (StatusCode::NOT_FOUND, "artwork not found").into_response(),
            Self::Storage(err) => {
                tracing::error!(error = %err, "artwork storage call failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response()
            }
            Self::Db(err) => {
                tracing::error!(error = %err, "artwork db call failed");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
            }
        }
    }
}
