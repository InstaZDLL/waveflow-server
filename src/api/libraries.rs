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
    let output = async_stream::stream! {
        yield Ok(Event::default().event("snapshot").json_data(initial).expect("scan snapshot serializes"));
        if let Some(ref mut receiver) = receiver {
            loop {
                match receiver.recv().await {
                    Ok(progress) => yield Ok(Event::default().event("progress").json_data(progress).expect("scan progress serializes")),
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
