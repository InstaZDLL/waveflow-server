//! Smart playlists (RFC-025).
//!
//! Tagged `smart-playlists` rather than `user-data`: the `user-data` tag selects
//! the routes that carry the operation-id protocol and appear in the desktop's
//! journal, and a smart playlist does neither — like a radio station, it is
//! held by the account and read by every surface, never synchronized.

use super::*;

use crate::services::{RuleError, SmartRules};

/// A refused rule set, and where in it.
///
/// Its own body rather than the generic validation error: the editor puts the
/// message beside the control that produced it, so it needs the path, and the
/// reason is one of a fixed set of sentences — nothing the client sent is
/// echoed back.
#[derive(Debug, Serialize, ToSchema)]
pub struct RuleErrorResponse {
    /// Always `invalid_rules`.
    pub code: &'static str,
    pub message: &'static str,
    /// Where in `rules`: `match`, `conditions[2].operator`, `limit`…
    pub at: String,
}

/// A smart playlist to create.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SmartPlaylistRequest {
    pub name: String,
    #[serde(default)]
    pub comment: Option<String>,
    #[serde(default)]
    pub public: bool,
    /// `{"match": "all" | "any", "conditions": [{"field", "operator",
    /// "value"}], "sort", "order", "limit"}`, validated against the
    /// allowlist RFC-025 decision 2 lists.
    #[schema(value_type = Object)]
    pub rules: serde_json::Value,
}

/// What may change. Omitted fields are left as they are; `rules`, when sent,
/// replaces the whole rule set; an empty `comment` clears it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SmartPlaylistPatch {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
    #[schema(value_type = Option<Object>)]
    pub rules: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SmartPreviewRequest {
    #[schema(value_type = Object)]
    pub rules: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct SmartPlaylistListQuery {
    /// Adds every other account's public smart playlists after this account's
    /// own, as for ordinary playlists.
    #[serde(default)]
    pub include_public: bool,
}

/// What a smart playlist route refuses with: the ordinary API error, or a rule
/// set outside the allowlist.
#[derive(Debug)]
pub enum SmartPlaylistRejection {
    Api(ApiError),
    Rules(RuleError),
}

impl From<ApiError> for SmartPlaylistRejection {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl IntoResponse for SmartPlaylistRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Api(error) => error.into_response(),
            Self::Rules(RuleError { at, reason }) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(RuleErrorResponse {
                    code: "invalid_rules",
                    message: reason,
                    at,
                }),
            )
                .into_response(),
        }
    }
}

fn parsed_rules(value: &serde_json::Value) -> Result<SmartRules, SmartPlaylistRejection> {
    SmartRules::parse(value).map_err(SmartPlaylistRejection::Rules)
}

fn api(error: crate::services::ServiceError) -> SmartPlaylistRejection {
    SmartPlaylistRejection::Api(service_error(error))
}

#[utoipa::path(get, path = "/api/v2/smart-playlists", tag = "smart-playlists", params(("include_public" = Option<bool>, Query, description = "Also list other accounts' public smart playlists")), responses((status = 200, description = "Without their tracks: counts and durations only, over what this account can play", body = [crate::services::SmartPlaylistItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_smart_playlists(
    State(state): State<AppState>,
    Query(query): Query<SmartPlaylistListQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SmartPlaylistItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .smart_playlists(user.id, query.include_public)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/smart-playlists", tag = "smart-playlists", request_body = SmartPlaylistRequest, responses((status = 201, body = crate::services::SmartPlaylistItem), (status = 401, body = ErrorResponse), (status = 409, description = "The account already holds as many smart playlists as it may", body = ErrorResponse), (status = 422, description = "The rules are outside the allowlist, or the name is empty", body = RuleErrorResponse)))]
pub async fn create_smart_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SmartPlaylistRequest>,
) -> Result<(StatusCode, Json<crate::services::SmartPlaylistItem>), SmartPlaylistRejection> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let rules = parsed_rules(&request.rules)?;
    let playlist = state
        .services
        .create_smart_playlist(
            user.id,
            &request.name,
            request.comment.as_deref(),
            request.public,
            &rules,
        )
        .await
        .map_err(api)?;
    Ok((StatusCode::CREATED, Json(playlist)))
}

/// Evaluates rules without storing them, for this account: what the editor
/// shows while the rules are written.
#[utoipa::path(post, path = "/api/v2/smart-playlists/preview", tag = "smart-playlists", request_body = SmartPreviewRequest, responses((status = 200, body = crate::services::SmartPlaylistPreview), (status = 401, body = ErrorResponse), (status = 422, body = RuleErrorResponse)))]
pub async fn preview_smart_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SmartPreviewRequest>,
) -> Result<Json<crate::services::SmartPlaylistPreview>, SmartPlaylistRejection> {
    // A read: nothing is stored, so a read-only credential may preview.
    let user = authenticated(&state, &headers, Access::Read).await?;
    let rules = parsed_rules(&request.rules)?;
    state
        .services
        .preview_smart_playlist(user.id, &rules)
        .await
        .map(Json)
        .map_err(api)
}

#[utoipa::path(get, path = "/api/v2/smart-playlists/{playlist_id}", tag = "smart-playlists", params(("playlist_id" = Uuid, Path)), responses((status = 200, description = "Evaluated now, its tracks read through this account's libraries", body = crate::services::SmartPlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_smart_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::SmartPlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .smart_playlist(user.id, playlist_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(patch, path = "/api/v2/smart-playlists/{playlist_id}", tag = "smart-playlists", params(("playlist_id" = Uuid, Path)), request_body = SmartPlaylistPatch, responses((status = 200, body = crate::services::SmartPlaylistItem), (status = 401, body = ErrorResponse), (status = 404, description = "Missing, or another account's", body = ErrorResponse), (status = 422, body = RuleErrorResponse)))]
pub async fn update_smart_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<SmartPlaylistPatch>,
) -> Result<Json<crate::services::SmartPlaylistItem>, SmartPlaylistRejection> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let rules = request.rules.as_ref().map(parsed_rules).transpose()?;
    let clear_comment = request
        .comment
        .as_deref()
        .is_some_and(|comment| comment.trim().is_empty());
    state
        .services
        .update_smart_playlist(
            user.id,
            playlist_id,
            request.name.as_deref(),
            request.comment.as_deref(),
            clear_comment,
            request.public,
            rules.as_ref(),
        )
        .await
        .map(Json)
        .map_err(api)
}

#[utoipa::path(delete, path = "/api/v2/smart-playlists/{playlist_id}", tag = "smart-playlists", params(("playlist_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_smart_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .delete_smart_playlist(user.id, playlist_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
