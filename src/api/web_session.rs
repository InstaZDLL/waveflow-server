//! The browser session: cookies, CSRF and origin checks.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Serialize, ToSchema)]
pub struct WebAuthResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub user: crate::authentication::AuthUser,
    pub device_id: Uuid,
}

pub(super) fn web_auth_response(
    state: &AppState,
    _headers: &HeaderMap,
    tokens: crate::authentication::AuthTokens,
) -> Result<Response, ApiError> {
    let csrf_token = crate::security::generate_token("wfcsrf_");
    let secure = secure_cookies(state);
    let refresh_cookie = format!(
        "{WEB_REFRESH_COOKIE}={}; Path=/api/v2/web/auth; HttpOnly; SameSite=Strict; Max-Age={}{}",
        tokens.refresh_token,
        state.refresh_token_ttl.as_secs(),
        if secure { "; Secure" } else { "" }
    );
    let csrf_cookie = format!(
        "{WEB_CSRF_COOKIE}={csrf_token}; Path=/; SameSite=Strict; Max-Age={}{}",
        state.refresh_token_ttl.as_secs(),
        if secure { "; Secure" } else { "" }
    );
    let body = WebAuthResponse {
        access_token: tokens.access_token,
        token_type: tokens.token_type,
        expires_in: tokens.expires_in,
        user: tokens.user,
        device_id: tokens.device_id,
    };
    let mut response = Json(body).into_response();
    append_cookie(&mut response, refresh_cookie)?;
    append_cookie(&mut response, csrf_cookie)?;
    Ok(response)
}

pub(super) fn append_cookie(response: &mut Response, value: String) -> Result<(), ApiError> {
    let value = HeaderValue::from_str(&value).map_err(|_| ApiError::Unavailable)?;
    response.headers_mut().append(header::SET_COOKIE, value);
    Ok(())
}

pub(super) fn expired_cookie(name: &str, http_only: bool, secure: bool) -> String {
    format!(
        "{name}=; Path={}; SameSite=Strict; Max-Age=0{}{}",
        if http_only { "/api/v2/web/auth" } else { "/" },
        if http_only { "; HttpOnly" } else { "" },
        if secure { "; Secure" } else { "" }
    )
}

pub(super) fn secure_cookies(state: &AppState) -> bool {
    public_url_is_https(state.public_url.as_deref())
}

pub(super) fn public_url_is_https(public_url: Option<&str>) -> bool {
    public_url
        .and_then(|url| url::Url::parse(url).ok())
        .is_some_and(|url| url.scheme() == "https")
}

pub(super) fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(key, value)| (key == name && !value.is_empty()).then_some(value))
}

pub(super) fn validate_web_request(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    validate_web_origin(state, headers)?;
    let cookie = cookie_value(headers, WEB_CSRF_COOKIE).ok_or(ApiError::Forbidden)?;
    let supplied = headers
        .get(WEB_CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Forbidden)?;
    if !crate::security::constant_time_bytes_eq(cookie.as_bytes(), supplied.as_bytes()) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

pub(super) fn validate_web_origin(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Forbidden)?;
    let parsed = url::Url::parse(origin).map_err(|_| ApiError::Forbidden)?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(ApiError::Forbidden);
    }
    if let Some(public_url) = state.public_url.as_deref() {
        let expected = url::Url::parse(public_url).map_err(|_| ApiError::Unavailable)?;
        return if parsed.origin() == expected.origin() {
            Ok(())
        } else {
            Err(ApiError::Forbidden)
        };
    }
    let authority = &parsed[url::Position::BeforeHost..url::Position::AfterPort];
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Forbidden)?;
    if authority.eq_ignore_ascii_case(host) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}
