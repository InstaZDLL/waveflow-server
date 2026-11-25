//! First-run setup.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;
use crate::auth::setup_token::Admission;

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupStatusResponse {
    pub required: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupRequest {
    pub username: String,
    pub password: String,
    /// The one-time setup token (RFC-019): the content of
    /// `<data dir>/setup-token`, or the value of `WAVEFLOW_SETUP_TOKEN` when
    /// the operator set one. Optional in the schema so that a request without
    /// it is answered `403 setup_token_rejected` rather than a parse error.
    #[serde(default)]
    pub setup_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SetupResponse {
    pub user_id: Uuid,
    /// The first library, made of `WAVEFLOW_MUSIC_DIR` and already queued for
    /// its first scan, or `null` when the server made none: the setting is
    /// unset, its folder is missing, or a library already existed.
    pub library: Option<CreateLibraryResponse>,
}

/// The name the first library is given. The administrator renames it at will.
const FIRST_LIBRARY_NAME: &str = "Music";

#[utoipa::path(get, path = "/api/v2/setup", tag = "authentication", responses((status = 200, body = SetupStatusResponse)))]
pub async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    let required = state.db.setup_required().await.map_err(db_error)?;
    Ok(Json(SetupStatusResponse { required }))
}

#[utoipa::path(post, path = "/api/v2/setup", tag = "authentication", params(("Origin" = String, Header, description = "Required browser origin")), request_body = SetupRequest, responses((status = 201, body = SetupResponse), (status = 403, description = "Origin header missing or rejected (`forbidden`), or the setup token missing or wrong (`setup_token_rejected`)", body = ErrorResponse), (status = 409, description = "An account already exists", body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn setup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SetupRequest>,
) -> Result<(StatusCode, Json<SetupResponse>), ApiError> {
    validate_web_origin(&state, &headers)?;
    // Before the username and the password are looked at: a caller without the
    // token learns nothing from this route, not even the password rules.
    match state.setup.admit(request.setup_token.as_deref()) {
        Admission::Admitted => {}
        Admission::Rejected => return Err(ApiError::SetupTokenRejected),
        Admission::Closed => return Err(ApiError::Conflict),
    }
    let outcome = state
        .services
        .bootstrap_admin(&request.username, &request.password)
        .await;
    // Closed on success, and on the conflict that says an account was made
    // some other way meanwhile — the command line, beside a running server.
    // A refused username or password leaves it open, for the next attempt.
    if matches!(
        outcome,
        Ok(_) | Err(crate::services::ServiceError::Conflict)
    ) {
        state.setup.close();
    }
    let user_id = outcome.map_err(service_error)?;
    let library = first_library(&state, user_id).await;
    Ok((
        StatusCode::CREATED,
        Json(SetupResponse { user_id, library }),
    ))
}

/// Registers `WAVEFLOW_MUSIC_DIR` as the new administrator's first library.
///
/// The music is mounted, the account exists, and nothing else stands between
/// them: without this, a fresh server opened on empty pages, refused uploads
/// for want of a library, and showed nothing of the folders created under
/// the mount until someone found the administration form and typed the path
/// the compose file had already named.
///
/// Best effort. The account is made by now, so no failure here may turn the
/// setup into an error: the administrator would be told setup failed for an
/// account that exists, and the token that could retry it is spent. A missing
/// folder or a refused registration is logged, and the library is added by
/// hand as before.
async fn first_library(state: &AppState, owner: Uuid) -> Option<CreateLibraryResponse> {
    let path = state.music_dir.as_deref()?;
    match state.db.all_libraries().await {
        Ok(libraries) if libraries.is_empty() => {}
        Ok(_) => return None,
        Err(error) => {
            tracing::warn!(error = %error, "the first library was not created: the libraries could not be listed");
            return None;
        }
    }
    let registered = super::libraries::register_library(
        state,
        owner,
        FIRST_LIBRARY_NAME.to_owned(),
        path,
        crate::storage::database::LibraryVisibility::Private,
    )
    .await;
    match registered {
        Ok(created) => {
            tracing::info!(
                library_id = %created.library_id,
                path = %path.display(),
                "the first library was created from WAVEFLOW_MUSIC_DIR"
            );
            Some(created)
        }
        Err(ApiError::Validation) => {
            tracing::warn!(
                path = %path.display(),
                "WAVEFLOW_MUSIC_DIR names no directory this server can register; add a library from Administration"
            );
            None
        }
        // The register logs its own failure; this says what it cost.
        Err(_) => {
            tracing::warn!(
                path = %path.display(),
                "the first library could not be created from WAVEFLOW_MUSIC_DIR; check Administration"
            );
            None
        }
    }
}
