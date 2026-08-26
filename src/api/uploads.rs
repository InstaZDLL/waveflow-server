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
