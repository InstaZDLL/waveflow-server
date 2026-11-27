//! The browser session: cookies, CSRF and origin checks.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;
use crate::http::ingress::Ingress;

#[derive(Debug, Serialize, ToSchema)]
pub struct WebAuthResponse {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub user: crate::auth::authentication::AuthUser,
    pub device_id: Uuid,
}

/// Where the browser session's cookies live, and whether they are `Secure`.
///
/// On the main listener: at the root, `Secure` when `WAVEFLOW_PUBLIC_URL` is
/// https, as always. On Home Assistant's ingress (RFC-030 decision 5): under
/// the prefix, so the refresh cookie is sent back at all and neither is sent
/// with every request to Home Assistant itself; and `Secure` as the browser
/// reached Home Assistant, which Core names in `X-Forwarded-Proto`. The public
/// URL says nothing about that address, and a `Secure` cookie on a Home
/// Assistant served over plain http would never be stored.
pub(crate) struct CookieScope {
    root: String,
    secure: bool,
}

impl CookieScope {
    pub(crate) fn of(state: &AppState, ingress: &Ingress, headers: &HeaderMap) -> Self {
        let secure = if ingress.0.is_some() {
            headers
                .get("x-forwarded-proto")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
        } else {
            secure_cookies(state)
        };
        Self {
            root: ingress.root().to_owned(),
            secure,
        }
    }

    fn refresh_path(&self) -> String {
        format!("{}/api/v2/web/auth", self.root)
    }

    fn csrf_path(&self) -> String {
        format!("{}/", self.root)
    }

    fn secure(&self) -> &'static str {
        if self.secure {
            "; Secure"
        } else {
            ""
        }
    }
}

pub(super) fn web_auth_response(
    state: &AppState,
    scope: &CookieScope,
    tokens: crate::auth::authentication::AuthTokens,
) -> Result<Response, ApiError> {
    let csrf_token = crate::auth::security::generate_token("wfcsrf_");
    let refresh_cookie = format!(
        "{WEB_REFRESH_COOKIE}={}; Path={}; HttpOnly; SameSite=Strict; Max-Age={}{}",
        tokens.refresh_token,
        scope.refresh_path(),
        state.refresh_token_ttl.as_secs(),
        scope.secure()
    );
    let csrf_cookie = format!(
        "{WEB_CSRF_COOKIE}={csrf_token}; Path={}; SameSite=Strict; Max-Age={}{}",
        scope.csrf_path(),
        state.refresh_token_ttl.as_secs(),
        scope.secure()
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

pub(super) fn expired_cookie(name: &str, http_only: bool, scope: &CookieScope) -> String {
    format!(
        "{name}=; Path={}; SameSite=Strict; Max-Age=0{}{}",
        if http_only {
            scope.refresh_path()
        } else {
            scope.csrf_path()
        },
        if http_only { "; HttpOnly" } else { "" },
        scope.secure()
    )
}

pub(super) fn secure_cookies(state: &AppState) -> bool {
    public_url_is_https(state.public_url.as_deref())
}

pub fn public_url_is_https(public_url: Option<&str>) -> bool {
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

pub(super) fn validate_web_request(
    state: &AppState,
    ingress: &Ingress,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
    validate_web_origin(state, ingress, headers)?;
    let cookie = cookie_value(headers, WEB_CSRF_COOKIE).ok_or(ApiError::Forbidden)?;
    let supplied = headers
        .get(WEB_CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .ok_or(ApiError::Forbidden)?;
    if !crate::auth::security::constant_time_bytes_eq(cookie.as_bytes(), supplied.as_bytes()) {
        return Err(ApiError::Forbidden);
    }
    Ok(())
}

/// Whether a browser request comes from the server's own page.
///
/// On Home Assistant's ingress (RFC-030 decision 6), `WAVEFLOW_PUBLIC_URL` is
/// not the address the browser is on, so it is not asked: `Origin` is compared
/// with `Host`, which Home Assistant passes through from the browser, as on a
/// server with no public URL.
pub(super) fn validate_web_origin(
    state: &AppState,
    ingress: &Ingress,
    headers: &HeaderMap,
) -> Result<(), ApiError> {
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
    if let Some(public_url) = state.public_url.as_deref().filter(|_| ingress.0.is_none()) {
        let expected = url::Url::parse(public_url).map_err(|_| ApiError::Unavailable)?;
        return if parsed.origin() == expected.origin() {
            Ok(())
        } else {
            Err(ApiError::OriginRejected)
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
        Err(ApiError::OriginRejected)
    }
}

#[cfg(test)]
mod tests;
