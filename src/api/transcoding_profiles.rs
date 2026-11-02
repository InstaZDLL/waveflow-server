//! Transcoding profiles (RFC-027).
//!
//! Tagged on their own rather than `user-data`, for the reason `radio.rs` gives:
//! that tag selects the routes carrying the operation-id protocol, and this is
//! per-account configuration no client replicates.

use super::*;

/// A row to store, whole: a second `PUT` of the same `client` replaces it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct TranscodingProfileRequest {
    /// The Subsonic client name this overrides the default for, trimmed and
    /// compared without ASCII case. Left out, the row is the account's default.
    #[serde(default)]
    pub client: Option<String>,
    /// `mp3`, `opus` or `aac`; on an override also `raw`, which keeps that
    /// client on the original.
    pub format: String,
    /// The ceiling, from 32 to 512 kbit/s. Left out, the format's default; `raw`
    /// takes none.
    #[serde(default)]
    pub bitrate: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct TranscodingProfileQuery {
    pub client: Option<String>,
}

#[utoipa::path(get, path = "/api/v2/transcoding-profiles", tag = "transcoding-profiles", responses((status = 200, body = crate::services::TranscodingProfiles), (status = 401, body = ErrorResponse)))]
pub async fn get_transcoding_profiles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<crate::services::TranscodingProfiles>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .transcoding_profiles(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(put, path = "/api/v2/transcoding-profiles", tag = "transcoding-profiles", request_body = TranscodingProfileRequest, responses((status = 200, body = crate::services::TranscodingProfileItem), (status = 401, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn put_transcoding_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<TranscodingProfileRequest>,
) -> Result<Json<crate::services::TranscodingProfileItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .set_transcoding_profile(
            user.id,
            request.client.as_deref(),
            &request.format,
            request.bitrate,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(delete, path = "/api/v2/transcoding-profiles", tag = "transcoding-profiles", params(("client" = Option<String>, Query, description = "The override to remove; left out, the default")), responses((status = 204), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn delete_transcoding_profile(
    State(state): State<AppState>,
    Query(query): Query<TranscodingProfileQuery>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .delete_transcoding_profile(user.id, query.client.as_deref())
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
