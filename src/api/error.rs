//! The API error type and the mappings onto it.
//!
//! Split out of `http.rs`; `mod.rs` re-exports it, so `crate::api::*` paths are unchanged.

use super::*;

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorResponse {
    pub code: &'static str,
    pub message: &'static str,
}

#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    Forbidden,
    Validation,
    /// The request is well formed but collides with existing state: an
    /// operation id replayed with a different payload, or a name already taken.
    /// Distinct from `Validation` so a client can tell "my request is malformed"
    /// from "my retry is inconsistent" — both permanent, different fixes.
    Conflict,
    /// The sync cursor precedes the oldest retained event. Same 409 status as
    /// `Conflict` but a distinct code, because the reactions are opposite:
    /// a conflict means mint a new operation id, this one means discard the
    /// local projection and take a fresh snapshot.
    CursorExpired,
    /// The caller's previous request of the same kind is still running.
    TooManyRequests,
    /// Too many failed sign-ins for this account from this address
    /// (RFC-017). Answered with `Retry-After`, and nothing was checked.
    SignInThrottled,
    /// First-run setup is open and the setup token is missing or wrong
    /// (RFC-019 decision 1). Its own code, so the setup screen can say which
    /// field to correct.
    SetupTokenRejected,
    Unavailable,
    NotFound,
}

impl From<AuthError> for ApiError {
    fn from(value: AuthError) -> Self {
        match value {
            AuthError::InvalidCredentials | AuthError::InvalidRefreshToken => Self::Unauthorized,
            AuthError::InvalidDeviceName => Self::Validation,
            AuthError::Throttled => Self::SignInThrottled,
            AuthError::Unavailable => Self::Unavailable,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let retry_after = matches!(self, Self::SignInThrottled);
        let (status, code, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication failed",
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Request rejected"),
            Self::SetupTokenRejected => (
                StatusCode::FORBIDDEN,
                "setup_token_rejected",
                "The setup token is missing or wrong",
            ),
            Self::Validation => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                "The request is invalid",
            ),
            Self::Conflict => (
                StatusCode::CONFLICT,
                "conflict",
                "The request conflicts with existing state",
            ),
            Self::CursorExpired => (
                StatusCode::CONFLICT,
                "cursor_expired",
                "The cursor precedes the oldest retained event; take a fresh snapshot",
            ),
            Self::TooManyRequests => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_requests",
                "The previous request is still running",
            ),
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "Authentication is temporarily unavailable",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
            Self::SignInThrottled => (
                StatusCode::TOO_MANY_REQUESTS,
                "too_many_attempts",
                "Too many failed sign-in attempts; try again later",
            ),
        };
        let mut response = (status, Json(ErrorResponse { code, message })).into_response();
        if retry_after {
            // The window the limiter counts over: by then, every failure that
            // filled it has aged out.
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        }
        response
    }
}

pub(super) fn db_error(error: sqlx::Error) -> ApiError {
    tracing::error!(error = %error, "catalog database operation failed");
    ApiError::Unavailable
}

pub(super) fn sync_error(error: crate::storage::sync::SyncError) -> ApiError {
    match error {
        crate::storage::sync::SyncError::Invalid => ApiError::Validation,
        crate::storage::sync::SyncError::Conflict => ApiError::Conflict,
        crate::storage::sync::SyncError::CursorExpired => ApiError::CursorExpired,
        crate::storage::sync::SyncError::Database(error) => db_error(error),
    }
}

/// Maps a domain failure onto the HTTP surface. `Forbidden` deliberately answers
/// 404 like `NotFound`: telling a caller that a resource exists but belongs to
/// someone else would leak another tenant's catalogue, which is the same
/// no-existence-leak rule the Subsonic facade applies.
pub(super) fn service_error(error: crate::services::ServiceError) -> ApiError {
    use crate::services::ServiceError;
    match error {
        ServiceError::NotFound | ServiceError::Forbidden => ApiError::NotFound,
        ServiceError::Invalid => ApiError::Validation,
        ServiceError::Conflict => ApiError::Conflict,
        ServiceError::Busy => ApiError::TooManyRequests,
        ServiceError::Unavailable => ApiError::Unavailable,
        ServiceError::Database(error) => db_error(error),
        ServiceError::Security(error) => {
            tracing::error!(error = %error, "catalog security operation failed");
            ApiError::Unavailable
        }
    }
}
