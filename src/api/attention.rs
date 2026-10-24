//! "Needs attention" and credit decisions (RFC-015).
//!
//! Handlers orchestrate HTTP only; the rules live in `DomainServices`. Every
//! route here is for a library's owners and managers, and everyone else gets
//! the same `404` a library they are not in would.

use super::*;

#[derive(Debug, Deserialize)]
pub struct AttentionTracksQuery {
    pub category: String,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreditDecisionRequest {
    /// An artist the candidates listed: the credit it makes is what is decided.
    pub artist_id: Uuid,
    pub decision: crate::services::CreditDecisionKind,
}

/// How many items each list of the page holds, in display order.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/attention", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 200, body = [crate::services::AttentionCount]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn attention_summary(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::AttentionCount>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .attention_summary(user.id, library_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// The joined credits waiting for a decision, each with its fragments.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/attention/credits", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 200, body = [crate::services::CreditCandidate]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn attention_credits(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::CreditCandidate>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .credit_candidates(user.id, library_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// One page of the tracks in one category. `422` for a category that is not
/// a track category.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/attention/tracks", tag = "catalog", params(("library_id" = Uuid, Path), ("category" = String, Query), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::AttentionTrack]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn attention_tracks(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    Query(query): Query<AttentionTracksQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::AttentionTrack>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let category = crate::services::AttentionCategory::parse_track_category(&query.category)
        .ok_or(ApiError::Validation)?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .attention_tracks(user.id, library_id, category, page)
        .await
        .map(Json)
        .map_err(service_error)
}

/// The library's decisions, newest first.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/credit-decisions", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 200, body = [crate::services::CreditDecisionRecord]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn list_credit_decisions(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::CreditDecisionRecord>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .credit_decisions(user.id, library_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Splits a joined credit or keeps it whole, and applies that at once. `409`
/// when the credit already has a decision, `422` for an artist that is not a
/// candidate.
#[utoipa::path(post, path = "/api/v2/libraries/{library_id}/credit-decisions", tag = "catalog", params(("library_id" = Uuid, Path)), request_body = CreditDecisionRequest, responses((status = 201, body = crate::services::CreditDecisionRecord), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_credit_decision(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<CreditDecisionRequest>,
) -> Result<(StatusCode, Json<crate::services::CreditDecisionRecord>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .decide_credit(user.id, library_id, request.artist_id, request.decision)
        .await
        .map(|decision| (StatusCode::CREATED, Json(decision)))
        .map_err(service_error)
}

/// Removes a decision and restores what the library holds without it.
#[utoipa::path(delete, path = "/api/v2/libraries/{library_id}/credit-decisions/{decision_id}", tag = "catalog", params(("library_id" = Uuid, Path), ("decision_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_credit_decision(
    State(state): State<AppState>,
    Path((library_id, decision_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .remove_credit_decision(user.id, library_id, decision_id)
        .await
        .map(|()| StatusCode::NO_CONTENT)
        .map_err(service_error)
}
