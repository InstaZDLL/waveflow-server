//! HTTP middleware.
//!
//! Two authentication paths live here: the dev-only `X-User-Id`
//! header shim (Phase 1.b) and the Bearer-JWT verifier (Phase
//! 1.d.1-PR3). Both feed the same [`UserId`] extension shape so the
//! handlers downstream can't tell them apart.
//!
//! Phase 1.d.2 retires the shim entirely once Better Auth is deployed
//! as the only auth path — this module shrinks to just the JWT
//! middleware then.

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode},
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
    // Delegates to the shared [`parse_x_user_id_header`] helper —
    // the `i64 > 0` invariant + the missing/malformed → 401 mapping
    // live there. Phase 1.d.1-PR3's `authenticate` middleware uses
    // the same helper, so there's exactly one place to audit when
    // the shim's contract needs revisiting. 1.d.2 will delete both
    // this function AND the helper.
    let user_id = parse_x_user_id_header(request.headers())?;
    request.extensions_mut().insert(UserId(user_id));
    Ok(next.run(request).await)
}

/// Unified auth middleware for `/api/v1/*`. Tries Bearer JWT first
/// (when [`AppState::jwt_verifier`] is `Some`), falls back to the
/// dev `X-User-Id` shim (when [`AppState::dev_auth_enabled`]) and
/// short-circuits to **503** when neither path is configured — the
/// production gate documented in [`crate::Config::auth_disabled_at_boot`].
///
/// Per-request flow:
///
/// 1. **No auth configured at all** → 503. The state matches a
///    fresh boot where the operator hasn't pointed at a JWKS and
///    hasn't flipped the shim on; failing closed is the only safe
///    default.
/// 2. **JWT configured, `Authorization` present** → verify the token,
///    resolve `sub` → `users.id`, attach [`UserId`].
///    - Bad signature / claims / `kid` → 401 (no body detail, the
///      reason lands in `tracing::warn!`).
///    - JWKS fetch failure → 503. Routes around the instance while
///      the upstream is unreachable, lets the load balancer pick a
///      healthy peer.
///    - `sub` has no `users` row yet → lazy-provision it via
///      [`db::users::find_or_provision_by_external_id`] and attach
///      the freshly-minted [`UserId`]. The verified JWT is the
///      authoritative onboarding signal; no separate `POST /users`
///      is needed after a Better Auth signup (Phase 1.c.3a).
///    - DB error while provisioning → 500. The signature was
///      valid, so this is server-side fault, not a client problem.
/// 3. **JWT configured, NO `Authorization`** → fall through to the
///    shim path if it's enabled; otherwise 401.
/// 4. **Shim path** — same parse as the legacy `require_user_id`:
///    `X-User-Id` must be an `i64 > 0`. Reject 401 if missing or
///    malformed.
///
/// Order matters: JWT before shim means a request that carries both
/// headers gets authenticated by the cryptographically-trusted side,
/// not the forgeable one. Phase 1.d.2 deletes the shim branch
/// entirely.
pub async fn authenticate(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Production gate: no auth configured at boot → every request
    // 503. Matches the legacy `reject_dev_auth_disabled` behaviour
    // for the case where the shim was off, generalised to the JWT
    // path being off too.
    if state.jwt_verifier.is_none() && !state.dev_auth_enabled {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }

    // JWT path — try first whenever the verifier is configured AND
    // the client supplied an `Authorization` header. Missing header
    // falls through to the shim path so an existing dev client that
    // only knows `X-User-Id` keeps working during the transition.
    if let Some(verifier) = state.jwt_verifier.as_ref() {
        if let Some(auth_header) = request.headers().get(axum::http::header::AUTHORIZATION) {
            let user_id = resolve_bearer(verifier, &state, auth_header).await?;
            request.extensions_mut().insert(UserId(user_id));
            return Ok(next.run(request).await);
        }
    }

    // Shim path — only reachable when `dev_auth_enabled`. The legacy
    // `X-User-Id` parse stays bit-for-bit identical so the existing
    // 1.b.5 test suite keeps passing.
    if state.dev_auth_enabled {
        let user_id = parse_x_user_id_header(request.headers())?;
        request.extensions_mut().insert(UserId(user_id));
        return Ok(next.run(request).await);
    }

    // JWT configured but the client didn't carry `Authorization`,
    // and the shim is off — surface 401 rather than 503 since the
    // server itself IS configured (just the request wasn't).
    Err(StatusCode::UNAUTHORIZED)
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
    // "first request from a fresh signup", not "intruder". Inserting
    // here keeps the auth flow single-round-trip: no separate
    // /users POST is needed after Better Auth's signUp.email.
    //
    // The UPSERT is idempotent on the UNIQUE(external_id) index —
    // a concurrent first request from the same user collapses
    // cleanly to one row.
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

/// Legacy `X-User-Id` parse, factored out so [`require_user_id`]
/// (still used by the existing test surface) and [`authenticate`]
/// share one implementation. Phase 1.d.2 deletes this alongside the
/// `require_user_id` middleware itself.
fn parse_x_user_id_header(headers: &HeaderMap) -> Result<i64, StatusCode> {
    let value = headers
        .get(X_USER_ID_HEADER)
        .ok_or(StatusCode::UNAUTHORIZED)?;

    let user_id: i64 = value
        .to_str()
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if user_id <= 0 {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(user_id)
}
