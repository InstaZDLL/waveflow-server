//! Playlists.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePlaylistRequest {
    pub name: String,
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdatePlaylistRequest {
    pub name: Option<String>,
    pub comment: Option<String>,
    pub public: Option<bool>,
    /// Track ids appended to the end, applied after `remove_indexes`.
    #[serde(default)]
    pub add: Vec<Uuid>,
    /// Zero-based positions removed before `add` is applied.
    #[serde(default)]
    pub remove_indexes: Vec<usize>,
    /// Optional fields to blank out, by name. Currently `comment`.
    ///
    /// Omitting a field leaves it untouched, so clearing needs its own verb:
    /// naming it here is the only way to distinguish "unchanged" from "empty",
    /// and it cannot fire by accident on a client that simply omits the field.
    #[serde(default)]
    pub clear: Vec<String>,
}

#[utoipa::path(get, path = "/api/v2/playlists", tag = "user-data", responses((status = 200, body = [crate::services::PlaylistItem]), (status = 401, body = ErrorResponse)))]
pub async fn list_playlists(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<crate::services::PlaylistItem>>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .playlists(user.id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(post, path = "/api/v2/playlists", tag = "user-data", request_body = CreatePlaylistRequest, responses((status = 201, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn create_playlist(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<CreatePlaylistRequest>,
) -> Result<(StatusCode, Json<crate::services::PlaylistItem>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let playlist = state
        .services
        .create_playlist_with_context(user.id, &request.name, &request.track_ids, context)
        .await
        .map_err(service_error)?;
    Ok((StatusCode::CREATED, Json(playlist)))
}

#[utoipa::path(get, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn get_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    state
        .services
        .playlist(user.id, playlist_id)
        .await
        .map(Json)
        .map_err(service_error)
}

#[derive(Debug, Deserialize)]
pub struct PlaylistImportQuery {
    /// The name to give the playlist. Falls back to the file's own
    /// `#PLAYLIST` directive, and the request is refused when neither says
    /// anything — a playlist with no name is not a thing this server stores.
    pub name: Option<String>,
    /// Narrows resolution to one library. Absent means every library the
    /// account can reach.
    pub library_id: Option<Uuid>,
    /// Resolves and reports without creating anything, so a client can show
    /// what an import would lose before it happens.
    #[serde(default)]
    pub dry_run: bool,
}

/// Imports an `.m3u`/`.m3u8` file as a playlist of the calling account.
///
/// **The body is the file, not JSON.** The encoding of a playlist file is part
/// of the problem — `.m3u` is frequently not UTF-8 — so the bytes have to
/// arrive as bytes. A JSON envelope would have forced the client to decode
/// them first, which is the one decision it cannot make correctly.
#[utoipa::path(post, path = "/api/v2/playlists/import", tag = "user-data", params(("name" = Option<String>, Query), ("library_id" = Option<Uuid>, Query), ("dry_run" = Option<bool>, Query)), request_body(content = String, description = "The playlist file", content_type = "audio/x-mpegurl"), responses((status = 201, body = crate::services::PlaylistImport), (status = 200, description = "A dry run, which created nothing", body = crate::services::PlaylistImport), (status = 401, body = ErrorResponse), (status = 413, description = "Larger than a playlist may be"), (status = 422, body = ErrorResponse)))]
pub async fn import_playlist(
    State(state): State<AppState>,
    Query(query): Query<PlaylistImportQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<(StatusCode, Json<crate::services::PlaylistImport>), ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    let libraries = query.library_id.into_iter().collect::<Vec<_>>();
    let report = state
        .services
        .import_playlist(
            user.id,
            &libraries,
            query.name.as_deref(),
            &body,
            query.dry_run,
            context,
        )
        .await
        .map_err(service_error)?;
    // 200 for a dry run, because nothing was created and a 201 would say
    // otherwise.
    let status = match report.playlist {
        Some(_) => StatusCode::CREATED,
        None => StatusCode::OK,
    };
    Ok((status, Json(report)))
}

/// Renders a playlist back as an extended `.m3u8`.
///
/// **The only `/api/v2` route that does not answer JSON**, and the reason is
/// that the resource *is* a file: wrapping it in an envelope would make every
/// consumer unwrap it before writing it to disk. Errors still answer JSON,
/// like everywhere else.
#[utoipa::path(get, path = "/api/v2/playlists/{playlist_id}/export", tag = "user-data", params(("playlist_id" = Uuid, Path)), responses((status = 200, description = "An extended .m3u8", content_type = "audio/x-mpegurl", body = String), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn export_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<([(header::HeaderName, String); 2], String), ApiError> {
    let user = authenticated(&state, &headers, Access::Read).await?;
    // The name comes back with the file. Reading it through the playlist read
    // model instead cost a full hydration of every track and its relations —
    // twice over, since the service did the same — to label a download.
    let (name, file) = state
        .services
        .export_playlist(user.id, playlist_id)
        .await
        .map_err(service_error)?;
    Ok((
        [
            (
                header::CONTENT_TYPE,
                "audio/x-mpegurl; charset=utf-8".to_owned(),
            ),
            (header::CONTENT_DISPOSITION, disposition(&name)),
        ],
        file,
    ))
}

/// `Content-Disposition` for a name chosen by the account that owns it.
///
/// A header value carries no newline and no quote, and a playlist name is free
/// text, so neither spelling of the name is passed through: the quoted form is
/// filtered down to an ASCII subset, and the full name travels percent-encoded
/// in `filename*` as RFC 6266 asks. A name with nothing ASCII left in it falls
/// back to `playlist`, so the quoted form is never empty.
fn disposition(name: &str) -> String {
    let ascii = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
        .collect::<String>()
        .trim()
        .to_owned();
    let ascii = match ascii.is_empty() {
        true => "playlist".to_owned(),
        false => ascii,
    };
    let encoded = percent_encoding::utf8_percent_encode(name, percent_encoding::NON_ALPHANUMERIC);
    format!("attachment; filename=\"{ascii}.m3u8\"; filename*=UTF-8''{encoded}.m3u8")
}

/// An unknown name is refused rather than ignored: a client asking to clear
/// `expiresAt` instead of `expires_at` would otherwise be told it succeeded
/// while the field stayed put.
pub(super) fn playlist_clear(names: &[String]) -> Result<crate::services::PlaylistClear, ApiError> {
    let mut clear = crate::services::PlaylistClear::default();
    for name in names {
        match name.as_str() {
            "comment" => clear.comment = true,
            _ => return Err(ApiError::Validation),
        }
    }
    Ok(clear)
}

#[utoipa::path(patch, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), request_body = UpdatePlaylistRequest, responses((status = 200, body = crate::services::PlaylistItem), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse), (status = 409, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn update_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<UpdatePlaylistRequest>,
) -> Result<Json<crate::services::PlaylistItem>, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .update_playlist_with_context(
            user.id,
            playlist_id,
            request.name.as_deref(),
            request.comment.as_deref(),
            request.public,
            &request.add,
            &request.remove_indexes,
            playlist_clear(&request.clear)?,
            context,
        )
        .await
        .map(Json)
        .map_err(service_error)
}

#[utoipa::path(delete, path = "/api/v2/playlists/{playlist_id}", tag = "user-data", params(("playlist_id" = Uuid, Path)), responses((status = 204), (status = 401, body = ErrorResponse), (status = 404, body = ErrorResponse)))]
pub async fn delete_playlist(
    State(state): State<AppState>,
    Path(playlist_id): Path<Uuid>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = authenticated(&state, &headers, Access::Write).await?;
    let context = mutation_context(&state, &headers, user.id).await?;
    state
        .services
        .delete_playlist_with_context(user.id, playlist_id, context)
        .await
        .map_err(service_error)?;
    Ok(StatusCode::NO_CONTENT)
}
