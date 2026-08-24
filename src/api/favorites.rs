//! Favourites and ratings.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RatingRequest {
    /// 1 to 5 stars; 0 clears the rating.
    pub rating: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StarredEntry {
    pub entity_type: String,
    pub entity_id: Uuid,
    pub starred_at: i64,
}

#[utoipa::path(get, path = "/api/v2/favorites", tag = "user-data", responses((status = 200, body = [StarredEntry]), (status = 401, body = ErrorResponse)))]
pub async fn list_favorites(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<StarredEntry>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let entries = state
        .services
        .starred_ids(user.id)
        .await
        .map_err(service_error)?
        .into_iter()
        .map(|(entity_type, entity_id, starred_at)| StarredEntry {
            entity_type,
            entity_id,
            starred_at,
        })
        .collect();
    Ok(Json(entries))
}

#[utoipa::path(put, path = "/api/v2/favorites/{entity_type}/{entity_id}", tag = "user-data", params(("entity_type" = String, Path, description = "track, album or artist"), ("entity_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn add_favorite(
    State(state): State<AppState>,
    Path((entity_type, entity_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    set_favorite(state, headers, &entity_type, entity_id, true).await
}

#[utoipa::path(delete, path = "/api/v2/favorites/{entity_type}/{entity_id}", tag = "user-data", params(("entity_type" = String, Path, description = "track, album or artist"), ("entity_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn remove_favorite(
    State(state): State<AppState>,
    Path((entity_type, entity_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    set_favorite(state, headers, &entity_type, entity_id, false).await
}

pub(super) async fn set_favorite(
    state: AppState,
    headers: HeaderMap,
    entity_type: &str,
    entity_id: Uuid,
    starred: bool,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .set_star_with_context(user.id, entity_type, entity_id, starred, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(put, path = "/api/v2/ratings/{entity_type}/{entity_id}", tag = "user-data", params(("entity_type" = String, Path, description = "track, album or artist"), ("entity_id" = Uuid, Path)), request_body = RatingRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_rating(
    State(state): State<AppState>,
    Path((entity_type, entity_id)): Path<(String, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<RatingRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .set_rating_with_context(user.id, &entity_type, entity_id, request.rating, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/v2/ratings", tag = "user-data", responses((status = 200, body = [crate::services::RatingItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_ratings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::RatingItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .ratings(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}
