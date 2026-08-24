//! Login, refresh and logout, for native clients and for the browser.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub device_name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[utoipa::path(
    post,
    path = "/api/v2/auth/login",
    tag = "authentication",
    request_body = LoginRequest,
    responses(
        (status = 200, body = crate::authentication::AuthTokens),
        (status = 401, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<crate::authentication::AuthTokens>, ApiError> {
    state
        .auth
        .login(&request.username, &request.password, &request.device_name)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

#[utoipa::path(
    post,
    path = "/api/v2/auth/refresh",
    tag = "authentication",
    request_body = RefreshRequest,
    responses(
        (status = 200, body = crate::authentication::AuthTokens),
        (status = 401, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub async fn refresh(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Result<Json<crate::authentication::AuthTokens>, ApiError> {
    state
        .auth
        .refresh(&request.refresh_token)
        .await
        .map(Json)
        .map_err(ApiError::from)
}

#[utoipa::path(
    post,
    path = "/api/v2/auth/logout",
    tag = "authentication",
    responses(
        (status = 204),
        (status = 401, body = ErrorResponse),
        (status = 503, body = ErrorResponse)
    )
)]
pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let access_token = bearer_token(&headers).ok_or(ApiError::Unauthorized)?;
    state
        .auth
        .logout(access_token)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Browser sessions keep only the short-lived access token in JavaScript. The
/// rotating refresh token is an HttpOnly, same-site cookie and is therefore
/// never exposed to the embedded SPA.
#[utoipa::path(
    post,
    path = "/api/v2/web/auth/login",
    tag = "authentication",
    request_body = LoginRequest,
    responses(
        (status = 200, body = WebAuthResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse)
    )
)]
pub async fn web_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    validate_web_origin(&state, &headers)?;
    let tokens = state
        .auth
        .login(&request.username, &request.password, &request.device_name)
        .await
        .map_err(ApiError::from)?;
    web_auth_response(&state, &headers, tokens)
}

#[utoipa::path(
    post,
    path = "/api/v2/web/auth/refresh",
    tag = "authentication",
    responses(
        (status = 200, body = WebAuthResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse)
    )
)]
pub async fn web_refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_web_request(&state, &headers)?;
    let refresh_token = cookie_value(&headers, WEB_REFRESH_COOKIE).ok_or(ApiError::Unauthorized)?;
    let tokens = state
        .auth
        .refresh(refresh_token)
        .await
        .map_err(ApiError::from)?;
    web_auth_response(&state, &headers, tokens)
}

#[utoipa::path(
    post,
    path = "/api/v2/web/auth/logout",
    tag = "authentication",
    responses(
        (status = 204),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse)
    )
)]
pub async fn web_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    validate_web_request(&state, &headers)?;
    let result = match cookie_value(&headers, WEB_REFRESH_COOKIE) {
        Some(refresh_token) => state.auth.revoke_refresh(refresh_token).await,
        None => Err(AuthError::InvalidRefreshToken),
    };
    let mut response = match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => ApiError::from(error).into_response(),
    };
    let secure = secure_cookies(&state);
    append_cookie(
        &mut response,
        expired_cookie(WEB_REFRESH_COOKIE, true, secure),
    )?;
    append_cookie(
        &mut response,
        expired_cookie(WEB_CSRF_COOKIE, false, secure),
    )?;
    Ok(response)
}
