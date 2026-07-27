use std::path::Path;

use url::Url;
use uuid::Uuid;

use crate::GitRuntimeError;

#[derive(Debug, Clone, Copy)]
pub struct GitSourcePolicy {
    allow_local: bool,
}

impl GitSourcePolicy {
    pub fn remote_only() -> Self {
        Self { allow_local: false }
    }

    pub fn allow_local() -> Self {
        Self { allow_local: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedGitSource(String);

impl ValidatedGitSource {
    pub fn parse(source: &str, policy: GitSourcePolicy) -> Result<Self, GitRuntimeError> {
        let source = source.trim();
        if source.is_empty()
            || source.starts_with('-')
            || source.contains(['\0', '\n', '\r'])
            || source.chars().any(char::is_whitespace)
        {
            return Err(GitRuntimeError::InvalidSource(
                "malformed source".to_string(),
            ));
        }

        if policy.allow_local && Path::new(source).is_absolute() {
            return Ok(Self(source.to_string()));
        }
        if is_scp_source(source) {
            return Ok(Self(source.to_string()));
        }

        let parsed = Url::parse(source)
            .map_err(|_| GitRuntimeError::InvalidSource("expected HTTPS or SSH URL".to_string()))?;
        let scheme_allowed = matches!(parsed.scheme(), "https" | "ssh")
            || (policy.allow_local && parsed.scheme() == "file");
        if !scheme_allowed {
            return Err(GitRuntimeError::InvalidSource(
                "unsupported URL scheme".to_string(),
            ));
        }
        if parsed.scheme() != "file" && parsed.host_str().is_none() {
            return Err(GitRuntimeError::InvalidSource(
                "remote URL omitted host".to_string(),
            ));
        }
        if parsed.password().is_some()
            || (parsed.scheme() == "https" && !parsed.username().is_empty())
        {
            return Err(GitRuntimeError::InvalidSource(
                "embedded credentials are not allowed".to_string(),
            ));
        }
        if parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path().trim_matches('/').is_empty()
        {
            return Err(GitRuntimeError::InvalidSource(
                "malformed repository URL".to_string(),
            ));
        }
        Ok(Self(source.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn managed(project_id: Uuid) -> Self {
        Self(format!("managed://{project_id}"))
    }

    pub(crate) fn managed_project_id(&self) -> Option<Uuid> {
        self.0
            .strip_prefix("managed://")
            .and_then(|value| Uuid::parse_str(value).ok())
    }
}

fn is_scp_source(source: &str) -> bool {
    let Some((authority, path)) = source.split_once(':') else {
        return false;
    };
    let Some((user, host)) = authority.split_once('@') else {
        return false;
    };
    !user.is_empty()
        && !host.is_empty()
        && !path.trim_matches('/').is_empty()
        && !path.starts_with('-')
        && authority
            .chars()
            .chain(path.chars())
            .all(|character| character.is_ascii_alphanumeric() || "@._/-".contains(character))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedGitRef(String);

impl ValidatedGitRef {
    pub fn parse(git_ref: &str) -> Result<Self, GitRuntimeError> {
        let value = git_ref.trim();
        let invalid = value.is_empty()
            || value.len() > 255
            || value == "@"
            || value.starts_with(['-', '.', '/'])
            || value.ends_with(['.', '/'])
            || value.contains("..")
            || value.contains("@{")
            || value.contains("//")
            || value
                .chars()
                .any(|character| character.is_control() || " ~^:?*[\\".contains(character))
            || value.split('/').any(|component| {
                component.is_empty() || component.starts_with('.') || component.ends_with(".lock")
            });
        if invalid {
            return Err(GitRuntimeError::InvalidRef(
                "malformed branch name".to_string(),
            ));
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}
