//! Listening statistics, mixes and similar songs — RFC-024 decisions 6 to 8.
//!
//! Parameter adapters over `DomainServices`; nothing here reads the database.

use super::*;

#[derive(Debug, Deserialize)]
pub struct StatsParams {
    pub period: Option<String>,
    pub utc_offset_minutes: Option<i32>,
    pub limit: Option<i64>,
}

/// One account's listening over a period (RFC-024 decision 6).
///
/// `period` is `7d`, `30d` (the default), `365d` or `all`;
/// `utc_offset_minutes` makes a day the listener's day; `limit` bounds each
/// top list, 1 to 50. Only the caller's own listens, on tracks they can still
/// reach, are counted.
#[utoipa::path(get, path = "/api/v2/stats", tag = "user-data", params(("period" = Option<String>, Query, description = "7d, 30d, 365d or all"), ("utc_offset_minutes" = Option<i32>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = crate::services::ListeningStats), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn get_listening_stats(
    State(state): State<AppState>,
    Query(params): Query<StatsParams>,
    headers: HeaderMap,
) -> Result<Json<crate::services::ListeningStats>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let query = crate::services::StatsQuery::new(
        params.period.as_deref(),
        params.utc_offset_minutes,
        params.limit,
    )
    .map_err(service_error)?;
    state
        .services
        .listening_stats(user.id, query)
        .await
        .map(Json)
        .map_err(service_error)
}

#[derive(Debug, Deserialize)]
pub struct MixParams {
    pub library_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub utc_offset_minutes: Option<i32>,
}

impl MixParams {
    fn query(&self) -> Result<crate::services::MixQuery, ApiError> {
        crate::services::MixQuery::new(
            self.library_id.into_iter().collect(),
            self.limit,
            self.utc_offset_minutes,
        )
        .map_err(service_error)
    }
}

/// The mixes the caller has today (RFC-024 decision 7). A mix with no track
/// is left out.
#[utoipa::path(get, path = "/api/v2/mixes", tag = "catalog", params(("library_id" = Option<Uuid>, Query), ("limit" = Option<i64>, Query, description = "1 to 100, default 50"), ("utc_offset_minutes" = Option<i32>, Query)), responses((status = 200, body = [crate::services::MixSummary]), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_mixes(
    State(state): State<AppState>,
    Query(params): Query<MixParams>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::MixSummary>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let query = params.query()?;
    state
        .services
        .list_mixes(user.id, &query)
        .await
        .map(Json)
        .map_err(service_error)
}

/// One mix and its tracks. An unknown mix, a genre the caller cannot see and
/// a mix with no track all answer `404`.
#[utoipa::path(get, path = "/api/v2/mixes/{mix_id}", tag = "catalog", params(("mix_id" = String, Path), ("library_id" = Option<Uuid>, Query), ("limit" = Option<i64>, Query, description = "1 to 100, default 50"), ("utc_offset_minutes" = Option<i32>, Query)), responses((status = 200, body = crate::services::Mix), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn get_mix(
    State(state): State<AppState>,
    Path(mix_id): Path<String>,
    Query(params): Query<MixParams>,
    headers: HeaderMap,
) -> Result<Json<crate::services::Mix>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let query = params.query()?;
    state
        .services
        .mix(user.id, &mix_id, &query)
        .await
        .map(Json)
        .map_err(service_error)
}

#[derive(Debug, Deserialize)]
pub struct SimilarSongParams {
    pub limit: Option<i64>,
}

/// The native form of `getSimilarSongs` (RFC-024 decision 8): a draw from
/// the artist behind `id` — an artist, an album or a track — and from its
/// neighbours, a provider's when one names any, the catalogue's otherwise.
#[utoipa::path(get, path = "/api/v2/similar-songs/{id}", tag = "catalog", params(("id" = Uuid, Path, description = "an artist, album or track id"), ("limit" = Option<i64>, Query, description = "0 to 500, default 50")), responses((status = 200, body = [crate::services::SongItem]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn list_similar_songs(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(params): Query<SimilarSongParams>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SongItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .similar_songs(user.id, id, params.limit.unwrap_or(50))
        .await
        .map(Json)
        .map_err(service_error)
}
