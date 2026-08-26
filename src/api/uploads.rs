//! Offering the server a file, and being told whether it wants it.
//!
//! Only the negotiation lives here for now. It is the half that decides, and
//! every refusal it can hand back costs the client nothing but the question.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct NegotiateUploadsRequest {
    /// The files being offered. Bounded by `WAVEFLOW_UPLOAD_BATCH_LIMIT`: a
    /// batch exists so five thousand candidates are not five thousand round
    /// trips, not so they become one unbounded body.
    pub offers: Vec<crate::services::UploadOffer>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct NegotiateUploadsResponse {
    /// One verdict per offer, each carrying the hash it answers rather than
    /// relying on position.
    pub verdicts: Vec<crate::services::UploadVerdict>,
}

#[utoipa::path(
    post,
    path = "/api/v2/libraries/{library_id}/uploads",
    tag = "catalog",
    params(("library_id" = Uuid, Path)),
    request_body = NegotiateUploadsRequest,
    responses(
        (status = 200, body = NegotiateUploadsResponse),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn negotiate_uploads(
    State(state): State<AppState>,
    Path(library_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<NegotiateUploadsRequest>,
) -> Result<Json<NegotiateUploadsResponse>, ApiError> {
    // `Write` rather than `Read`: this opens sessions and reserves quota, which
    // is a mutation whatever the question sounds like.
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .negotiate_uploads(user.id, library_id, request.offers)
        .await
        .map(|verdicts| Json(NegotiateUploadsResponse { verdicts }))
        .map_err(service_error)
}

#[utoipa::path(
    get,
    path = "/api/v2/uploads/{session_id}",
    tag = "catalog",
    params(("session_id" = Uuid, Path)),
    responses(
        (status = 200, body = crate::services::UploadSessionState),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse)
    )
)]
pub async fn upload_session(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::UploadSessionState>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .upload_session_state(user.id, session_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(
    put,
    path = "/api/v2/uploads/{session_id}/chunks/{index}",
    tag = "catalog",
    params(("session_id" = Uuid, Path), ("index" = i64, Path)),
    request_body(content = Vec<u8>, content_type = "application/octet-stream"),
    responses(
        (status = 200, body = crate::services::UploadSessionState),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn upload_chunk(
    State(state): State<AppState>,
    Path((session_id, index)): Path<(Uuid, i64)>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Json<crate::services::UploadSessionState>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .receive_chunk(user.id, session_id, index, &body)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(
    post,
    path = "/api/v2/uploads/{session_id}/commit",
    tag = "catalog",
    params(("session_id" = Uuid, Path)),
    responses(
        (status = 201, body = crate::services::CommittedUpload),
        (status = 401, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 422, body = ErrorResponse)
    )
)]
pub async fn commit_upload(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<crate::services::CommittedUpload>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let committed = state
        .services
        .commit_upload(user.id, session_id)
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(committed)))
}
