//! HTTP middleware.
//!
//! Single auth path: every `/api/v1/*` request must carry a
//! Bearer JWT signed by the upstream Better Auth instance. The dev
//! `X-User-Id` header shim retired in Phase 1.d.2 along with
//! `POST /api/v1/users`; lazy auto-provisioning via the JWT path is
//! the only way to land a row in `users`.

use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode},
    middleware::Next,
    response::Response,
};

use crate::{
    auth::{AuthError, JwtVerifier},
    db, AppState,
};

/// Owning user id, attached to every authenticated request via
/// `request.extensions().get::<UserId>()`. Wrapping the raw `i64`
/// keeps a handler from accidentally reading a different `i64`
/// extension (e.g. a future `ProfileId`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserId(pub i64);

/// Bearer-JWT auth middleware for `/api/v1/*`. Per-request flow:
///
/// 1. **No `Authorization` header** → 401. The server is configured
///    (boot would have failed otherwise), so the request, not the
///    server, is at fault.
/// 2. **Verify the token** via [`AppState::jwt_verifier`].
///    - Bad signature / claims / `kid` → 401 (no body detail, the
///      reason lands in `tracing::warn!`).
///    - JWKS fetch failure → 503. Routes around the instance while
///      the upstream is unreachable, lets the load balancer pick a
///      healthy peer.
/// 3. **Resolve `sub` → `users.id`** via
///    [`db::users::find_or_provision_by_external_id`].
///    - Hit → attach the existing [`UserId`].
///    - Miss → lazy-provision a fresh row (a valid JWT from the
///      configured issuer IS the authoritative onboarding signal,
///      so a separate `POST /users` isn't needed).
///    - DB error → 500. Signature was valid, so this is server-side
///      fault, not a client problem.
pub async fn authenticate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let Some(auth_header) = request.headers().get(axum::http::header::AUTHORIZATION) else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    let user_id = resolve_bearer(&state.jwt_verifier, &state, auth_header).await?;
    request.extensions_mut().insert(UserId(user_id));
    Ok(next.run(request).await)
}

/// Verify a `Authorization` header against the configured JWT
/// verifier and resolve the resulting `sub` against `users.external_id`.
/// Maps each [`AuthError`] variant to its documented status code.
async fn resolve_bearer(
    verifier: &JwtVerifier,
    state: &AppState,
    header_value: &HeaderValue,
) -> Result<i64, StatusCode> {
    let header_str = header_value.to_str().map_err(|_| {
        // Non-UTF8 header value — RFC violations get the same
        // 401 as missing / wrong-scheme. No need to log: a bad
        // client is already on its way to a tighter error path.
        StatusCode::UNAUTHORIZED
    })?;

    let claims = verifier.verify_bearer(header_str).await.map_err(|err| {
        // The verifier's discriminants intentionally separate
        // client errors (401) from upstream-unreachable (503).
        // Log the actual reason at warn so operators can correlate
        // a 401 spike with bad client config; the response body
        // stays opaque to keep enumeration probes blind.
        match err {
            AuthError::JwksFetchFailed(_) => {
                tracing::warn!(error = %err, "JWKS fetch failed — JWT path returning 503");
                StatusCode::SERVICE_UNAVAILABLE
            }
            AuthError::EmptyJwks => {
                tracing::warn!(error = %err, "JWKS empty — JWT path returning 503");
                StatusCode::SERVICE_UNAVAILABLE
            }
            other => {
                tracing::warn!(error = %other, "JWT verification rejected");
                StatusCode::UNAUTHORIZED
            }
        }
    })?;

    // Resolve sub → users.id, lazy-provisioning on cache miss. The
    // verified JWT is the authoritative statement that the sub is a
    // real user (Better Auth signed it), so a missing row means
    // "first request from a fresh signup", not "intruder".
    let created_at_ms = chrono::Utc::now().timestamp_millis();
    let user_id =
        db::users::find_or_provision_by_external_id(&state.db, &claims.sub, created_at_ms)
            .await
            .map_err(|err| {
                tracing::error!(error = %err, "users provision failed during JWT auth");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

    Ok(user_id)
}
