//! Authorization Code + PKCE for native clients.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct AuthorizeRequest {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    #[serde(default = "default_challenge_method")]
    pub code_challenge_method: String,
    pub state: Option<String>,
    /// Name recorded for the device this grant will create a session for.
    pub device_name: String,
}

pub(super) fn default_challenge_method() -> String {
    "S256".into()
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthorizeResponse {
    /// Where the consent screen must send the user agent.
    pub redirect_to: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TokenRequest {
    pub code: String,
    pub code_verifier: String,
    pub client_id: String,
    pub redirect_uri: String,
}

#[utoipa::path(post, path = "/api/v2/oauth/authorize", tag = "authentication", request_body = AuthorizeRequest, responses((status = 200, body = AuthorizeResponse), (status = 401, body = ErrorResponse), (status = 422, body = ErrorResponse)))]
pub async fn oauth_authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AuthorizeRequest>,
) -> Result<Json<AuthorizeResponse>, ApiError> {
    // The browser session is the proof of identity; the consent screen is a
    // route of the embedded client, so this is a JSON call rather than a form.
    //
    // A write, because pairing a device is a mutation on the account and
    // nothing more. What used to make this Unrestricted — that the session it
    // minted carried the account's whole authority whatever asked for it — is
    // gone: the caller's scopes are recorded on the grant just below and the
    // redeemed session is issued under them, so this cannot widen a credential.
    let user = authenticated(&state, &headers, Access::Write).await?;
    let redirect_to = state
        .services
        .authorize_native_client(
            user.id,
            crate::services::AuthorizationRequest {
                client_id: &request.client_id,
                redirect_uri: &request.redirect_uri,
                code_challenge: &request.code_challenge,
                code_challenge_method: &request.code_challenge_method,
                device_name: &request.device_name,
                state: request.state.as_deref(),
                scopes: &user.scopes,
            },
        )
        .await
        .map_err(service_error)?;
    Ok(Json(AuthorizeResponse { redirect_to }))
}

#[utoipa::path(post, path = "/api/v2/oauth/token", tag = "authentication", request_body = TokenRequest, responses((status = 200, body = crate::authentication::AuthTokens), (status = 401, body = ErrorResponse), (status = 503, body = ErrorResponse)))]
pub async fn oauth_token(
    State(state): State<AppState>,
    Json(request): Json<TokenRequest>,
) -> Result<Json<crate::authentication::AuthTokens>, ApiError> {
    // Mounted without authentication by design: the code plus the verifier are
    // the credential. Every rejection below is the same 401 so a caller cannot
    // learn whether a code existed, expired, or was already spent.
    let now = crate::authentication::now_ms();
    let grant = state
        .db
        .redeem_authorization(&crate::security::token_hash(&request.code), now)
        .await
        .map_err(db_error)?
        .ok_or(ApiError::Unauthorized)?;
    if grant.client_id != request.client_id.trim()
        || grant.redirect_uri != request.redirect_uri
        || crate::oauth::verify_challenge(&grant.code_challenge, &request.code_verifier).is_err()
    {
        return Err(ApiError::Unauthorized);
    }
    state
        .auth
        .issue_session_for_account(grant.user_id, &grant.device_name, &grant.scopes)
        .await
        .map(Json)
        .map_err(|error| match error {
            crate::authentication::AuthError::Unavailable => ApiError::Unavailable,
            _ => ApiError::Unauthorized,
        })
}
