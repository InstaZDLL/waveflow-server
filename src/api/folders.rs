//! Filed folders and placements (RFC-012 decisions 2 and 3).
//!
//! Handlers orchestrate HTTP only; the rules live in `DomainServices`. Reads
//! are open to any member, writes to owners and managers, and everyone else
//! gets the same `404` a missing folder would.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateFolderRequest {
    pub name: String,
    /// `null` or absent for the root.
    #[serde(default)]
    pub parent_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PlacementRequest {
    /// Up to 500 tracks of the library.
    pub track_ids: Vec<Uuid>,
    /// The folder to file them in, or `null` to unfile them.
    pub folder_id: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct FolderTracksQuery {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// The body a placement may carry: the most tracks one request files, each
/// as a quoted UUID and its comma, and room for the envelope.
pub(super) fn placement_body_limit() -> usize {
    crate::services::MAX_PLACEMENTS * 48 + 1024
}

/// The whole filed tree of a library, flat, with the count of unfiled tracks.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/folders", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 200, body = crate::services::FiledTree), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn list_folders(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::FiledTree>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .filed_tree(user.id, library_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Creates a folder at the root or under `parent_id`. `409` for a sibling of
/// the same name, `422` past the depth or folder cap.
#[utoipa::path(post, path = "/api/v2/libraries/{library_id}/folders", tag = "catalog", params(("library_id" = Uuid, Path)), request_body = CreateFolderRequest, responses((status = 201, body = crate::services::FiledFolder), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_folder(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<CreateFolderRequest>,
) -> Result<(StatusCode, Json<crate::services::FiledFolder>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .create_folder(user.id, library_id, &request.name, request.parent_id)
        .await
        .map(|folder| (StatusCode::CREATED, Json(folder)))
        .map_err(service_error)
}

/// Renames a folder, moves it (`parent_id: null` for the root), or both.
#[utoipa::path(patch, path = "/api/v2/folders/{folder_id}", tag = "catalog", params(("folder_id" = Uuid, Path)), request_body = crate::services::FolderPatch, responses((status = 200, body = crate::services::FiledFolder), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_folder(
    State(state): State<AppState>,
    Path(folder_id): Path<Uuid>,
    headers: HeaderMap,
    Json(patch): Json<crate::services::FolderPatch>,
) -> Result<Json<crate::services::FiledFolder>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .update_folder(user.id, folder_id, patch)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Deletes a folder and its sub-folders; their tracks become unfiled.
#[utoipa::path(delete, path = "/api/v2/folders/{folder_id}", tag = "catalog", params(("folder_id" = Uuid, Path)), responses((status = 200, body = crate::services::FolderRemoved), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_folder(
    State(state): State<AppState>,
    Path(folder_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::FolderRemoved>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .delete_folder(user.id, folder_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// The tracks filed directly in a folder, paged; `unfiled` in place of an id
/// for the library's unfiled tracks.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/folders/{folder}/tracks", tag = "catalog", params(("library_id" = Uuid, Path), ("folder" = String, Path, description = "a folder id, or unfiled"), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = [crate::services::SongItem]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn folder_tracks(
    State(state): State<AppState>,
    Path((library_id, folder)): Path<(Uuid, String)>,
    Query(query): Query<FolderTracksQuery>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::SongItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let folder = match folder.as_str() {
        "unfiled" => None,
        id => Some(Uuid::parse_str(id).map_err(|_| ApiError::NotFound)?),
    };
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .folder_tracks(user.id, library_id, folder, page)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Files up to 500 tracks into one folder, or unfiles them with
/// `folder_id: null`. All or nothing.
#[utoipa::path(put, path = "/api/v2/libraries/{library_id}/placements", tag = "catalog", params(("library_id" = Uuid, Path)), request_body = PlacementRequest, responses((status = 200, body = crate::services::Placed), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 413, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn place_tracks(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<PlacementRequest>,
) -> Result<Json<crate::services::Placed>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .place_tracks(user.id, library_id, &request.track_ids, request.folder_id)
        .await
        .map(Json)
        .map_err(service_error)
}
