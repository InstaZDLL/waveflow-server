//! Where an account links a destination, and answers for what could not be sent.
//!
//! RFC-010 decision 9: a destination's base URL belongs to the deployment and
//! lives in `src/config.rs`, but an *authorisation* belongs to a person, so it
//! is posed here rather than by an environment variable. The same gestures exist
//! on the CLI for an operator preparing a server without a browser — both call
//! the same `DomainServices` methods, so the two surfaces cannot drift.
//!
//! **Self-scoped, not administrative.** A dedicated Subsonic password is set by
//! an administrator *for* someone; a scrobbling link is the account's own, like
//! a bookmark or a share. `Access::Write` for the mutations, `Access::Read` for
//! the two listings.

use super::*;

/// The credential an account presents at a destination.
#[derive(Debug, Deserialize, ToSchema)]
pub struct LinkScrobbleRequest {
    /// Never read back. Like every other secret here it is replaced rather than
    /// shown, and the service seals it under the instance key.
    pub secret: String,
}

/// What a deliberate retry produced.
#[derive(Debug, Serialize, ToSchema)]
pub struct RetriedScrobbleResponse {
    /// The new entry. The ambiguous one it came from stays where it was — it
    /// must, because erasing it would falsify the only trace explaining why a
    /// duplicate exists.
    pub id: Uuid,
}

/// The destination named in a path, or a 422.
///
/// Parsed through `FromStr` rather than by deriving `Deserialize` on the enum:
/// the wire name, the database `CHECK`, `as_str` and `FromStr` are one fact held
/// together by a test, and a fifth spelling is exactly the drift it exists to
/// stop.
fn provider(raw: &str) -> Result<crate::services::ScrobbleProvider, ApiError> {
    crate::services::ScrobbleProvider::from_str(raw).map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/scrobble-links", tag = "user-data", responses((status = 200, body = [crate::services::ScrobbleLinkState]), (status = 401, body = ErrorResponse)))]
pub async fn list_scrobble_links(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::ScrobbleLinkState>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .scrobble_links(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Linking again replaces the authorisation rather than adding one.
///
/// `PUT` for that reason: the destination names the resource, and presenting a
/// second token leaves one link, not two. What it does *not* do is reuse the
/// old row — decision 4 makes the row the generation, so listens already queued
/// stay attached to the authorisation they were queued under and can never be
/// submitted to whichever profile is linked now.
#[utoipa::path(put, path = "/api/v2/scrobble-links/{provider}", tag = "user-data", params(("provider" = String, Path)), request_body = LinkScrobbleRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn link_scrobble(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(request): Json<LinkScrobbleRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .link_scrobble(user.id, provider(&name)?, &request.secret)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unlinking a destination that is not linked succeeds: the caller asked for
/// this account to have no authorisation there, and it has none.
#[utoipa::path(delete, path = "/api/v2/scrobble-links/{provider}", tag = "user-data", params(("provider" = String, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn unlink_scrobble(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .unlink_scrobble(user.id, provider(&name)?)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every listen whose fate nobody knows, and which is still asking.
///
/// Under `scrobble-queue` rather than beside `{provider}`, so that a
/// destination's name and this word can never be read as the same segment.
#[utoipa::path(get, path = "/api/v2/scrobble-queue/uncertain", tag = "user-data", responses((status = 200, body = [crate::services::UncertainScrobble]), (status = 401, body = ErrorResponse)))]
pub async fn list_uncertain_scrobbles(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::UncertainScrobble>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .uncertain_scrobbles(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// The first of the two gestures decision 13 grants: the person prefers the gap.
#[utoipa::path(delete, path = "/api/v2/scrobble-queue/uncertain/{entry_id}", tag = "user-data", params(("entry_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn discard_uncertain_scrobble(
    State(state): State<AppState>,
    Path(entry_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .discard_uncertain_scrobble(user.id, entry_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The second, and the only path in this design that can make a duplicate on
/// purpose: the person accepts that the destination may already hold this
/// listen. Granted once per entry, which the schema holds as well as the
/// service — a second call is a 409, not a second copy.
#[utoipa::path(post, path = "/api/v2/scrobble-queue/uncertain/{entry_id}/retry", tag = "user-data", params(("entry_id" = Uuid, Path)), responses((status = 200, body = RetriedScrobbleResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse)))]
pub async fn retry_uncertain_scrobble(
    State(state): State<AppState>,
    Path(entry_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<RetriedScrobbleResponse>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .retry_uncertain_scrobble(user.id, entry_id)
        .await
        .map(|id| Json(RetriedScrobbleResponse { id }))
        .map_err(service_error)
}
