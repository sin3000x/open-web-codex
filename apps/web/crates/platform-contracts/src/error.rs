use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Platform-level API error.
#[derive(Debug, Clone, Serialize, Deserialize, Error)]
#[error("{kind:?}: {message}")]
pub struct PlatformError {
    pub kind: ErrorKind,
    pub message: String,
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    Unprocessable,
    RateLimited,
    CodexUnavailable,
    CodexRejected,
    Internal,
}

impl PlatformError {
    pub fn bad_request(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::BadRequest,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::NotFound,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Internal,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
        }
    }
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Unauthorized,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Forbidden,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
        }
    }

    pub fn with_request_id(mut self, request_id: String) -> Self {
        self.request_id = Some(request_id);
        self
    }
}

/// Convert PlatformError into an HTTP response status code.
impl From<&PlatformError> for u16 {
    fn from(err: &PlatformError) -> u16 {
        match err.kind {
            ErrorKind::BadRequest => 400,
            ErrorKind::Unauthorized => 401,
            ErrorKind::Forbidden => 403,
            ErrorKind::NotFound => 404,
            ErrorKind::Conflict => 409,
            ErrorKind::Unprocessable => 422,
            ErrorKind::RateLimited => 429,
            ErrorKind::CodexUnavailable => 503,
            ErrorKind::CodexRejected => 502,
            ErrorKind::Internal => 500,
        }
    }
}
