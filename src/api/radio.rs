//! Internet radio stations.
//!
//! Tagged `radio` rather than `user-data`, and that is a decision rather than a
//! spelling: the `user-data` tag selects the routes that carry the operation-id
//! protocol and appear in the desktop's journal, and a station does not. It is
//! per-account configuration, beside the API tokens and the scrobbling links.
//! See `src/services/radio.rs` for the same decision stated where the table
//! is.

use super::*;

/// A station to store, whole.
///
/// The same body creates and replaces, because the protocol's
/// `updateInternetRadioStation` requires the name and the address every time.
/// Omitting `homepage_url` therefore clears it: a station is replaced, not
/// patched, and the two surfaces answer identically.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RadioStationRequest {
    /// Trimmed. A name that is empty, or only spaces, is refused.
    pub name: String,
    /// Absolute `http` or `https`, with a host. Anything else is refused — the
    /// server never fetches this, and a column of unvalidated addresses would
    /// be waiting for the first feature that did.
    pub stream_url: String,
    #[serde(default)]
    pub homepage_url: Option<String>,
    /// An image for the station, absolute `http` or `https`, which the server
    /// fetches once into the artwork store and hands out as `artwork_hash`.
    ///
    /// The one field that is not replaced by omission: absent, the image stays
    /// as it is; `null` or blank, it is cleared. A client written before the
    /// field existed — or a Subsonic one, which has no such parameter — must
    /// not take a station's image away by renaming it.
    #[serde(default, deserialize_with = "crate::services::present")]
    #[schema(value_type = Option<String>, nullable)]
    pub image_url: Option<Option<String>>,
}

impl RadioStationRequest {
    fn image(&self) -> crate::services::StationImage<'_> {
        match &self.image_url {
            None => crate::services::StationImage::Keep,
            Some(value) => crate::services::StationImage::Set(value.as_deref()),
        }
    }
}

#[utoipa::path(get, path = "/api/v2/radio-stations", tag = "radio", responses((status = 200, body = [crate::services::RadioStationItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_radio_stations(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::RadioStationItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .radio_stations(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(get, path = "/api/v2/radio-stations/{station_id}", tag = "radio", params(("station_id" = Uuid, Path)), responses((status = 200, body = crate::services::RadioStationItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_radio_station(
    State(state): State<AppState>,
    Path(station_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::RadioStationItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .radio_station(user.id, station_id)
        .await
        .map(Json)
        .map_err(service_error)
}

/// A station the account did not have, so `POST` and a 201. The 409 is the
/// per-account ceiling: the request is well formed and the list is full.
#[utoipa::path(post, path = "/api/v2/radio-stations", tag = "radio", request_body = RadioStationRequest, responses((status = 201, body = crate::services::RadioStationItem), (status = 401, body = ErrorResponse), (status = 409, description = "The account already holds as many stations as it may", body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_radio_station(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RadioStationRequest>,
) -> Result<(StatusCode, Json<crate::services::RadioStationItem>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let station = state
        .services
        .create_radio_station(
            user.id,
            &request.name,
            &request.stream_url,
            request.homepage_url.as_deref(),
            request.image_url.as_ref().and_then(Option::as_deref),
        )
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(station)))
}

/// `PUT` and not `PATCH`: the body carries every field, and sending it twice
/// leaves the same station. A station the account cannot see answers 404, like
/// one that does not exist.
#[utoipa::path(put, path = "/api/v2/radio-stations/{station_id}", tag = "radio", params(("station_id" = Uuid, Path)), request_body = RadioStationRequest, responses((status = 200, body = crate::services::RadioStationItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn replace_radio_station(
    State(state): State<AppState>,
    Path(station_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<RadioStationRequest>,
) -> Result<Json<crate::services::RadioStationItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .update_radio_station(
            user.id,
            station_id,
            &request.name,
            &request.stream_url,
            request.homepage_url.as_deref(),
            request.image(),
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(delete, path = "/api/v2/radio-stations/{station_id}", tag = "radio", params(("station_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_radio_station(
    State(state): State<AppState>,
    Path(station_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    state
        .services
        .delete_radio_station(user.id, station_id)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
