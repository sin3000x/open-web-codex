use std::path::Path;
use std::sync::Arc;

use axum::extract::{Path as AxumPath, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use open_web_codex_git_runtime::GitRuntime;
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    CreateGitHubRepositoryRequest, CreateGitHubRepositoryResponse, GitHubIssue, GitHubIssues,
    GitHubPullRequest, GitHubPullRequestComment, GitHubPullRequestDiff, GitHubPullRequests,
};
use open_web_codex_platform_store::AppState;
use tokio::process::Command;
use uuid::Uuid;

use crate::middleware::auth::AuthenticatedUser;
use crate::routes::workspaces::authorized_workspace;

type ApiError = (StatusCode, Json<PlatformError>);
type ApiResult<T> = Result<Json<T>, ApiError>;

const MAX_GITHUB_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

pub async fn create_repository(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(run_id): AxumPath<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
    Json(request): Json<CreateGitHubRepositoryRequest>,
) -> ApiResult<CreateGitHubRepositoryResponse> {
    let workspace_id = authorized_workspace(&state, &auth, run_id, true).await?;
    let workspace = git.workspace_path(workspace_id);
    let mut repository = validate_requested_repository(&request.repo)?;
    if !repository.contains('/') {
        let owner = run_gh(&workspace, ["api", "user", "--jq", ".login"]).await?;
        let owner = String::from_utf8(owner)
            .map_err(|_| bad_gateway("GitHub returned an invalid account name"))?;
        let owner = owner.trim();
        if !valid_repo_segment(owner) {
            return Err(bad_gateway("GitHub returned an invalid account name"));
        }
        repository = format!("{owner}/{repository}");
    }
    let visibility = match request.visibility.trim() {
        "private" => "--private",
        "public" => "--public",
        _ => {
            return Err(bad_request(
                "Repository visibility must be private or public",
            ))
        }
    };
    let origin_before = git_remote_url(&workspace).await?;
    if let Some(remote) = origin_before.as_deref() {
        let existing = parse_github_repository(remote)
            .ok_or_else(|| bad_request("The existing origin remote is not a GitHub repository"))?;
        if !existing.eq_ignore_ascii_case(&repository) {
            return Err(bad_request(
                "The existing origin remote points to a different GitHub repository",
            ));
        }
    }

    if !github_repository_exists(&workspace, &repository).await {
        let mut arguments = vec!["repo", "create", repository.as_str(), visibility];
        if origin_before.is_none() {
            arguments.extend(["--source=.", "--remote=origin"]);
        }
        run_gh(&workspace, arguments).await?;
    }

    if git_remote_url(&workspace).await?.is_none() {
        let protocol = run_gh(&workspace, ["config", "get", "git_protocol"])
            .await
            .ok()
            .and_then(|value| String::from_utf8(value).ok())
            .unwrap_or_default();
        let jq = if protocol.trim() == "ssh" {
            ".sshUrl"
        } else {
            ".httpsUrl"
        };
        let remote = run_gh(
            &workspace,
            [
                "repo",
                "view",
                repository.as_str(),
                "--json",
                "sshUrl,httpsUrl",
                "--jq",
                jq,
            ],
        )
        .await?;
        let remote = String::from_utf8(remote)
            .map_err(|_| bad_gateway("GitHub returned an invalid remote URL"))?;
        let remote = remote.trim();
        if !(remote.starts_with("https://github.com/") || remote.starts_with("git@github.com:")) {
            return Err(bad_gateway("GitHub returned an invalid remote URL"));
        }
        run_git(&workspace, ["remote", "add", "origin", remote]).await?;
    }

    let remote_url = git_remote_url(&workspace).await?;
    let push_error = run_git(&workspace, ["push", "-u", "origin", "HEAD"])
        .await
        .err()
        .map(|_| "The repository was created, but the initial push failed".to_string());
    let branch = match request.branch.as_deref().map(str::trim) {
        Some(branch) if !branch.is_empty() => Some(validate_branch(branch)?),
        _ => current_branch(&workspace).await?,
    };
    let default_branch_error = if let Some(branch) = branch.as_deref() {
        let endpoint = format!("/repos/{repository}");
        let field = format!("default_branch={branch}");
        run_gh(
            &workspace,
            [
                "api",
                "-X",
                "PATCH",
                endpoint.as_str(),
                "-f",
                field.as_str(),
            ],
        )
        .await
        .err()
        .map(|_| {
            "The repository was created, but its default branch could not be updated".to_string()
        })
    } else {
        None
    };
    let status = if push_error.is_some() || default_branch_error.is_some() {
        "partial"
    } else {
        "ok"
    };
    sqlx::query(
        "INSERT INTO audit_log \
         (organization_id, actor_id, action, target_type, target_id, metadata, outcome) \
         VALUES ($1, $2, 'github.repository_create', 'workspace', $3, $4, $5)",
    )
    .bind(auth.organization_id)
    .bind(auth.user_id)
    .bind(workspace_id)
    .bind(serde_json::json!({ "runId": run_id, "repository": repository, "visibility": request.visibility }))
    .bind(if status == "ok" { "success" } else { "partial" })
    .execute(&state.db)
    .await
    .map_err(|_| bad_gateway("GitHub repository was created but audit recording failed"))?;
    Ok(Json(CreateGitHubRepositoryResponse {
        status: status.to_string(),
        repo: repository,
        remote_url,
        push_error,
        default_branch_error,
    }))
}

pub async fn issues(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(run_id): AxumPath<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<GitHubIssues> {
    let (workspace, repository) = context(&state, &auth, run_id, &git, false).await?;
    let output = run_gh(
        &workspace,
        [
            "issue",
            "list",
            "--repo",
            repository.as_str(),
            "--limit",
            "50",
            "--json",
            "number,title,url,updatedAt",
        ],
    )
    .await?;
    let issues: Vec<GitHubIssue> = serde_json::from_slice(&output)
        .map_err(|_| bad_gateway("GitHub returned invalid issue data"))?;
    Ok(Json(GitHubIssues {
        total: issues.len(),
        issues,
    }))
}

pub async fn pull_requests(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(run_id): AxumPath<Uuid>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<GitHubPullRequests> {
    let (workspace, repository) = context(&state, &auth, run_id, &git, false).await?;
    let output = run_gh(
        &workspace,
        [
            "pr",
            "list",
            "--repo",
            repository.as_str(),
            "--state",
            "open",
            "--limit",
            "50",
            "--json",
            "number,title,url,updatedAt,createdAt,body,headRefName,baseRefName,isDraft,author",
        ],
    )
    .await?;
    let pull_requests: Vec<GitHubPullRequest> = serde_json::from_slice(&output)
        .map_err(|_| bad_gateway("GitHub returned invalid pull request data"))?;
    Ok(Json(GitHubPullRequests {
        total: pull_requests.len(),
        pull_requests,
    }))
}

pub async fn pull_request_diff(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath((run_id, number)): AxumPath<(Uuid, u64)>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<Vec<GitHubPullRequestDiff>> {
    validate_pr_number(number)?;
    let (workspace, repository) = context(&state, &auth, run_id, &git, false).await?;
    let number = number.to_string();
    let output = run_gh(
        &workspace,
        [
            "pr",
            "diff",
            number.as_str(),
            "--repo",
            repository.as_str(),
            "--color",
            "never",
        ],
    )
    .await?;
    let text = String::from_utf8(output)
        .map_err(|_| bad_gateway("GitHub returned a non-text pull request diff"))?;
    Ok(Json(parse_pr_diff(&text)))
}

pub async fn pull_request_comments(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath((run_id, number)): AxumPath<(Uuid, u64)>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<Vec<GitHubPullRequestComment>> {
    validate_pr_number(number)?;
    let (workspace, repository) = context(&state, &auth, run_id, &git, false).await?;
    let endpoint = format!("/repos/{repository}/issues/{number}/comments?per_page=30");
    let filter = "[.[] | {id, body, createdAt: .created_at, url: .html_url, author: (if .user then {login: .user.login} else null end)}]";
    let output = run_gh(&workspace, ["api", endpoint.as_str(), "--jq", filter]).await?;
    let comments = serde_json::from_slice(&output)
        .map_err(|_| bad_gateway("GitHub returned invalid pull request comment data"))?;
    Ok(Json(comments))
}

pub async fn checkout_pull_request(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath((run_id, number)): AxumPath<(Uuid, u64)>,
    Extension(git): Extension<Arc<GitRuntime>>,
) -> ApiResult<serde_json::Value> {
    validate_pr_number(number)?;
    let (workspace, _) = context(&state, &auth, run_id, &git, true).await?;
    let number = number.to_string();
    run_gh(&workspace, ["pr", "checkout", number.as_str()]).await?;
    Ok(Json(serde_json::json!({ "status": "checkedOut" })))
}

async fn context(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
    git: &GitRuntime,
    require_owner: bool,
) -> Result<(std::path::PathBuf, String), ApiError> {
    let workspace_id = authorized_workspace(state, auth, run_id, require_owner).await?;
    let remote = git
        .remote(workspace_id)
        .await
        .map_err(|_| bad_gateway("Unable to read the Git remote"))?
        .ok_or_else(|| bad_request("No Git remote is configured"))?;
    let repository = parse_github_repository(&remote)
        .ok_or_else(|| bad_request("The Git remote is not a GitHub repository"))?;
    Ok((git.workspace_path(workspace_id), repository))
}

async fn run_gh<'a, I>(workspace: &Path, args: I) -> Result<Vec<u8>, ApiError>
where
    I: IntoIterator<Item = &'a str>,
{
    let output = Command::new("gh")
        .args(args)
        .current_dir(workspace)
        .env("GH_PAGER", "cat")
        .env("NO_COLOR", "1")
        .output()
        .await
        .map_err(|_| bad_gateway("GitHub CLI is not available on the Runner"))?;
    if !output.status.success() {
        return Err(bad_gateway("GitHub CLI request failed"));
    }
    if output.stdout.len() > MAX_GITHUB_OUTPUT_BYTES {
        return Err(bad_gateway("GitHub response exceeded the supported size"));
    }
    Ok(output.stdout)
}

async fn run_git<'a, I>(workspace: &Path, args: I) -> Result<Vec<u8>, ApiError>
where
    I: IntoIterator<Item = &'a str>,
{
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|_| bad_gateway("Git is not available on the Runner"))?;
    if !output.status.success() {
        return Err(bad_gateway("Git repository operation failed"));
    }
    if output.stdout.len() > MAX_GITHUB_OUTPUT_BYTES {
        return Err(bad_gateway("Git response exceeded the supported size"));
    }
    Ok(output.stdout)
}

async fn github_repository_exists(workspace: &Path, repository: &str) -> bool {
    run_gh(
        workspace,
        [
            "repo", "view", repository, "--json", "name", "--jq", ".name",
        ],
    )
    .await
    .is_ok()
}

async fn git_remote_url(workspace: &Path) -> Result<Option<String>, ApiError> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(workspace)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .map_err(|_| bad_gateway("Git is not available on the Runner"))?;
    if !output.status.success() {
        return Ok(None);
    }
    let remote = String::from_utf8(output.stdout)
        .map_err(|_| bad_gateway("Git returned an invalid remote URL"))?;
    let remote = remote.trim();
    Ok((!remote.is_empty()).then(|| remote.to_string()))
}

async fn current_branch(workspace: &Path) -> Result<Option<String>, ApiError> {
    let output = run_git(workspace, ["branch", "--show-current"]).await?;
    let branch = String::from_utf8(output)
        .map_err(|_| bad_gateway("Git returned an invalid branch name"))?;
    let branch = branch.trim();
    if branch.is_empty() {
        Ok(None)
    } else {
        Ok(Some(validate_branch(branch)?))
    }
}

fn validate_requested_repository(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    let parts: Vec<_> = value.split('/').collect();
    if parts.is_empty() || parts.len() > 2 || parts.iter().any(|part| !valid_repo_segment(part)) {
        return Err(bad_request(
            "Repository must be a valid name or owner/name pair",
        ));
    }
    Ok(value.to_string())
}

fn validate_branch(value: &str) -> Result<String, ApiError> {
    if value.is_empty()
        || value.len() > 255
        || value.starts_with('-')
        || value.contains("..")
        || value.contains("@{")
        || value.ends_with('.')
        || value.ends_with('/')
        || value.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
    {
        return Err(bad_request("Default branch name is invalid"));
    }
    Ok(value.to_string())
}

fn parse_github_repository(remote: &str) -> Option<String> {
    let trimmed = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let path = if let Some(path) = trimmed.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = trimmed.strip_prefix("ssh://git@github.com/") {
        path
    } else if let Some(path) = trimmed.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = trimmed.strip_prefix("http://github.com/") {
        path
    } else {
        return None;
    };
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repository = parts.next()?;
    if parts.next().is_some() || !valid_repo_segment(owner) || !valid_repo_segment(repository) {
        return None;
    }
    Some(format!("{owner}/{repository}"))
}

fn valid_repo_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 100
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
}

fn parse_pr_diff(diff: &str) -> Vec<GitHubPullRequestDiff> {
    let mut results = Vec::new();
    let mut lines = Vec::new();
    let mut path = String::new();
    let mut status = "M".to_string();
    let finish = |lines: &mut Vec<&str>,
                  path: &str,
                  status: &str,
                  results: &mut Vec<GitHubPullRequestDiff>| {
        if !lines.is_empty() && !path.is_empty() {
            results.push(GitHubPullRequestDiff {
                path: path.to_string(),
                status: status.to_string(),
                diff: lines.join("\n"),
            });
        }
        lines.clear();
    };
    for line in diff.lines() {
        if let Some(header) = line.strip_prefix("diff --git ") {
            finish(&mut lines, &path, &status, &mut results);
            let mut parts = header.split_whitespace();
            let old = parts.next().unwrap_or_default().trim_start_matches("a/");
            let new = parts.next().unwrap_or_default().trim_start_matches("b/");
            path = if new.is_empty() { old } else { new }.to_string();
            status = "M".to_string();
        } else if line.starts_with("new file mode ") {
            status = "A".to_string();
        } else if line.starts_with("deleted file mode ") {
            status = "D".to_string();
        } else if let Some(renamed) = line.strip_prefix("rename to ") {
            status = "R".to_string();
            path = renamed.trim().to_string();
        }
        lines.push(line);
    }
    finish(&mut lines, &path, &status, &mut results);
    results
}

fn validate_pr_number(number: u64) -> Result<(), ApiError> {
    if number == 0 {
        return Err(bad_request("Pull request number must be positive"));
    }
    Ok(())
}

fn bad_request(message: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(PlatformError::bad_request(message)),
    )
}

fn bad_gateway(message: &str) -> ApiError {
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal(message)),
    )
}

#[cfg(test)]
mod tests {
    use super::{parse_github_repository, parse_pr_diff};

    #[test]
    fn accepts_only_canonical_github_remotes() {
        assert_eq!(
            parse_github_repository("git@github.com:openai/codex.git"),
            Some("openai/codex".to_string())
        );
        assert_eq!(
            parse_github_repository("https://github.com/openai/codex.git"),
            Some("openai/codex".to_string())
        );
        assert_eq!(
            parse_github_repository("https://example.com/openai/codex"),
            None
        );
    }

    #[test]
    fn splits_pull_request_patch_by_file() {
        let diffs = parse_pr_diff(
            "diff --git a/a.txt b/a.txt\nnew file mode 100644\n--- /dev/null\n+++ b/a.txt\n+hello\n",
        );
        assert_eq!(diffs.len(), 1);
        assert_eq!(diffs[0].path, "a.txt");
        assert_eq!(diffs[0].status, "A");
    }
}
