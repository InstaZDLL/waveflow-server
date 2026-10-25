//! Podcasts (RFC-025).
//!
//! Tagged `podcasts`: one list per instance, so nothing here is user data in
//! the synchronization sense. Every write answers 404 to an account that may not
//! manage podcasts — the same blur `Forbidden` gets everywhere — and the
//! overview says in advance whether this account may (`can_manage`).

use super::*;

use crate::services::PodcastFailure;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePodcastChannelRequest {
    /// The feed's address: absolute https, or http where the operator allows
    /// it; no user information.
    pub url: String,
}

/// A feed that could not be subscribed to, and why, as a word from a fixed
/// list: `destination_refused`, `address_refused`, `too_many_redirects`,
/// `too_large`, `unreached`, `http_<status>` or `invalid_feed`.
#[derive(Debug, Serialize, ToSchema)]
pub struct FeedErrorResponse {
    /// Always `feed_rejected`.
    pub code: &'static str,
    pub message: &'static str,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct NewestQuery {
    pub count: Option<i64>,
}

#[utoipa::path(get, path = "/api/v2/podcasts", tag = "podcasts", responses((status = 200, body = crate::services::PodcastOverview), (status = 401, body = ErrorResponse)))]
pub async fn podcast_overview(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<crate::services::PodcastOverview>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .podcast_overview(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Subscribes: the feed is fetched and read before anything is stored, so a
/// typo or an unreachable host leaves nothing behind.
#[utoipa::path(post, path = "/api/v2/podcasts", tag = "podcasts", request_body = CreatePodcastChannelRequest, responses((status = 201, body = crate::services::PodcastChannelItem), (status = 401, body = ErrorResponse), (status = 404, description = "Podcasts are off, or this account may not manage them", body = ErrorResponse), (status = 409, description = "Already subscribed, or the instance holds as many channels as it may", body = ErrorResponse), (status = 422, description = "Not an address a feed may be fetched from, or the feed could not be read", body = FeedErrorResponse)))]
pub async fn create_podcast_channel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreatePodcastChannelRequest>,
) -> Result<(StatusCode, Json<crate::services::PodcastChannelItem>), PodcastRejection> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    match state
        .services
        .create_podcast_channel(user.id, &request.url)
        .await
    {
        Ok(channel) => Ok((StatusCode::CREATED, Json(channel))),
        Err(PodcastFailure::Service(error)) => Err(service_error(error).into()),
        Err(PodcastFailure::Feed(reason)) => Err(PodcastRejection::Feed(reason)),
    }
}

/// What subscribing refuses with: the ordinary API error, or a feed that could
/// not be fetched or read.
#[derive(Debug)]
pub enum PodcastRejection {
    Api(ApiError),
    Feed(String),
}

impl From<ApiError> for PodcastRejection {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for PodcastRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Feed(reason) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(FeedErrorResponse {
                    code: "feed_rejected",
                    message: "The feed could not be fetched or read",
                    reason,
                }),
            )
                .into_response(),
        }
    }
}

/// Fetches every channel again, in the background.
#[utoipa::path(post, path = "/api/v2/podcasts/refresh", tag = "podcasts", responses((status = 202), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn refresh_podcasts(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .refresh_podcasts(user.id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(get, path = "/api/v2/podcasts/{channel_id}", tag = "podcasts", params(("channel_id" = Uuid, Path)), responses((status = 200, description = "The channel and its episodes, newest first, with this account's position in each", body = crate::services::PodcastChannelItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_podcast_channel(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::PodcastChannelItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .podcast_channel(user.id, channel_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(delete, path = "/api/v2/podcasts/{channel_id}", tag = "podcasts", params(("channel_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_podcast_channel(
    State(state): State<AppState>,
    Path(channel_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .delete_podcast_channel(user.id, channel_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/podcast-episodes/newest", tag = "podcasts", params(("count" = Option<i64>, Query, description = "20 by default, at most 500")), responses((status = 200, body = [crate::services::PodcastEpisodeItem]), (status = 401, body = ErrorResponse)))]
pub async fn newest_podcast_episodes(
    State(state): State<AppState>,
    Query(query): Query<NewestQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::PodcastEpisodeItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .newest_podcast_episodes(
            user.id,
            query
                .count
                .unwrap_or(crate::services::NEWEST_EPISODES_DEFAULT),
        )
        .await
        .map(Json)
        .map_err(service_error)
}

/// One episode. A stored one plays through
/// `POST /api/v2/tracks/{episode_id}/stream-ticket`, and its position is kept
/// with `PUT /api/v2/bookmarks/{episode_id}`, as a track's is.
#[utoipa::path(get, path = "/api/v2/podcast-episodes/{episode_id}", tag = "podcasts", params(("episode_id" = Uuid, Path)), responses((status = 200, body = crate::services::PodcastEpisodeItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_podcast_episode(
    State(state): State<AppState>,
    Path(episode_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::PodcastEpisodeItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .podcast_episode(user.id, episode_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Queues the episode's download. Its status moves to `downloading`, then to
/// `completed` or `error`.
#[utoipa::path(post, path = "/api/v2/podcast-episodes/{episode_id}/download", tag = "podcasts", params(("episode_id" = Uuid, Path)), responses((status = 202), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn download_podcast_episode(
    State(state): State<AppState>,
    Path(episode_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .download_podcast_episode(user.id, episode_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::ACCEPTED)
}

/// Removes the episode's file and marks it deleted, so a refresh does not
/// bring it back.
#[utoipa::path(delete, path = "/api/v2/podcast-episodes/{episode_id}", tag = "podcasts", params(("episode_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_podcast_episode(
    State(state): State<AppState>,
    Path(episode_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .delete_podcast_episode(user.id, episode_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
