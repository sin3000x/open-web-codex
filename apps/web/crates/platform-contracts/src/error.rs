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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_catalog_failure: Option<ProviderCatalogFailure>,
}

/// Typed, bounded failure causes returned by Codex when a Provider model
/// catalog cannot be fetched. This is intentionally a platform error detail,
/// not a raw Runtime error or upstream response body.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCatalogFailure {
    Authentication,
    NotFound,
    RateLimited,
    Upstream,
    Timeout,
    Network,
    InvalidJson,
    IncompatibleSchema,
    EmptyCatalog,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    WorkspaceUnavailable,
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
            provider_catalog_failure: None,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::NotFound,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
            provider_catalog_failure: None,
        }
    }

    pub fn workspace_unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::WorkspaceUnavailable,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
            provider_catalog_failure: None,
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Conflict,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
            provider_catalog_failure: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Internal,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
            provider_catalog_failure: None,
        }
    }
    /// Codex answered, but rejected the request or returned an unusable result.
    pub fn codex_rejected(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::CodexRejected,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
            provider_catalog_failure: None,
        }
    }

    /// Codex could not be reached, was restarting, busy, timed out or lacks a
    /// required capability.
    pub fn codex_unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::CodexUnavailable,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
            provider_catalog_failure: None,
        }
    }

    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Unauthorized,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
            provider_catalog_failure: None,
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Forbidden,
            message: message.into(),
            request_id: None,
            retry_after_ms: None,
            provider_catalog_failure: None,
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
            ErrorKind::WorkspaceUnavailable => 409,
            ErrorKind::Conflict => 409,
            ErrorKind::Unprocessable => 422,
            ErrorKind::RateLimited => 429,
            ErrorKind::CodexUnavailable => 503,
            ErrorKind::CodexRejected => 502,
            ErrorKind::Internal => 500,
        }
    }
}
