//! HTTP middleware.
//!
//! Currently just the dev-only `X-User-Id` header auth shim used by
//! the `/api/v1/*` data routes. Phase 1.d replaces this with JWT
//! verification against the Better Auth JWKS endpoint — the shape of
//! `UserId` stays the same so handlers don't need to change, only the
//! extractor source.

use axum::{
    extract::Request,
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};

/// Owning user id, attached to every authenticated request via
/// `request.extensions().get::<UserId>()`. Wrapping the raw `i64`
/// keeps a handler from accidentally reading a different `i64`
/// extension (e.g. a future `ProfileId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserId(pub i64);

/// Header name used by the dev shim. **NOT** present in 1.d once
/// Better Auth lands — the JWT bearer header replaces it. Keeping
/// the constant here so a future grep for "x-user-id" surfaces every
/// touch point in one shot.
pub const X_USER_ID_HEADER: &str = "x-user-id";

/// Middleware: pull `X-User-Id` off the request, parse it as an
/// `i64`, and attach a [`UserId`] extension. Reject 401 if missing or
/// malformed so a handler can never run against an unscoped query by
/// accident.
///
/// The middleware does NOT verify the id exists in the `users` table
/// — that's enforced at the storage layer by `profile.user_id`'s FK,
/// which rejects an insert with a dangling user. The trade-off keeps
/// the middleware free of a DB round-trip per request; the loud
/// failure mode (insert) is acceptable in a dev shim.
pub async fn require_user_id(mut request: Request, next: Next) -> Result<Response, StatusCode> {
    let value: &HeaderValue = request
        .headers()
        .get(X_USER_ID_HEADER)
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let user_id: i64 = value
        .to_str()
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    // `0` and negative ids are reserved (0 is the desktop sentinel,
    // negatives are conventionally invalid) — reject so a stray
    // header from a confused dev client can't sneak through.
    if user_id <= 0 {
        return Err(StatusCode::UNAUTHORIZED);
    }

    request.extensions_mut().insert(UserId(user_id));
    Ok(next.run(request).await)
}
