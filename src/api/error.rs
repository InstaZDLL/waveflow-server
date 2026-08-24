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
    Unavailable,
    NotFound,
}

impl From<AuthError> for ApiError {
    fn from(value: AuthError) -> Self {
        match value {
            AuthError::InvalidCredentials | AuthError::InvalidRefreshToken => Self::Unauthorized,
            AuthError::InvalidDeviceName => Self::Validation,
            AuthError::Unavailable => Self::Unavailable,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication failed",
            ),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden", "Request rejected"),
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
            Self::Unavailable => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "Authentication is temporarily unavailable",
            ),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
        };
        (status, Json(ErrorResponse { code, message })).into_response()
    }
}

pub(super) fn db_error(error: sqlx::Error) -> ApiError {
    tracing::error!(error = %error, "catalog database operation failed");
    ApiError::Unavailable
}

pub(super) fn sync_error(error: crate::sync::SyncError) -> ApiError {
    match error {
        crate::sync::SyncError::Invalid => ApiError::Validation,
        crate::sync::SyncError::Conflict => ApiError::Conflict,
        crate::sync::SyncError::CursorExpired => ApiError::CursorExpired,
        crate::sync::SyncError::Database(error) => db_error(error),
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
        ServiceError::Unavailable => ApiError::Unavailable,
        ServiceError::Database(error) => db_error(error),
        ServiceError::Security(error) => {
            tracing::error!(error = %error, "catalog security operation failed");
            ApiError::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{public_url_is_https, sync_notice_action, SyncNoticeAction};

    #[test]
    fn secure_cookie_detection_uses_the_parsed_url_scheme() {
        assert!(public_url_is_https(Some("HTTPS://waveflow.test/")));
        assert!(!public_url_is_https(Some("http://waveflow.test")));
        assert!(!public_url_is_https(Some("not a URL")));
        assert!(!public_url_is_https(None));
    }

    #[tokio::test]
    async fn lagged_sync_socket_recovers_from_the_durable_cursor() {
        let temp = tempfile::tempdir().unwrap();
        let config = crate::Config::for_data_dir(temp.path().join("data"));
        let db = crate::database::Database::open(&config).await.unwrap();
        db.migrate().await.unwrap();
        let sync = crate::sync::SyncService::new(db);
        let action = sync_notice_action(
            &sync,
            uuid::Uuid::new_v4(),
            Err(tokio::sync::broadcast::error::RecvError::Lagged(3)),
        )
        .await
        .unwrap();

        assert_eq!(action, SyncNoticeAction::Send(0));
    }
}
