//! Libraries, their members and their scans.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Serialize, ToSchema)]
pub struct ScanQueuedResponse {
    pub scan_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLibraryRequest {
    pub name: String,
    pub path: String,
    pub visibility: crate::database::LibraryVisibility,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CreateLibraryResponse {
    pub library_id: Uuid,
    pub scan_id: Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetLibraryMemberRequest {
    pub role: crate::database::LibraryRole,
}

#[utoipa::path(post, path = "/api/v2/libraries/{library_id}/scans", tag = "catalog", params(("library_id" = Uuid, Path)), responses((status = 202, body = ScanQueuedResponse), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn start_scan(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<ScanQueuedResponse>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let scan_id = state
        .services
        .start_library_scan(user.id, library_id)
        .await
        .map_err(service_error)?;
    Ok((StatusCode::ACCEPTED, Json(ScanQueuedResponse { scan_id })))
}

#[utoipa::path(get, path = "/api/v2/libraries", tag = "catalog", responses((status = 200, body = [crate::catalog::LibraryAccess]), (status = 401, body = ErrorResponse)))]
pub async fn list_libraries(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::catalog::LibraryAccess>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .db
        .libraries_for_user(user.id)
        .await
        .map(Json)
        .map_err(db_error)
}

#[utoipa::path(post, path = "/api/v2/libraries", tag = "administration", request_body = CreateLibraryRequest, responses((status = 201, body = CreateLibraryResponse), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_library(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreateLibraryRequest>,
) -> Result<(StatusCode, Json<CreateLibraryResponse>), ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    let path = std::path::PathBuf::from(&request.path);
    let metadata = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| ApiError::Validation)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() || request.name.trim().is_empty() {
        return Err(ApiError::Validation);
    }
    let canonical = tokio::fs::canonicalize(&path)
        .await
        .map_err(|_| ApiError::Validation)?;
    let library_id = state
        .db
        .create_library(
            actor.id,
            &request.name,
            &canonical,
            request.visibility,
            crate::authentication::now_ms(),
        )
        .await
        .map_err(db_error)?;
    state.scanner.libraries_changed();
    let scan_id = state
        .scanner
        .trigger(
            crate::catalog::LibraryRecord {
                id: library_id,
                name: request.name,
                root_path: canonical,
            },
            Some(actor.id),
            "library_added",
        )
        .await
        .map_err(|error| {
            tracing::error!(error = %error, library_id = %library_id, "initial scan queue failed");
            ApiError::Unavailable
        })?;
    Ok((
        StatusCode::CREATED,
        Json(CreateLibraryResponse {
            library_id,
            scan_id,
        }),
    ))
}

#[utoipa::path(put, path = "/api/v2/libraries/{library_id}/members/{user_id}", tag = "administration", params(("library_id" = Uuid, Path), ("user_id" = Uuid, Path)), request_body = SetLibraryMemberRequest, responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn set_library_member(
    State(state): State<AppState>,
    Path((library_id, user_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
    Json(request): Json<SetLibraryMemberRequest>,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    if request.role == crate::database::LibraryRole::Owner
        || state
            .db
            .account_by_id(user_id)
            .await
            .map_err(db_error)?
            .is_none()
        || !state
            .db
            .all_libraries()
            .await
            .map_err(db_error)?
            .iter()
            .any(|library| library.id == library_id)
    {
        return Err(ApiError::Validation);
    }
    state
        .db
        .add_library_member(
            actor.id,
            library_id,
            user_id,
            request.role,
            crate::authentication::now_ms(),
        )
        .await
        .map_err(db_error)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/v2/libraries/{library_id}/members/{user_id}", tag = "administration", params(("library_id" = Uuid, Path), ("user_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn remove_library_member(
    State(state): State<AppState>,
    Path((library_id, user_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    if state
        .db
        .remove_library_member(
            actor.id,
            library_id,
            user_id,
            crate::authentication::now_ms(),
        )
        .await
        .map_err(db_error)?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

#[utoipa::path(get, path = "/api/v2/scans/{scan_id}", tag = "catalog", params(("scan_id" = Uuid, Path)), responses((status = 200, body = crate::catalog::ScanJobRecord), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn scan_status(
    State(state): State<AppState>,
    Path(scan_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::catalog::ScanJobRecord>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .db
        .scan_job_for_user(user.id, scan_id)
        .await
        .map_err(db_error)?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(get, path = "/api/v2/scans/{scan_id}/events", tag = "catalog", params(("scan_id" = Uuid, Path)), responses((status = 200, description = "Server-sent scan progress events", content_type = "text/event-stream"), (status = 404, body = ErrorResponse)))]
pub async fn scan_events(
    State(state): State<AppState>,
    Path(scan_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let initial = state
        .db
        .scan_job_for_user(user.id, scan_id)
        .await
        .map_err(db_error)?
        .ok_or(ApiError::NotFound)?;
    let mut receiver = state.scanner.subscribe(scan_id);
    // Both events carry a `ScanJobRecord`, so a watcher replaces what it holds
    // and never has to tell the two frames apart. `progress` used to carry the
    // scanner's own `ScanProgress`, which names three of the same fields
    // differently — see `ScanProgress::record`.
    let output = async_stream::stream! {
        yield Ok(Event::default().event("snapshot").json_data(initial).expect("scan snapshot serializes"));
        if let Some(ref mut receiver) = receiver {
            loop {
                match receiver.recv().await {
                    Ok(progress) => yield Ok(Event::default().event("progress").json_data(progress.record()).expect("scan progress serializes")),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    };
    Ok(Sse::new(output).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

/// One page of a library's change feed.
///
/// The counterpart of `/api/v2/sync/changes` for catalogue state. Its cursor is
/// a different sequence and advances for different reasons, so it is a separate
/// route with a separate cursor rather than a widening of the other — a rescan
/// must not move a client's position in its own user journal.
/// How far a device has read one library's feed.
///
/// A body rather than a header for the device, exactly like `/api/v2/sync/ack`:
/// the acknowledgement *is* about that device, so it is the request rather than
/// a note attached to it. The two are refused identically — 422 — for an
/// unknown or revoked device, a library the account cannot see, and a cursor
/// beyond what the feed has written. One answer for all three, because telling
/// them apart would say whether a library exists to somebody who may not know.
#[utoipa::path(
    put,
    path = "/api/v2/libraries/{library_id}/events/ack",
    tag = "libraries",
    params(("library_id" = Uuid, Path)),
    request_body = LibraryEventAckRequest,
    responses(
        (status = 204),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn library_events_ack(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<LibraryEventAckRequest>,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let acknowledged = state
        .services
        .acknowledge_library_events(user.id, library_id, request.device_id, request.cursor)
        .await
        .map_err(service_error)?;
    if !acknowledged {
        return Err(ApiError::Validation);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Who may see a library, and in what standing.
///
/// `PUT` and `DELETE` on `/libraries/{library_id}/members/{user_id}` have
/// existed since M4 with nothing to read them back, so an interface could grant
/// and revoke without showing who already had access.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/members", tag = "administration", params(("library_id" = Uuid, Path)), responses((status = 200, body = [crate::services::LibraryMember]), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn list_library_members(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::LibraryMember>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .library_members(user.id, library_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// What a device says it has read.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct LibraryEventAckRequest {
    pub device_id: Uuid,
    /// The highest cursor this device has processed. Never lowered by the
    /// server: a client that acknowledges an older cursor after a newer one has
    /// raced its own two requests.
    pub cursor: i64,
}

#[utoipa::path(
    get,
    path = "/api/v2/libraries/{library_id}/events",
    tag = "catalog",
    params(
        ("library_id" = Uuid, Path),
        ("after" = Option<i64>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = crate::services::LibraryEventPage),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn library_events(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Query(query): Query<SyncQuery>,
) -> Result<Json<crate::services::LibraryEventPage>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .library_changes(
            user.id,
            library_id,
            query.after.unwrap_or(0),
            query.limit.unwrap_or(crate::sync::DEFAULT_SYNC_LIMIT),
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[derive(Debug, Deserialize)]
pub struct DiskTreeQuery {
    /// The folder to list, relative to the library; absent or empty for its
    /// root, `.waveflow-managed` for the files received through uploads.
    pub path: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// One level of a library's disk tree (RFC-012 decision 1): the sub-folders
/// of `path` with their track counts, then the tracks directly in it, paged as
/// one list. Derived from the indexed paths, read-only, and open to every
/// member of the library.
#[utoipa::path(
    get,
    path = "/api/v2/libraries/{library_id}/tree",
    tag = "catalog",
    params(
        ("library_id" = Uuid, Path),
        ("path" = Option<String>, Query),
        ("offset" = Option<i64>, Query),
        ("limit" = Option<i64>, Query)
    ),
    responses(
        (status = 200, body = crate::services::DiskListing),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn library_tree(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    Query(query): Query<DiskTreeQuery>,
    headers: HeaderMap,
) -> Result<Json<crate::services::DiskListing>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .disk_tree(
            user.id,
            library_id,
            query.path.as_deref().unwrap_or(""),
            page,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

/// What deleting a library must carry: its current name, exactly. A body that
/// lacks it, or names anything else, deletes nothing and answers `422`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteLibraryRequest {
    #[serde(default)]
    pub confirm: String,
}

/// Renames a library, moves its root or changes whether its walk follows
/// links (RFC-021 decision 5). A name is the owner's or an administrator's to
/// change; the path and `follow_links` are an administrator's, on a credential
/// that carries administrative authority. A new root or links setting queues a
/// scan, whose id is answered. `409` while the library is being scanned, or
/// when another library already has that root.
#[utoipa::path(patch, path = "/api/v2/libraries/{library_id}", tag = "administration", params(("library_id" = Uuid, Path)), request_body = crate::services::LibraryUpdate, responses((status = 200, body = crate::services::LibraryUpdated), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_library(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<crate::services::LibraryUpdate>,
) -> Result<Json<crate::services::LibraryUpdated>, ApiError> {
    let access = if request.is_administrative() {
        Access::Admin
    } else {
        Access::Write
    };
    let actor = authenticated(&state, &headers, access).await?;
    state
        .services
        .update_library(actor.id, library_id, request)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Deletes a library from the catalogue, and nothing from the disk (RFC-021
/// decision 5): every track, album and artist, with everything that cascades
/// from them for every member. The body must carry `confirm`, equal to the
/// library's current name. `409` while the library is being scanned.
#[utoipa::path(delete, path = "/api/v2/libraries/{library_id}", tag = "administration", params(("library_id" = Uuid, Path)), request_body = DeleteLibraryRequest, responses((status = 200, body = crate::services::LibraryDeleted), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn delete_library(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<DeleteLibraryRequest>,
) -> Result<Json<crate::services::LibraryDeleted>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .delete_library(actor.id, library_id, &request.confirm)
        .await
        .map(Json)
        .map_err(service_error)
}

/// A library as an administrator manages it, its root included.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/settings", tag = "administration", params(("library_id" = Uuid, Path)), responses((status = 200, body = crate::services::LibrarySettings), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn library_settings(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::LibrarySettings>, ApiError> {
    let actor = authenticated(&state, &headers, Access::Admin).await?;
    state
        .services
        .library_settings(actor.id, library_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[derive(Debug, Deserialize)]
pub struct UnavailableTracksQuery {
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

/// The tracks a library's scans can no longer find, oldest first, with what
/// purging each would cost (RFC-021 decision 4). For the library's owners and
/// managers, and administrators; anyone else gets the `404` a library they are
/// not in would.
#[utoipa::path(get, path = "/api/v2/libraries/{library_id}/unavailable-tracks", tag = "catalog", params(("library_id" = Uuid, Path), ("offset" = Option<i64>, Query), ("limit" = Option<i64>, Query)), responses((status = 200, body = crate::services::UnavailableTracks), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn unavailable_tracks(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    Query(query): Query<UnavailableTracksQuery>,
    headers: HeaderMap,
) -> Result<Json<crate::services::UnavailableTracks>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    let page =
        crate::services::BrowsePage::new(query.offset, query.limit).map_err(service_error)?;
    state
        .services
        .unavailable_tracks(user.id, library_id, page)
        .await
        .map(Json)
        .map_err(service_error)
}

/// Purges missing tracks: those named in `track_ids`, or every one with
/// `"all": true`, and never neither (`422`). Only tracks still unavailable are
/// removed. Not reversible: see RFC-021 decision 4 for what is lost. `409`
/// while the library is being scanned.
#[utoipa::path(post, path = "/api/v2/libraries/{library_id}/unavailable-tracks/purge", tag = "catalog", params(("library_id" = Uuid, Path)), request_body = crate::services::PurgeRequest, responses((status = 200, body = crate::services::PurgeOutcome), (status = 401, body = ErrorResponse), (status = 403, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn purge_unavailable_tracks(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<crate::services::PurgeRequest>,
) -> Result<Json<crate::services::PurgeOutcome>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .purge_unavailable_tracks(user.id, library_id, request)
        .await
        .map(Json)
        .map_err(service_error)
}
