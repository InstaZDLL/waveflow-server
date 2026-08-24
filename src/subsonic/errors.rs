//! The protocol error constructors.
//!
//! Split out of `subsonic.rs`; the wire contract is frozen, so this moved nothing.

use super::*;

pub(super) fn auth_error() -> ProtocolError {
    ProtocolError {
        code: 40,
        message: "Wrong username or password",
    }
}

pub(super) fn missing() -> ProtocolError {
    ProtocolError {
        code: 10,
        message: "Required parameter is missing",
    }
}

pub(super) fn invalid(message: &'static str) -> ProtocolError {
    ProtocolError { code: 10, message }
}

pub(super) fn not_found() -> ProtocolError {
    ProtocolError {
        code: 70,
        message: "The requested data was not found",
    }
}

pub(super) fn internal(error: impl std::fmt::Display) -> ProtocolError {
    tracing::error!(error = %error, "Subsonic service failure");
    ProtocolError {
        code: 0,
        message: "Internal server error",
    }
}

pub(super) fn service_protocol(error: ServiceError) -> ProtocolError {
    match error {
        ServiceError::NotFound => not_found(),
        ServiceError::Forbidden => ProtocolError {
            code: 50,
            message: "User is not authorized for the given operation",
        },
        ServiceError::Invalid => invalid("Invalid parameters"),
        ServiceError::Conflict => ProtocolError {
            code: 0,
            message: "Conflict",
        },
        other => internal(other),
    }
}
