//! Scrobbles, history, now playing, the queue and transcode status.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct ScrobbleRequest {
    pub track_id: Uuid,
    /// `false` records a "now playing" ping, `true` a completed listen.
    #[serde(default)]
    pub submission: bool,
    pub played_at: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SaveQueueRequest {
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
    pub current: Option<Uuid>,
    #[serde(default)]
    pub position_ms: i64,
    pub client: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NowPlayingEntry {
    pub username: String,
    pub song: crate::services::SongItem,
    pub started_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TranscodeStatusResponse {
    pub available: bool,
    pub active: usize,
    /// How many transcodes the whole server may run at once, and how many one
    /// account may. `active` alone says how busy the server is without saying
    /// how busy it is allowed to get, so a client had no way to size its own
    /// concurrency except by being refused.
    pub global_limit: usize,
    pub per_user_limit: usize,
}

#[utoipa::path(post, path = "/api/v2/scrobbles", tag = "user-data", request_body = ScrobbleRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_scrobble(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ScrobbleRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .scrobble_with_context(
            user.id,
            request.track_id,
            request.submission,
            request.played_at,
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/history", tag = "user-data", params(("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::HistoryItem]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_history(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Vec<crate::services::HistoryItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let limit = query.limit.unwrap_or(200);
    if !(1..=crate::sync::MAX_SYNC_LIMIT).contains(&limit) {
        return Err(ApiError::Validation);
    }
    state
        .services
        .history(user.id, limit)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/now-playing", tag = "user-data", responses((status = 200, body = [NowPlayingEntry]), (status = 401, body = ErrorResponse)))]
pub async fn list_now_playing(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<NowPlayingEntry>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let entries = state
        .services
        .now_playing(user.id)
        .await
        .map_err(service_error)?
        .into_iter()
        .map(|(username, song, started_at)| NowPlayingEntry {
            username,
            song,
            started_at,
        })
        .collect();
    Ok(Json(entries))
}

#[utoipa::path(get, path = "/api/v2/queue", tag = "user-data", responses((status = 200, body = Option<crate::services::QueueItem>), (status = 401, body = ErrorResponse)))]
pub async fn get_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Option<crate::services::QueueItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .queue(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(put, path = "/api/v2/queue", tag = "user-data", request_body = SaveQueueRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn save_queue(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SaveQueueRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .save_queue_with_context(
            user.id,
            &request.track_ids,
            request.current,
            request.position_ms,
            request.client.as_deref(),
            context,
        )
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/transcode/status", tag = "catalog", responses((status = 200, body = TranscodeStatusResponse), (status = 401, body = ErrorResponse)))]
pub async fn transcode_status(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TranscodeStatusResponse>, ApiError> {
    authenticated(&state, &headers, Access::Read).await?;
    let (global_limit, per_user_limit) = state.media.transcode_limits();
    Ok(Json(TranscodeStatusResponse {
        available: state.media.transcoding_available(),
        active: state.media.active_transcodes(),
        global_limit,
        per_user_limit,
    }))
}
