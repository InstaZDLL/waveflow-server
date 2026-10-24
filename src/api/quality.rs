//! The real quality of a file (RFC-016).
//!
//! Handlers orchestrate HTTP only; the rules live in `DomainServices`. Any
//! member of a library reads its quality; asking for an analysis again is for
//! its owners and managers, and everyone else gets the same `404` a library
//! they are not in would.

use super::*;

#[derive(Debug, Deserialize)]
pub struct QualityTracksQuery {
    pub verdict: Option<String>,
    pub codec: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// A library's counts per verdict, and how far the analysis has got.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/quality", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 200, body = crate::services::QualitySummary), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn quality_summary(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::QualitySummary>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .quality_summary(user.id, library_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// One page of a library's files with their verdicts, worst first. `422` for
/// a verdict the server does not know.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/quality/tracks", tag = "catalog", params(("library_id" = Uuid, Path), ("verdict" = Option<String>, Query), ("codec" = Option<String>, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::QualityTrack]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn quality_tracks(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    Query(query): Query<QualityTracksQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::QualityTrack>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let verdict = match query.verdict.as_deref().filter(|value| !value.is_empty()) {
        Some(value) => {
            Some(crate::services::QualityVerdict::parse(value).ok_or(ApiError::Validation)?)
        }
        None => None,
    };
    let codec = query.codec.as_deref().filter(|value| !value.is_empty());
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .quality_tracks(user.id, library_id, verdict, codec, page)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Deletes the library's measurements and queues its files. Owners and
/// managers only.
#[utoipa::path(post, path = "/api/v2/libraries/{library_id}/quality/analysis", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 202), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn request_quality_analysis(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .request_quality_analysis(user.id, library_id)
        .await
        .map(|()| StatusCode::ACCEPTED)
        .map_err(service_error)
}

/// One track's verdict, the numbers behind it, and its averaged spectrum.
#[utoipa::path(get, path = "/api/v2/tracks/{track_id}/quality", tag = "catalog", params(("track_id" = Uuid, Path)), responses((status = 200, body = crate::services::TrackQualityDetail), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn track_quality(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::TrackQualityDetail>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .track_quality(user.id, track_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// The track's spectrogram, drawn by FFmpeg on demand: WebP, or JPEG when
/// FFmpeg has no libwebp. The browser may keep it but asks again before every
/// use; the answer is `304` only once the caller's access has been checked.
/// `429` with `Retry-After` while the caller, or the server, is already drawing
/// as many as it may.
#[utoipa::path(get, path = "/api/v2/tracks/{track_id}/spectrogram", tag = "catalog", params(("track_id" = Uuid, Path)), responses((status = 200, description = "The picture", content_type = "image/webp"), (status = 304), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 429, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub async fn track_spectrogram(
    State(state): State<AppState>,
    Path(track_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let if_none_match = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok());
    let answer = match state
        .services
        .track_spectrogram(user.id, track_id, if_none_match)
        .await
    {
        Ok(answer) => answer,
        Err(crate::services::ServiceError::Busy) => {
            let mut response = ApiError::TooManyRequests.into_response();
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("2"));
            return Ok(response);
        }
        Err(error) => return Err(service_error(error)),
    };
    let cache = HeaderValue::from_static("private, no-cache");
    Ok(match answer {
        crate::services::Spectrogram::NotModified { etag } => (
            StatusCode::NOT_MODIFIED,
            [
                (header::CACHE_CONTROL, cache),
                (
                    header::ETAG,
                    HeaderValue::from_str(&etag).map_err(|_| ApiError::Unavailable)?,
                ),
            ],
        )
            .into_response(),
        crate::services::Spectrogram::Picture {
            etag,
            bytes,
            content_type,
        } => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
                (header::CACHE_CONTROL, cache),
                (
                    header::ETAG,
                    HeaderValue::from_str(&etag).map_err(|_| ApiError::Unavailable)?,
                ),
            ],
            bytes,
        )
            .into_response(),
    })
}
