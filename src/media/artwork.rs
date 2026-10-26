//! Covers: where a stored artwork is read from, and the route that serves it.

use axum::{
    extract::{Path as AxumPath, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use super::{bearer_principal, MediaError};
use crate::AppState;

/// Artwork addressed by its own hash: the bytes behind the URL cannot change,
/// so a client never needs to ask again.
const ARTWORK_CANONICAL_CACHE: &str = "private, max-age=31536000, immutable";
/// Artwork addressed by a track, album or artist: the URL resolves whichever
/// cover that entity carries *now*, and a rescan can move it.
const ARTWORK_ALIAS_CACHE: &str = "private, no-cache";

/// Content type for a stored artwork, or `None` for a format we never wrote.
///
/// Shared by the native route and the Subsonic facade: two copies would drift
/// on the day a format is added, and the client that got
/// `application/octet-stream` would simply show no cover.
pub fn artwork_mime(format: &str) -> Option<&'static str> {
    match format {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Reads an artwork file from the store.
///
/// `hash` and `format` come from the database, never from the request, so the
/// joined path cannot escape `artwork_dir` — the caller resolves the request's
/// id through `artwork_for_user`, which also enforces tenancy.
pub async fn read_artwork(
    artwork_dir: &std::path::Path,
    hash: &str,
    format: &str,
) -> Option<(&'static str, Vec<u8>)> {
    let mime = artwork_mime(format)?;
    let bytes = tokio::fs::read(artwork_dir.join(format!("{hash}.{format}")))
        .await
        .ok()?;
    Some((mime, bytes))
}

#[utoipa::path(
    get,
    path = "/api/v2/artwork/{artwork_id}",
    tag = "media",
    params((
        "artwork_id" = String,
        Path,
        description = "An `artwork_hash` from any catalogue payload, or the id of a track, \
                       album or artist that carries one."
    )),
    responses(
        (status = 200, description = "Cover image", content_type = "image/jpeg"),
        (status = 401),
        (status = 404)
    )
)]
/// Serves a cover behind the same bearer as the rest of the native API.
///
/// Catalogue payloads carry `artwork_hash` but nothing resolved it: only the
/// Subsonic facade served images, behind a separate credential. A native client
/// therefore showed a remote catalogue with no covers at all.
///
/// Access is re-checked per request through `artwork_for_user`, so a hash
/// guessed from another tenant's library answers 404 like anything else.
pub async fn artwork(
    State(state): State<AppState>,
    AxumPath(artwork_id): AxumPath<String>,
    headers: HeaderMap,
) -> Result<Response, MediaError> {
    let user = bearer_principal(&state, &headers).await?;
    let (hash, format) = state
        .services
        .artwork_for_user(user.id, &artwork_id)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "artwork lookup failed");
            MediaError::Internal
        })?
        .ok_or(MediaError::NotFound)?;

    // How the URL was addressed decides what a client may do with the answer,
    // and the two forms are not the same resource.
    //
    // Addressed by hash, the hash *is* the content: this URL can never answer
    // differently, so a client should stop revalidating it entirely. Addressed
    // by a track, album or artist id, the lookup above resolved that entity's
    // *current* cover — and a rescan that finds new embedded art moves it. The
    // alias must stay revalidatable or a replaced cover would be frozen in
    // every client's cache for a year.
    //
    // `private` on both, and neither becomes `public`: the route is
    // authenticated and tenant-checked, and two accounts whose libraries hold
    // the same cover share its hash, so a shared cache keyed on the URL would
    // hand one tenant's artwork to another with no credential in sight. A
    // private cache is the client's own and loses nothing by the restriction.
    let cache_control = if artwork_id == hash {
        ARTWORK_CANONICAL_CACHE
    } else {
        ARTWORK_ALIAS_CACHE
    };
    // The validator is the hash either way, which is what makes revalidating an
    // alias a 304 rather than a second full transfer of the same bytes.
    let etag = format!("\"{hash}\"");
    if if_none_match_hits(&headers, &etag) {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [
                (header::CACHE_CONTROL, cache_control.to_owned()),
                (header::ETAG, etag),
            ],
        )
            .into_response());
    }

    let (mime, bytes) = read_artwork(&state.artwork_dir, &hash, &format)
        .await
        .ok_or(MediaError::NotFound)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, mime.to_owned()),
            (header::CACHE_CONTROL, cache_control.to_owned()),
            (header::ETAG, etag),
        ],
        bytes,
    )
        .into_response())
}

/// Whether the request already holds this exact entity.
///
/// `*` matches anything the client has, which for a single-representation
/// resource is this one.
pub(super) fn if_none_match_hits(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .map(str::trim)
                .any(|candidate| candidate == etag || candidate == "*")
        })
}
