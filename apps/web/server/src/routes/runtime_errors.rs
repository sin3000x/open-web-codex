//! Typed mapping of Codex adapter failures to browser-facing platform errors.
//!
//! The browser receives only a bounded, class-specific message. The full
//! adapter cause (including the official app-server JSON-RPC message and code)
//! is written to the server log at ERROR level so it is visible under the
//! default log filter; it never enters a browser payload.

use axum::{http::StatusCode, Json};
use open_web_codex_adapter::{AdapterError, RuntimeFailureClass};
use open_web_codex_platform_contracts::error::PlatformError;
use uuid::Uuid;

/// Upper bound for the adapter cause written to the server log.
const MAX_LOGGED_CAUSE_CHARS: usize = 1024;

/// Authoritative platform resource an adapter call was made for.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RuntimeErrorScope {
    Run(Uuid),
    Workspace(Uuid),
    Profile,
}

impl RuntimeErrorScope {
    fn run_id(self) -> Option<Uuid> {
        match self {
            Self::Run(id) => Some(id),
            Self::Workspace(_) | Self::Profile => None,
        }
    }

    fn workspace_id(self) -> Option<Uuid> {
        match self {
            Self::Workspace(id) => Some(id),
            Self::Run(_) | Self::Profile => None,
        }
    }
}

/// Log `error` with its operation context and return the typed API error.
///
/// `operation` is a stable, low-cardinality name such as `thread.archive`;
/// `subject` is the browser-visible noun such as `Thread`.
pub(crate) fn runtime_api_error(
    operation: &'static str,
    subject: &'static str,
    scope: RuntimeErrorScope,
    error: &AdapterError,
) -> (StatusCode, Json<PlatformError>) {
    let class = error.class();
    let cause = bounded_cause(error);
    tracing::error!(
        operation,
        run_id = ?scope.run_id(),
        workspace_id = ?scope.workspace_id(),
        failure_class = class.as_str(),
        rpc_code = ?error.rpc_code(),
        cause = %cause,
        "Codex Runtime operation failed"
    );
    let platform_error = match class {
        RuntimeFailureClass::Rejected => {
            PlatformError::codex_rejected(format!("Codex rejected the {subject} request"))
        }
        RuntimeFailureClass::ContractViolation => {
            PlatformError::codex_rejected(format!("Codex returned an invalid {subject} response"))
        }
        RuntimeFailureClass::Unavailable => {
            PlatformError::codex_unavailable("Codex Runtime is unavailable; retry shortly")
        }
        RuntimeFailureClass::Unsupported => PlatformError::codex_unavailable(format!(
            "Codex Runtime does not support this {subject} operation"
        )),
        RuntimeFailureClass::Platform => {
            PlatformError::internal(format!("{subject} request could not be sent to Codex"))
        }
    };
    let status = StatusCode::from_u16(u16::from(&platform_error))
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(platform_error))
}

fn bounded_cause(error: &AdapterError) -> String {
    let cause = error.to_string();
    match cause.char_indices().nth(MAX_LOGGED_CAUSE_CHARS) {
        Some((end, _)) => format!("{}…", &cause[..end]),
        None => cause,
    }
}

#[cfg(test)]
mod tests {
    use super::{runtime_api_error, RuntimeErrorScope};
    use axum::http::StatusCode;
    use open_web_codex_adapter::AdapterError;
    use open_web_codex_platform_contracts::error::ErrorKind;
    use open_web_codex_profile_host::ProfileHostError;

    fn map(error: AdapterError) -> (StatusCode, ErrorKind, String) {
        let (status, body) = runtime_api_error(
            "thread.archive",
            "Thread",
            RuntimeErrorScope::Run(uuid::Uuid::nil()),
            &error,
        );
        (status, body.0.kind, body.0.message)
    }

    #[test]
    fn maps_each_failure_class_to_a_distinct_typed_error() {
        let rejected = map(ProfileHostError::Rpc {
            method: "thread/archive".into(),
            code: Some(-32600),
            message: "no rollout found for thread id t-1 at /srv/codex-home".into(),
        }
        .into());
        assert_eq!(rejected.0, StatusCode::BAD_GATEWAY);
        assert_eq!(rejected.1, ErrorKind::CodexRejected);
        assert_eq!(rejected.2, "Codex rejected the Thread request");

        let overloaded = map(ProfileHostError::Rpc {
            method: "thread/archive".into(),
            code: Some(-32001),
            message: "overloaded".into(),
        }
        .into());
        assert_eq!(overloaded.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(overloaded.1, ErrorKind::CodexUnavailable);

        for unavailable in [
            AdapterError::Unreachable("connection refused".into()),
            ProfileHostError::TransportClosed.into(),
            ProfileHostError::RequestTimeout {
                method: "thread/archive".into(),
            }
            .into(),
        ] {
            let mapped = map(unavailable);
            assert_eq!(mapped.0, StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(mapped.1, ErrorKind::CodexUnavailable);
        }

        let unsupported = map(AdapterError::CapabilityUnavailable("thread/archive".into()));
        assert_eq!(unsupported.0, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(unsupported.1, ErrorKind::CodexUnavailable);
        assert_eq!(
            unsupported.2,
            "Codex Runtime does not support this Thread operation"
        );

        let invalid = map(AdapterError::Rpc("thread/read omitted thread".into()));
        assert_eq!(invalid.0, StatusCode::BAD_GATEWAY);
        assert_eq!(invalid.1, ErrorKind::CodexRejected);
        assert_eq!(invalid.2, "Codex returned an invalid Thread response");

        for platform in [
            AdapterError::Internal("Thread id is required".into()),
            AdapterError::NotImplemented("x".into()),
            ProfileHostError::Encode("x".into()).into(),
        ] {
            let mapped = map(platform);
            assert_eq!(mapped.0, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(mapped.1, ErrorKind::Internal);
        }
    }

    #[test]
    fn browser_message_never_contains_the_runtime_cause() {
        let secretish = "no rollout found at /Users/someone/.codex/sessions token=sk-123";
        let (_, body) = runtime_api_error(
            "thread.archive",
            "Thread",
            RuntimeErrorScope::Profile,
            &ProfileHostError::Rpc {
                method: "thread/archive".into(),
                code: Some(-32600),
                message: secretish.into(),
            }
            .into(),
        );
        let encoded = serde_json::to_string(&body.0).expect("serializable error");
        assert!(!encoded.contains("/Users/someone"));
        assert!(!encoded.contains("sk-123"));
        assert!(!encoded.contains("-32600"));
        assert!(!encoded.contains("thread/archive"));
    }

    #[test]
    fn bounds_the_logged_cause() {
        let cause = super::bounded_cause(&AdapterError::Internal("x".repeat(5000)));
        assert!(cause.chars().count() <= super::MAX_LOGGED_CAUSE_CHARS + 1);
    }
}
