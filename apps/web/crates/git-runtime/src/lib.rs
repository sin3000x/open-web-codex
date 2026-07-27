mod validation;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::{OsStr, OsString};
use std::fs::OpenOptions;
use std::path::{Component, Path, PathBuf};
use std::process::{ExitStatus, Output, Stdio};
use std::sync::{Arc, Mutex as StdMutex};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{Mutex, OwnedMutexGuard};
use uuid::Uuid;

pub use validation::{GitSourcePolicy, ValidatedGitRef, ValidatedGitSource};

const LARGE_FILE_BYTES: u64 = 1024 * 1024;
const MAX_WORKSPACE_FILES: usize = 20_000;
const MAX_FILE_READ_BYTES: u64 = 2 * 1024 * 1024;
const MAX_IMAGE_ASSET_BYTES: u64 = 50 * 1024 * 1024;
const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
const MAX_APPLY_PATCH_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum GitRuntimeError {
    #[error("invalid Git source: {0}")]
    InvalidSource(String),
    #[error("invalid Git ref: {0}")]
    InvalidRef(String),
    #[error("unsafe workspace path: {0}")]
    UnsafePath(String),
    #[error("workspace conflict: {0}")]
    Conflict(String),
    #[error("unsupported workspace image: {0}")]
    UnsupportedImage(String),
    #[error("workspace image exceeds the maximum supported size")]
    ImageTooLarge,
    #[error("Git {operation} failed: {message}")]
    Git {
        operation: &'static str,
        message: String,
    },
    #[error("I/O error during {operation}: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("no selected changes are available to commit")]
    NoChanges,
}

#[derive(Debug, Clone)]
pub struct GitRuntimeConfig {
    pub root: PathBuf,
    pub git_bin: PathBuf,
    pub source_policy: GitSourcePolicy,
}

impl GitRuntimeConfig {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            git_bin: PathBuf::from("git"),
            source_policy: GitSourcePolicy::remote_only(),
        }
    }

    pub fn with_local_sources(mut self) -> Self {
        self.source_policy = GitSourcePolicy::allow_local();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCheckout {
    pub workspace_id: Uuid,
    pub head_commit: String,
    pub branch: String,
    #[serde(skip)]
    pub root: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub binary: bool,
    pub size_bytes: Option<u64>,
    pub large: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatus {
    pub branch: String,
    pub head_commit: String,
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileContent {
    pub content: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceImageAsset {
    pub bytes: Vec<u8>,
    pub media_type: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceFileDiff {
    pub path: String,
    pub diff: String,
    pub is_binary: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    pub last_commit: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitLogEntry {
    pub sha: String,
    pub summary: String,
    pub author: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitLog {
    pub total: u64,
    pub entries: Vec<GitLogEntry>,
    pub ahead: u64,
    pub behind: u64,
    pub ahead_entries: Vec<GitLogEntry>,
    pub behind_entries: Vec<GitLogEntry>,
    pub upstream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitDiff {
    pub path: String,
    pub status: String,
    pub diff: String,
    pub is_binary: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitAuthor {
    pub name: String,
    pub email: String,
}

#[derive(Clone)]
pub struct GitRuntime {
    config: GitRuntimeConfig,
    mirrors: PathBuf,
    workspaces: PathBuf,
    locks: PathBuf,
    process_locks: Arc<StdMutex<HashMap<String, Arc<Mutex<()>>>>>,
    scoped_roots: Arc<StdMutex<HashMap<Uuid, PathBuf>>>,
}

impl GitRuntime {
    pub fn new(config: GitRuntimeConfig) -> Result<Self, GitRuntimeError> {
        reject_symlink(&config.root, "runner root")?;
        create_private_dir(&config.root, "runner root")?;
        let root = canonicalize(&config.root, "runner root")?;
        let mirrors = create_owned_child(&root, "mirrors")?;
        let workspaces = create_owned_child(&root, "workspaces")?;
        let locks = create_owned_child(&root, "locks")?;
        Ok(Self {
            config,
            mirrors,
            workspaces,
            locks,
            process_locks: Arc::new(StdMutex::new(HashMap::new())),
            scoped_roots: Arc::new(StdMutex::new(HashMap::new())),
        })
    }

    pub fn validate_source(&self, source: &str) -> Result<ValidatedGitSource, GitRuntimeError> {
        if let Some(value) = source.trim().strip_prefix("managed://") {
            let project_id = Uuid::parse_str(value).map_err(|_| {
                GitRuntimeError::InvalidSource("invalid managed project id".to_string())
            })?;
            return Ok(ValidatedGitSource::managed(project_id));
        }
        ValidatedGitSource::parse(source, self.config.source_policy)
    }

    /// Validate a source supplied by an API client. Server-owned managed
    /// sources are deliberately excluded from this boundary.
    pub fn validate_external_source(
        &self,
        source: &str,
    ) -> Result<ValidatedGitSource, GitRuntimeError> {
        ValidatedGitSource::parse(source, self.config.source_policy)
    }

    pub fn validate_ref(&self, git_ref: &str) -> Result<ValidatedGitRef, GitRuntimeError> {
        ValidatedGitRef::parse(git_ref)
    }

    pub fn workspace_path(&self, workspace_id: Uuid) -> PathBuf {
        self.workspaces.join(workspace_id.to_string())
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspaces
    }

    /// Bind Git operations for a Run workspace to a nested repository. The
    /// selection is always a validated relative path within the checkout.
    pub async fn set_workspace_git_root(
        &self,
        workspace_id: Uuid,
        relative: Option<&str>,
    ) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_base_workspace(workspace_id)?;
        let mut roots = self.scoped_roots.lock().map_err(|_| {
            GitRuntimeError::Conflict("workspace Git root lock is poisoned".to_string())
        })?;
        let Some(relative) = relative.map(str::trim).filter(|value| !value.is_empty()) else {
            roots.remove(&workspace_id);
            return Ok(());
        };
        validate_relative_path(relative)?;
        let mut candidate = workspace.clone();
        for component in Path::new(relative).components() {
            let Component::Normal(component) = component else {
                return Err(GitRuntimeError::UnsafePath(
                    "invalid nested Git root".to_string(),
                ));
            };
            candidate.push(component);
            let metadata =
                std::fs::symlink_metadata(&candidate).map_err(|source| GitRuntimeError::Io {
                    operation: "inspect nested Git root",
                    source,
                })?;
            if metadata.file_type().is_symlink() {
                return Err(GitRuntimeError::UnsafePath(
                    "nested Git root contains a symlink".to_string(),
                ));
            }
        }
        let candidate = canonicalize(&candidate, "nested Git root")?;
        if !candidate.starts_with(&workspace) || !git_marker_is_safe(&candidate) {
            return Err(GitRuntimeError::UnsafePath(
                "nested Git root is not a repository inside the workspace".to_string(),
            ));
        }
        roots.insert(workspace_id, candidate);
        Ok(())
    }

    pub async fn list_git_roots(
        &self,
        workspace_id: Uuid,
        depth: usize,
    ) -> Result<Vec<String>, GitRuntimeError> {
        let workspace = self.require_base_workspace(workspace_id)?;
        let depth = depth.clamp(1, 6);
        tokio::task::spawn_blocking(move || scan_nested_git_roots(&workspace, depth, 200))
            .await
            .map_err(|error| {
                GitRuntimeError::Conflict(format!("nested Git root scan failed: {error}"))
            })
    }

    pub async fn provision(
        &self,
        project_id: Uuid,
        workspace_id: Uuid,
        source: &ValidatedGitSource,
        git_ref: &ValidatedGitRef,
    ) -> Result<WorkspaceCheckout, GitRuntimeError> {
        let _lock = self.acquire_project_lock(project_id).await?;
        let mirror = self.ensure_mirror(project_id, workspace_id, source).await?;
        let commit = self.resolve_commit(&mirror, git_ref).await?;
        let workspace = self.workspace_path(workspace_id);
        reject_existing_workspace(&workspace)?;

        let result = async {
            self.git(
                "workspace clone",
                None,
                [
                    OsString::from("clone"),
                    OsString::from("--no-hardlinks"),
                    OsString::from("--reference-if-able"),
                    mirror.as_os_str().to_owned(),
                    OsString::from("--no-checkout"),
                    mirror.as_os_str().to_owned(),
                    workspace.as_os_str().to_owned(),
                ],
            )
            .await?;
            ensure_owned_workspace(&self.workspaces, &workspace)?;
            self.git(
                "workspace checkout",
                Some(&workspace),
                [
                    OsString::from("checkout"),
                    OsString::from("--detach"),
                    commit.clone().into(),
                ],
            )
            .await?;
            let branch = format!("codex-runs/{workspace_id}");
            self.git(
                "workspace branch",
                Some(&workspace),
                [
                    OsString::from("switch"),
                    OsString::from("-c"),
                    branch.clone().into(),
                ],
            )
            .await?;
            self.git(
                "workspace remote",
                Some(&workspace),
                [
                    OsString::from("remote"),
                    OsString::from("set-url"),
                    OsString::from("origin"),
                    OsString::from(source.as_str()),
                ],
            )
            .await?;
            Ok(WorkspaceCheckout {
                workspace_id,
                head_commit: commit,
                branch,
                root: workspace.clone(),
            })
        }
        .await;

        if result.is_err() {
            remove_internal_dir(&self.workspaces, &workspace, "failed workspace cleanup")?;
        }
        result
    }

    pub async fn status(&self, workspace_id: Uuid) -> Result<WorkspaceStatus, GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let status = self
            .git_output(
                "workspace status",
                Some(&workspace),
                [
                    OsString::from("status"),
                    OsString::from("--porcelain=v1"),
                    OsString::from("-z"),
                    OsString::from("--untracked-files=all"),
                ],
                &[0],
            )
            .await?;
        let mut changes = parse_porcelain(&status.stdout)?;
        let numstat = self
            .git_output(
                "workspace diff",
                Some(&workspace),
                [
                    OsString::from("diff"),
                    OsString::from("--numstat"),
                    OsString::from("-z"),
                    OsString::from("HEAD"),
                    OsString::from("--"),
                ],
                &[0],
            )
            .await?;
        apply_numstat(&mut changes, &numstat.stdout)?;
        for change in &mut changes {
            change.size_bytes = safe_file_size(&workspace, &change.path)?;
            change.large = change
                .size_bytes
                .is_some_and(|size| size > LARGE_FILE_BYTES);
        }
        let branch = self
            .git_text(
                "workspace branch",
                Some(&workspace),
                ["branch", "--show-current"],
            )
            .await?;
        let head_commit = self
            .git_text("workspace head", Some(&workspace), ["rev-parse", "HEAD"])
            .await?;
        Ok(WorkspaceStatus {
            branch,
            head_commit,
            changes,
        })
    }

    pub async fn commit_selected(
        &self,
        workspace_id: Uuid,
        selected_paths: &[String],
        message: &str,
        author: &CommitAuthor,
    ) -> Result<String, GitRuntimeError> {
        validate_commit_input(selected_paths, message, author)?;
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let selected = selected_paths.iter().cloned().collect::<BTreeSet<_>>();
        let available = self
            .git_output(
                "read workspace changes",
                Some(&workspace),
                [
                    OsString::from("status"),
                    OsString::from("--porcelain=v1"),
                    OsString::from("-z"),
                    OsString::from("--untracked-files=all"),
                ],
                &[0],
            )
            .await?;
        let available = parse_porcelain(&available.stdout)?
            .into_iter()
            .map(|change| change.path)
            .collect::<BTreeSet<_>>();
        if !selected.is_subset(&available) {
            return Err(GitRuntimeError::Conflict(
                "selection contains paths that are not exact workspace changes".to_string(),
            ));
        }
        let before = self.staged_paths(&workspace).await?;
        if !before.is_subset(&selected) {
            return Err(GitRuntimeError::Conflict(
                "workspace contains staged paths outside the selection".to_string(),
            ));
        }
        let mut add_args = vec![OsString::from("add"), OsString::from("--")];
        add_args.extend(selected_paths.iter().map(OsString::from));
        self.git("stage selected files", Some(&workspace), add_args)
            .await?;
        let staged = self.staged_paths(&workspace).await?;
        if staged.is_empty() {
            return Err(GitRuntimeError::NoChanges);
        }
        if !staged.is_subset(&selected) {
            return Err(GitRuntimeError::Conflict(
                "staged paths changed while preparing the commit".to_string(),
            ));
        }
        self.git(
            "commit selected files",
            Some(&workspace),
            [
                OsString::from("-c"),
                OsString::from(format!("user.name={}", author.name)),
                OsString::from("-c"),
                OsString::from(format!("user.email={}", author.email)),
                OsString::from("commit"),
                OsString::from("--no-gpg-sign"),
                OsString::from("-m"),
                OsString::from(message),
            ],
        )
        .await?;
        self.git_text("read commit", Some(&workspace), ["rev-parse", "HEAD"])
            .await
    }

    /// List tracked and untracked (but not ignored) files using only validated,
    /// workspace-relative paths.
    pub async fn list_files(&self, workspace_id: Uuid) -> Result<Vec<String>, GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let output = self
            .git_output(
                "list workspace files",
                Some(&workspace),
                [
                    OsString::from("ls-files"),
                    OsString::from("-z"),
                    OsString::from("--cached"),
                    OsString::from("--others"),
                    OsString::from("--exclude-standard"),
                    OsString::from("--"),
                ],
                &[0],
            )
            .await?;
        let mut files = nul_strings(&output.stdout)?;
        files.sort();
        files.dedup();
        if files.len() > MAX_WORKSPACE_FILES {
            files.truncate(MAX_WORKSPACE_FILES);
        }
        Ok(files)
    }

    /// Read a regular UTF-8 file from a Run workspace without following a
    /// symlink outside the authorized checkout.
    pub async fn read_file(
        &self,
        workspace_id: Uuid,
        relative: &str,
    ) -> Result<WorkspaceFileContent, GitRuntimeError> {
        validate_relative_path(relative)?;
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let path = workspace.join(relative);
        let metadata =
            tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|source| GitRuntimeError::Io {
                    operation: "inspect workspace file",
                    source,
                })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(GitRuntimeError::UnsafePath(
                "workspace path is not a regular file".to_string(),
            ));
        }
        let canonical =
            tokio::fs::canonicalize(&path)
                .await
                .map_err(|source| GitRuntimeError::Io {
                    operation: "resolve workspace file",
                    source,
                })?;
        if !canonical.starts_with(&workspace) {
            return Err(GitRuntimeError::UnsafePath(
                "workspace file escaped through a symlink".to_string(),
            ));
        }
        let truncated = metadata.len() > MAX_FILE_READ_BYTES;
        let file =
            tokio::fs::File::open(&canonical)
                .await
                .map_err(|source| GitRuntimeError::Io {
                    operation: "open workspace file",
                    source,
                })?;
        let mut bytes = Vec::with_capacity(metadata.len().min(MAX_FILE_READ_BYTES) as usize);
        file.take(MAX_FILE_READ_BYTES)
            .read_to_end(&mut bytes)
            .await
            .map_err(|source| GitRuntimeError::Io {
                operation: "read workspace file",
                source,
            })?;
        let content = String::from_utf8(bytes).map_err(|_| {
            GitRuntimeError::Conflict("workspace file is not UTF-8 text".to_string())
        })?;
        Ok(WorkspaceFileContent { content, truncated })
    }

    /// Read a bounded image from a Run workspace. The caller supplies only a
    /// validated workspace-relative path; the registered workspace remains
    /// the authority for the filesystem root.
    pub async fn read_image_asset(
        &self,
        workspace_id: Uuid,
        relative: &str,
    ) -> Result<WorkspaceImageAsset, GitRuntimeError> {
        validate_relative_path(relative)?;
        let media_type = image_media_type(relative)?;
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let path = workspace.join(relative);
        let metadata =
            tokio::fs::symlink_metadata(&path)
                .await
                .map_err(|source| GitRuntimeError::Io {
                    operation: "inspect workspace image",
                    source,
                })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(GitRuntimeError::UnsafePath(
                "workspace image is not a regular file".to_string(),
            ));
        }
        if metadata.len() > MAX_IMAGE_ASSET_BYTES {
            return Err(GitRuntimeError::ImageTooLarge);
        }
        let canonical =
            tokio::fs::canonicalize(&path)
                .await
                .map_err(|source| GitRuntimeError::Io {
                    operation: "resolve workspace image",
                    source,
                })?;
        if !canonical.starts_with(&workspace) {
            return Err(GitRuntimeError::UnsafePath(
                "workspace image escaped through a symlink".to_string(),
            ));
        }
        let file =
            tokio::fs::File::open(&canonical)
                .await
                .map_err(|source| GitRuntimeError::Io {
                    operation: "open workspace image",
                    source,
                })?;
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        file.take(MAX_IMAGE_ASSET_BYTES + 1)
            .read_to_end(&mut bytes)
            .await
            .map_err(|source| GitRuntimeError::Io {
                operation: "read workspace image",
                source,
            })?;
        if bytes.len() as u64 > MAX_IMAGE_ASSET_BYTES {
            return Err(GitRuntimeError::ImageTooLarge);
        }
        validate_image_bytes(media_type, &bytes)?;
        Ok(WorkspaceImageAsset { bytes, media_type })
    }

    /// Return bounded unified diffs for the exact paths reported by Git
    /// status. Untracked files are compared with /dev/null.
    pub async fn diffs(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<WorkspaceFileDiff>, GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let changes = self.changed_paths(&workspace).await?;
        let mut diffs = Vec::with_capacity(changes.len());
        for change in changes {
            let untracked = change.status == "??";
            let output = if untracked {
                self.git_output(
                    "read untracked workspace diff",
                    Some(&workspace),
                    [
                        OsString::from("diff"),
                        OsString::from("--no-index"),
                        OsString::from("--no-ext-diff"),
                        OsString::from("--unified=3"),
                        OsString::from("--"),
                        OsString::from(null_device()),
                        OsString::from(&change.path),
                    ],
                    &[0, 1],
                )
                .await?
            } else {
                self.git_output(
                    "read workspace diff",
                    Some(&workspace),
                    [
                        OsString::from("diff"),
                        OsString::from("--no-ext-diff"),
                        OsString::from("--unified=3"),
                        OsString::from("HEAD"),
                        OsString::from("--"),
                        OsString::from(&change.path),
                    ],
                    &[0],
                )
                .await?
            };
            let truncated = output.stdout.len() > MAX_DIFF_BYTES;
            let bytes = &output.stdout[..output.stdout.len().min(MAX_DIFF_BYTES)];
            let diff = String::from_utf8_lossy(bytes).into_owned();
            diffs.push(WorkspaceFileDiff {
                path: change.path,
                diff,
                is_binary: change.binary,
                truncated,
            });
        }
        Ok(diffs)
    }

    pub async fn stage_paths(
        &self,
        workspace_id: Uuid,
        paths: &[String],
    ) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        self.require_exact_changes(&workspace, paths).await?;
        let mut args = vec![OsString::from("add"), OsString::from("--")];
        args.extend(paths.iter().map(OsString::from));
        self.git("stage workspace paths", Some(&workspace), args)
            .await
    }

    pub async fn stage_all(&self, workspace_id: Uuid) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        if self.changed_paths(&workspace).await?.is_empty() {
            return Err(GitRuntimeError::NoChanges);
        }
        self.git(
            "stage all workspace paths",
            Some(&workspace),
            [
                OsString::from("add"),
                OsString::from("--all"),
                OsString::from("--"),
            ],
        )
        .await
    }

    pub async fn unstage_paths(
        &self,
        workspace_id: Uuid,
        paths: &[String],
    ) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        self.require_exact_changes(&workspace, paths).await?;
        let staged = self.staged_paths(&workspace).await?;
        if !paths.iter().all(|path| staged.contains(path)) {
            return Err(GitRuntimeError::Conflict(
                "selection contains paths that are not staged".to_string(),
            ));
        }
        let mut args = vec![
            OsString::from("restore"),
            OsString::from("--staged"),
            OsString::from("--"),
        ];
        args.extend(paths.iter().map(OsString::from));
        self.git("unstage workspace paths", Some(&workspace), args)
            .await
    }

    pub async fn revert_paths(
        &self,
        workspace_id: Uuid,
        paths: &[String],
    ) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let changes = self.require_exact_changes(&workspace, paths).await?;
        let untracked = changes
            .iter()
            .filter(|change| change.status == "??")
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        let tracked = changes
            .iter()
            .filter(|change| change.status != "??")
            .map(|change| change.path.clone())
            .collect::<Vec<_>>();
        if !tracked.is_empty() {
            let mut args = vec![
                OsString::from("restore"),
                OsString::from("--staged"),
                OsString::from("--worktree"),
                OsString::from("--"),
            ];
            args.extend(tracked.iter().map(OsString::from));
            self.git("revert tracked workspace paths", Some(&workspace), args)
                .await?;
        }
        if !untracked.is_empty() {
            let mut args = vec![
                OsString::from("clean"),
                OsString::from("-f"),
                OsString::from("--"),
            ];
            args.extend(untracked.iter().map(OsString::from));
            self.git("remove untracked workspace paths", Some(&workspace), args)
                .await?;
        }
        Ok(())
    }

    pub async fn revert_all(&self, workspace_id: Uuid) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        if self.changed_paths(&workspace).await?.is_empty() {
            return Err(GitRuntimeError::NoChanges);
        }
        self.git(
            "revert tracked workspace changes",
            Some(&workspace),
            [
                OsString::from("reset"),
                OsString::from("--hard"),
                OsString::from("HEAD"),
            ],
        )
        .await?;
        self.git(
            "remove all untracked workspace paths",
            Some(&workspace),
            [
                OsString::from("clean"),
                OsString::from("-fd"),
                OsString::from("--"),
            ],
        )
        .await
    }

    pub async fn list_branches(
        &self,
        workspace_id: Uuid,
    ) -> Result<Vec<BranchInfo>, GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let output = self
            .git_output(
                "list workspace branches",
                Some(&workspace),
                [
                    OsString::from("for-each-ref"),
                    OsString::from("--sort=-committerdate"),
                    OsString::from("--format=%(refname:short)%00%(committerdate:unix)%00"),
                    OsString::from("refs/heads"),
                ],
                &[0],
            )
            .await?;
        let fields = output
            .stdout
            .split(|byte| *byte == 0)
            .map(|field| String::from_utf8_lossy(field).trim().to_string())
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>();
        let mut branches = Vec::new();
        for pair in fields.chunks(2) {
            if pair.len() != 2 {
                return Err(GitRuntimeError::Git {
                    operation: "list workspace branches",
                    message: "Git returned malformed branch metadata".to_string(),
                });
            }
            branches.push(BranchInfo {
                name: pair[0].clone(),
                last_commit: pair[1].parse().unwrap_or_default(),
            });
        }
        Ok(branches)
    }

    pub async fn checkout_branch(
        &self,
        workspace_id: Uuid,
        name: &str,
    ) -> Result<(), GitRuntimeError> {
        let branch = self.validate_ref(name)?;
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        if !self.changed_paths(&workspace).await?.is_empty() {
            return Err(GitRuntimeError::Conflict(
                "workspace has uncommitted changes".to_string(),
            ));
        }
        self.git(
            "checkout workspace branch",
            Some(&workspace),
            [OsString::from("switch"), OsString::from(branch.as_str())],
        )
        .await
    }

    pub async fn create_branch(
        &self,
        workspace_id: Uuid,
        name: &str,
    ) -> Result<(), GitRuntimeError> {
        let branch = self.validate_ref(name)?;
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        if !self.changed_paths(&workspace).await?.is_empty() {
            return Err(GitRuntimeError::Conflict(
                "workspace has uncommitted changes".to_string(),
            ));
        }
        self.git(
            "create workspace branch",
            Some(&workspace),
            [
                OsString::from("switch"),
                OsString::from("-c"),
                OsString::from(branch.as_str()),
            ],
        )
        .await
    }

    pub async fn switch_or_create_branch(
        &self,
        workspace_id: Uuid,
        name: &str,
    ) -> Result<(), GitRuntimeError> {
        let branch = self.validate_ref(name)?;
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        if !self.changed_paths(&workspace).await?.is_empty() {
            return Err(GitRuntimeError::Conflict(
                "workspace has uncommitted changes".to_string(),
            ));
        }
        let exists = self
            .git_output(
                "inspect workspace branch",
                Some(&workspace),
                [
                    OsString::from("show-ref"),
                    OsString::from("--verify"),
                    OsString::from("--quiet"),
                    OsString::from(format!("refs/heads/{}", branch.as_str())),
                ],
                &[0, 1],
            )
            .await?
            .status
            .success();
        let args = if exists {
            vec![OsString::from("switch"), OsString::from(branch.as_str())]
        } else {
            vec![
                OsString::from("switch"),
                OsString::from("-c"),
                OsString::from(branch.as_str()),
            ]
        };
        self.git("prepare workspace branch", Some(&workspace), args)
            .await
    }

    pub async fn rename_branch(
        &self,
        workspace_id: Uuid,
        name: &str,
    ) -> Result<String, GitRuntimeError> {
        let requested = self.validate_ref(name)?;
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let current = self
            .git_text(
                "read current workspace branch",
                Some(&workspace),
                ["branch", "--show-current"],
            )
            .await?;
        if current == requested.as_str() {
            return Err(GitRuntimeError::Conflict(
                "branch name is unchanged".to_string(),
            ));
        }
        let mut final_name = requested.as_str().to_string();
        for suffix in 1..=999 {
            let exists = self
                .git_output(
                    "inspect renamed workspace branch",
                    Some(&workspace),
                    [
                        OsString::from("show-ref"),
                        OsString::from("--verify"),
                        OsString::from("--quiet"),
                        OsString::from(format!("refs/heads/{final_name}")),
                    ],
                    &[0, 1],
                )
                .await?
                .status
                .success();
            if !exists {
                break;
            }
            final_name = format!("{}-{}", requested.as_str(), suffix + 1);
            self.validate_ref(&final_name)?;
            if suffix == 999 {
                return Err(GitRuntimeError::Conflict(
                    "could not allocate a unique branch name".to_string(),
                ));
            }
        }
        self.git(
            "rename workspace branch",
            Some(&workspace),
            [
                OsString::from("branch"),
                OsString::from("-m"),
                OsString::from(&final_name),
            ],
        )
        .await?;
        Ok(final_name)
    }

    pub async fn commit_diffs(
        &self,
        workspace_id: Uuid,
        sha: &str,
    ) -> Result<Vec<CommitDiff>, GitRuntimeError> {
        let sha = validate_commit_sha(sha)?;
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let revision = format!("{sha}^{{commit}}");
        let commit = self
            .git_text(
                "resolve commit diff",
                Some(&workspace),
                ["rev-parse", "--verify", revision.as_str()],
            )
            .await?;
        let names = self
            .git_output(
                "list commit diff paths",
                Some(&workspace),
                [
                    OsString::from("diff-tree"),
                    OsString::from("--root"),
                    OsString::from("--no-commit-id"),
                    OsString::from("--name-status"),
                    OsString::from("--find-renames"),
                    OsString::from("-r"),
                    OsString::from("-z"),
                    OsString::from(&commit),
                    OsString::from("--"),
                ],
                &[0],
            )
            .await?;
        let fields = names
            .stdout
            .split(|byte| *byte == 0)
            .filter(|field| !field.is_empty())
            .map(|field| String::from_utf8_lossy(field).to_string())
            .collect::<Vec<_>>();
        let mut index = 0;
        let mut paths = Vec::new();
        while index < fields.len() {
            let status = fields[index].clone();
            index += 1;
            let Some(first_path) = fields.get(index).cloned() else {
                return Err(GitRuntimeError::Git {
                    operation: "list commit diff paths",
                    message: "Git returned malformed commit paths".to_string(),
                });
            };
            index += 1;
            let path = if status.starts_with('R') || status.starts_with('C') {
                let Some(next_path) = fields.get(index).cloned() else {
                    return Err(GitRuntimeError::Git {
                        operation: "list commit diff paths",
                        message: "Git returned malformed renamed commit paths".to_string(),
                    });
                };
                index += 1;
                next_path
            } else {
                first_path
            };
            validate_relative_path(&path)?;
            paths.push((status, path));
            if paths.len() >= MAX_WORKSPACE_FILES {
                break;
            }
        }

        let mut diffs = Vec::with_capacity(paths.len());
        for (status, path) in paths {
            let output = self
                .git_output(
                    "read commit file diff",
                    Some(&workspace),
                    [
                        OsString::from("show"),
                        OsString::from("--format="),
                        OsString::from("--no-ext-diff"),
                        OsString::from("--find-renames"),
                        OsString::from("--unified=3"),
                        OsString::from(&commit),
                        OsString::from("--"),
                        OsString::from(&path),
                    ],
                    &[0],
                )
                .await?;
            let truncated = output.stdout.len() > MAX_DIFF_BYTES;
            let bytes = &output.stdout[..output.stdout.len().min(MAX_DIFF_BYTES)];
            let diff = String::from_utf8_lossy(bytes).into_owned();
            let is_binary = diff.contains("GIT binary patch") || diff.contains("Binary files ");
            diffs.push(CommitDiff {
                path,
                status: status.chars().next().unwrap_or('M').to_string(),
                diff: if truncated {
                    format!("{diff}\n[diff truncated]\n")
                } else {
                    diff
                },
                is_binary,
            });
        }
        Ok(diffs)
    }

    pub async fn copy_agents_md(
        &self,
        source_workspace_id: Uuid,
        target_workspace_id: Uuid,
    ) -> Result<bool, GitRuntimeError> {
        let (_first, _second) = self
            .acquire_workspace_pair(source_workspace_id, target_workspace_id)
            .await;
        let source = self.require_workspace(source_workspace_id)?;
        let target = self.require_workspace(target_workspace_id)?;
        let source_file = source.join("AGENTS.md");
        if !source_file.is_file() {
            return Ok(false);
        }
        let target_file = target.join("AGENTS.md");
        if target_file.exists() {
            return Ok(false);
        }
        let metadata = std::fs::metadata(&source_file).map_err(|source| GitRuntimeError::Io {
            operation: "inspect parent AGENTS.md",
            source,
        })?;
        if metadata.len() > MAX_FILE_READ_BYTES {
            return Err(GitRuntimeError::Conflict(
                "parent AGENTS.md exceeds the workspace file limit".to_string(),
            ));
        }
        let temporary = target.join(format!(".AGENTS.md.{}.tmp", Uuid::now_v7()));
        std::fs::copy(&source_file, &temporary).map_err(|source| GitRuntimeError::Io {
            operation: "copy parent AGENTS.md",
            source,
        })?;
        if let Err(source) = std::fs::rename(&temporary, &target_file) {
            let _ = std::fs::remove_file(&temporary);
            return Err(GitRuntimeError::Io {
                operation: "publish copied AGENTS.md",
                source,
            });
        }
        Ok(true)
    }

    pub async fn write_agents_md(
        &self,
        workspace_id: Uuid,
        content: &str,
    ) -> Result<(), GitRuntimeError> {
        if content.len() > MAX_FILE_READ_BYTES as usize {
            return Err(GitRuntimeError::Conflict(
                "AGENTS.md exceeds the workspace file limit".to_string(),
            ));
        }
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let target = workspace.join("AGENTS.md");
        match tokio::fs::symlink_metadata(&target).await {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(GitRuntimeError::UnsafePath(
                    "AGENTS.md is not a regular workspace file".to_string(),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => {
                return Err(GitRuntimeError::Io {
                    operation: "inspect workspace AGENTS.md",
                    source,
                });
            }
        }
        let temporary = workspace.join(format!(".AGENTS.md.{}.tmp", Uuid::now_v7()));
        tokio::fs::write(&temporary, content)
            .await
            .map_err(|source| GitRuntimeError::Io {
                operation: "write workspace AGENTS.md",
                source,
            })?;
        if let Err(source) = tokio::fs::rename(&temporary, &target).await {
            let _ = tokio::fs::remove_file(&temporary).await;
            return Err(GitRuntimeError::Io {
                operation: "publish workspace AGENTS.md",
                source,
            });
        }
        Ok(())
    }

    pub async fn apply_workspace_changes(
        &self,
        source_workspace_id: Uuid,
        target_workspace_id: Uuid,
    ) -> Result<(), GitRuntimeError> {
        if source_workspace_id == target_workspace_id {
            return Err(GitRuntimeError::Conflict(
                "source and target workspaces must differ".to_string(),
            ));
        }
        let (_first, _second) = self
            .acquire_workspace_pair(source_workspace_id, target_workspace_id)
            .await;
        let source = self.require_workspace(source_workspace_id)?;
        let target = self.require_workspace(target_workspace_id)?;
        if !self.changed_paths(&target).await?.is_empty() {
            return Err(GitRuntimeError::Conflict(
                "parent workspace has uncommitted changes".to_string(),
            ));
        }

        let mut patch = Vec::new();
        for args in [
            vec!["diff", "--binary", "--no-color", "--cached", "--"],
            vec!["diff", "--binary", "--no-color", "--"],
        ] {
            let output = self
                .git_output(
                    "build workspace patch",
                    Some(&source),
                    args.into_iter().map(OsString::from),
                    &[0],
                )
                .await?;
            append_bounded_patch(&mut patch, &output.stdout)?;
        }
        let untracked = self
            .git_output(
                "list untracked workspace paths",
                Some(&source),
                [
                    OsString::from("ls-files"),
                    OsString::from("--others"),
                    OsString::from("--exclude-standard"),
                    OsString::from("-z"),
                ],
                &[0],
            )
            .await?;
        for path in nul_strings(&untracked.stdout)? {
            validate_relative_path(&path)?;
            let output = self
                .git_output(
                    "build untracked workspace patch",
                    Some(&source),
                    [
                        OsString::from("diff"),
                        OsString::from("--binary"),
                        OsString::from("--no-color"),
                        OsString::from("--no-index"),
                        OsString::from("--"),
                        OsString::from(null_device()),
                        OsString::from(path),
                    ],
                    &[0, 1],
                )
                .await?;
            append_bounded_patch(&mut patch, &output.stdout)?;
        }
        if patch.iter().all(u8::is_ascii_whitespace) {
            return Err(GitRuntimeError::NoChanges);
        }

        let mut command = Command::new(&self.config.git_bin);
        command
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(["apply", "--3way", "--whitespace=nowarn", "-"])
            .current_dir(&target)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_GLOBAL", null_device())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().map_err(|source| GitRuntimeError::Io {
            operation: "apply workspace patch",
            source,
        })?;
        child
            .stdin
            .take()
            .ok_or_else(|| {
                GitRuntimeError::Conflict("Git patch input was unavailable".to_string())
            })?
            .write_all(&patch)
            .await
            .map_err(|source| GitRuntimeError::Io {
                operation: "write workspace patch",
                source,
            })?;
        let output = child
            .wait_with_output()
            .await
            .map_err(|source| GitRuntimeError::Io {
                operation: "apply workspace patch",
                source,
            })?;
        if output.status.success() {
            Ok(())
        } else {
            Err(git_failure(
                "apply workspace patch",
                &output.status,
                &output.stderr,
            ))
        }
    }

    pub async fn rename_upstream_branch(
        &self,
        workspace_id: Uuid,
        old_name: &str,
        new_name: &str,
    ) -> Result<(), GitRuntimeError> {
        let old_branch = self.validate_ref(old_name)?;
        let new_branch = self.validate_ref(new_name)?;
        if old_branch.as_str() == new_branch.as_str() {
            return Err(GitRuntimeError::Conflict(
                "branch name is unchanged".to_string(),
            ));
        }
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let current = self
            .git_text(
                "read current workspace branch",
                Some(&workspace),
                ["branch", "--show-current"],
            )
            .await?;
        if current != new_branch.as_str() {
            return Err(GitRuntimeError::Conflict(
                "local workspace branch changed before its upstream was renamed".to_string(),
            ));
        }
        let upstream = self
            .git_output(
                "read workspace upstream remote",
                Some(&workspace),
                [
                    OsString::from("for-each-ref"),
                    OsString::from("--format=%(upstream:remotename)%00%(upstream:remoteref)"),
                    OsString::from(format!("refs/heads/{}", new_branch.as_str())),
                ],
                &[0],
            )
            .await?;
        let fields = upstream
            .stdout
            .split(|byte| *byte == 0)
            .map(|field| String::from_utf8_lossy(field).trim().to_string())
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>();
        let remote = fields.first().map(String::as_str).unwrap_or("origin");
        let old_remote_ref = fields.get(1).map(String::as_str);
        let new_exists = self
            .git_output(
                "inspect renamed remote branch",
                Some(&workspace),
                [
                    OsString::from("ls-remote"),
                    OsString::from("--exit-code"),
                    OsString::from("--heads"),
                    OsString::from(remote),
                    OsString::from(new_branch.as_str()),
                ],
                &[0, 2],
            )
            .await?
            .status
            .success();
        if new_exists {
            return Err(GitRuntimeError::Conflict(
                "remote branch already exists".to_string(),
            ));
        }
        self.git(
            "publish renamed remote branch",
            Some(&workspace),
            [
                OsString::from("push"),
                OsString::from(remote),
                OsString::from(format!("{}:{}", new_branch.as_str(), new_branch.as_str())),
            ],
        )
        .await?;
        if old_remote_ref == Some(format!("refs/heads/{}", old_branch.as_str()).as_str()) {
            self.git(
                "delete previous remote branch",
                Some(&workspace),
                [
                    OsString::from("push"),
                    OsString::from(remote),
                    OsString::from(format!(":{}", old_branch.as_str())),
                ],
            )
            .await?;
        }
        self.git(
            "set renamed branch upstream",
            Some(&workspace),
            [
                OsString::from("branch"),
                OsString::from("--set-upstream-to"),
                OsString::from(format!("{}/{}", remote, new_branch.as_str())),
                OsString::from(new_branch.as_str()),
            ],
        )
        .await
    }

    pub async fn log(&self, workspace_id: Uuid, limit: usize) -> Result<GitLog, GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let limit = limit.clamp(1, 200);
        let entries = self.git_log_entries(&workspace, "HEAD", limit).await?;
        let total = self
            .git_text(
                "count workspace commits",
                Some(&workspace),
                ["rev-list", "--count", "HEAD"],
            )
            .await?
            .parse()
            .unwrap_or_default();
        let upstream_output = self
            .git_output(
                "read workspace upstream",
                Some(&workspace),
                [
                    OsString::from("rev-parse"),
                    OsString::from("--abbrev-ref"),
                    OsString::from("--symbolic-full-name"),
                    OsString::from("@{upstream}"),
                ],
                &[0, 128],
            )
            .await?;
        let upstream = if upstream_output.status.success() {
            Some(
                String::from_utf8_lossy(&upstream_output.stdout)
                    .trim()
                    .to_string(),
            )
            .filter(|value| !value.is_empty())
        } else {
            None
        };
        let (ahead, behind, ahead_entries, behind_entries) = if upstream.is_some() {
            let counts = self
                .git_text(
                    "count workspace divergence",
                    Some(&workspace),
                    ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
                )
                .await?;
            let values = counts
                .split_whitespace()
                .map(|value| value.parse::<u64>().unwrap_or_default())
                .collect::<Vec<_>>();
            (
                values.first().copied().unwrap_or_default(),
                values.get(1).copied().unwrap_or_default(),
                self.git_log_entries(&workspace, "@{upstream}..HEAD", limit)
                    .await?,
                self.git_log_entries(&workspace, "HEAD..@{upstream}", limit)
                    .await?,
            )
        } else {
            (0, 0, Vec::new(), Vec::new())
        };
        Ok(GitLog {
            total,
            entries,
            ahead,
            behind,
            ahead_entries,
            behind_entries,
            upstream,
        })
    }

    pub async fn remote(&self, workspace_id: Uuid) -> Result<Option<String>, GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        let output = self
            .git_output(
                "read workspace remote",
                Some(&workspace),
                [
                    OsString::from("remote"),
                    OsString::from("get-url"),
                    OsString::from("origin"),
                ],
                &[0, 2],
            )
            .await?;
        if !output.status.success() {
            return Ok(None);
        }
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok((!value.is_empty()).then_some(value))
    }

    pub async fn fetch(&self, workspace_id: Uuid) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        self.git(
            "fetch workspace remote",
            Some(&workspace),
            [
                OsString::from("fetch"),
                OsString::from("--prune"),
                OsString::from("origin"),
            ],
        )
        .await
    }

    pub async fn pull(&self, workspace_id: Uuid) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        if !self.changed_paths(&workspace).await?.is_empty() {
            return Err(GitRuntimeError::Conflict(
                "workspace has uncommitted changes".to_string(),
            ));
        }
        self.git(
            "pull workspace remote",
            Some(&workspace),
            [OsString::from("pull"), OsString::from("--ff-only")],
        )
        .await
    }

    pub async fn push(&self, workspace_id: Uuid) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        self.git(
            "push workspace remote",
            Some(&workspace),
            [
                OsString::from("push"),
                OsString::from("--set-upstream"),
                OsString::from("origin"),
                OsString::from("HEAD"),
            ],
        )
        .await
    }

    pub async fn sync(&self, workspace_id: Uuid) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        let workspace = self.require_workspace(workspace_id)?;
        if !self.changed_paths(&workspace).await?.is_empty() {
            return Err(GitRuntimeError::Conflict(
                "workspace has uncommitted changes".to_string(),
            ));
        }
        self.git(
            "pull workspace remote",
            Some(&workspace),
            [OsString::from("pull"), OsString::from("--ff-only")],
        )
        .await?;
        self.git(
            "push workspace remote",
            Some(&workspace),
            [
                OsString::from("push"),
                OsString::from("--set-upstream"),
                OsString::from("origin"),
                OsString::from("HEAD"),
            ],
        )
        .await
    }

    pub async fn remove_workspace(&self, workspace_id: Uuid) -> Result<(), GitRuntimeError> {
        let _lock = self.acquire_workspace_lock(workspace_id).await;
        self.scoped_roots
            .lock()
            .map_err(|_| {
                GitRuntimeError::Conflict("workspace Git root lock is poisoned".to_string())
            })?
            .remove(&workspace_id);
        let workspace = self.workspace_path(workspace_id);
        remove_internal_dir(&self.workspaces, &workspace, "workspace removal")
    }

    async fn ensure_mirror(
        &self,
        project_id: Uuid,
        workspace_id: Uuid,
        source: &ValidatedGitSource,
    ) -> Result<PathBuf, GitRuntimeError> {
        let mirror = self.mirrors.join(format!("{project_id}.git"));
        reject_symlink(&mirror, "repository mirror")?;
        if let Some(managed_project_id) = source.managed_project_id() {
            if managed_project_id != project_id {
                return Err(GitRuntimeError::InvalidSource(
                    "managed source does not match its project".to_string(),
                ));
            }
            if !mirror.exists() {
                self.initialize_managed_mirror(project_id, workspace_id, &mirror)
                    .await?;
            }
            ensure_owned_workspace(&self.mirrors, &mirror)?;
            return Ok(mirror);
        }
        if !mirror.exists() {
            let temporary = self
                .mirrors
                .join(format!(".{project_id}-{workspace_id}.tmp"));
            remove_internal_dir(&self.mirrors, &temporary, "stale mirror cleanup")?;
            let clone = self
                .git(
                    "mirror clone",
                    None,
                    [
                        OsString::from("clone"),
                        OsString::from("--mirror"),
                        OsString::from("--no-tags"),
                        OsString::from("--"),
                        OsString::from(source.as_str()),
                        temporary.as_os_str().to_owned(),
                    ],
                )
                .await;
            if let Err(error) = clone {
                remove_internal_dir(&self.mirrors, &temporary, "failed mirror cleanup")?;
                return Err(error);
            }
            std::fs::rename(&temporary, &mirror).map_err(|source| GitRuntimeError::Io {
                operation: "publish repository mirror",
                source,
            })?;
        }
        ensure_owned_workspace(&self.mirrors, &mirror)?;
        self.git(
            "mirror remote",
            None,
            [
                OsString::from("--git-dir"),
                mirror.as_os_str().to_owned(),
                OsString::from("remote"),
                OsString::from("set-url"),
                OsString::from("origin"),
                OsString::from(source.as_str()),
            ],
        )
        .await?;
        self.git(
            "mirror fetch",
            None,
            [
                OsString::from("--git-dir"),
                mirror.as_os_str().to_owned(),
                OsString::from("fetch"),
                OsString::from("--prune"),
                OsString::from("--no-tags"),
                OsString::from("origin"),
                OsString::from("+refs/heads/*:refs/heads/*"),
            ],
        )
        .await?;
        Ok(mirror)
    }

    async fn initialize_managed_mirror(
        &self,
        project_id: Uuid,
        workspace_id: Uuid,
        mirror: &Path,
    ) -> Result<(), GitRuntimeError> {
        let seed = self
            .mirrors
            .join(format!(".{project_id}-{workspace_id}.seed"));
        let temporary = self
            .mirrors
            .join(format!(".{project_id}-{workspace_id}.managed.tmp"));
        remove_internal_dir(&self.mirrors, &seed, "stale managed seed cleanup")?;
        remove_internal_dir(&self.mirrors, &temporary, "stale managed mirror cleanup")?;

        let result = async {
            self.git(
                "managed source init",
                None,
                [
                    OsString::from("init"),
                    OsString::from("--initial-branch=main"),
                    seed.as_os_str().to_owned(),
                ],
            )
            .await?;
            self.git(
                "managed source initial commit",
                Some(&seed),
                [
                    OsString::from("-c"),
                    OsString::from("user.name=Open Web Codex"),
                    OsString::from("-c"),
                    OsString::from("user.email=codex@localhost"),
                    OsString::from("commit"),
                    OsString::from("--allow-empty"),
                    OsString::from("-m"),
                    OsString::from("Initial commit"),
                ],
            )
            .await?;
            self.git(
                "managed mirror clone",
                None,
                [
                    OsString::from("clone"),
                    OsString::from("--mirror"),
                    OsString::from("--no-tags"),
                    OsString::from("--"),
                    seed.as_os_str().to_owned(),
                    temporary.as_os_str().to_owned(),
                ],
            )
            .await?;
            std::fs::rename(&temporary, mirror).map_err(|source| GitRuntimeError::Io {
                operation: "publish managed mirror",
                source,
            })?;
            Ok(())
        }
        .await;

        let seed_cleanup = remove_internal_dir(&self.mirrors, &seed, "managed seed cleanup");
        if result.is_err() {
            let _ = remove_internal_dir(&self.mirrors, &temporary, "failed managed mirror cleanup");
        }
        result?;
        seed_cleanup?;
        Ok(())
    }

    async fn resolve_commit(
        &self,
        mirror: &Path,
        git_ref: &ValidatedGitRef,
    ) -> Result<String, GitRuntimeError> {
        self.git_text_os(
            "resolve Git ref",
            None,
            [
                OsString::from("--git-dir"),
                mirror.as_os_str().to_owned(),
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from(format!("refs/heads/{}^{{commit}}", git_ref.as_str())),
            ],
        )
        .await
    }

    fn require_base_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, GitRuntimeError> {
        let workspace = self.workspace_path(workspace_id);
        ensure_owned_workspace(&self.workspaces, &workspace)?;
        if !workspace.join(".git").exists() {
            return Err(GitRuntimeError::Conflict(
                "workspace is not a Git checkout".to_string(),
            ));
        }
        Ok(workspace)
    }

    fn require_workspace(&self, workspace_id: Uuid) -> Result<PathBuf, GitRuntimeError> {
        let workspace = self.require_base_workspace(workspace_id)?;
        let selected = self
            .scoped_roots
            .lock()
            .map_err(|_| {
                GitRuntimeError::Conflict("workspace Git root lock is poisoned".to_string())
            })?
            .get(&workspace_id)
            .cloned();
        let Some(selected) = selected else {
            return Ok(workspace);
        };
        if !selected.starts_with(&workspace) || !git_marker_is_safe(&selected) {
            return Err(GitRuntimeError::UnsafePath(
                "selected Git root is no longer a safe workspace repository".to_string(),
            ));
        }
        Ok(selected)
    }

    async fn staged_paths(&self, workspace: &Path) -> Result<BTreeSet<String>, GitRuntimeError> {
        let output = self
            .git_output(
                "read staged paths",
                Some(workspace),
                [
                    OsString::from("diff"),
                    OsString::from("--cached"),
                    OsString::from("--name-only"),
                    OsString::from("-z"),
                    OsString::from("--"),
                ],
                &[0],
            )
            .await?;
        nul_strings(&output.stdout).map(|paths| paths.into_iter().collect())
    }

    async fn changed_paths(&self, workspace: &Path) -> Result<Vec<FileChange>, GitRuntimeError> {
        let output = self
            .git_output(
                "read workspace changes",
                Some(workspace),
                [
                    OsString::from("status"),
                    OsString::from("--porcelain=v1"),
                    OsString::from("-z"),
                    OsString::from("--untracked-files=all"),
                ],
                &[0],
            )
            .await?;
        parse_porcelain(&output.stdout)
    }

    async fn require_exact_changes(
        &self,
        workspace: &Path,
        paths: &[String],
    ) -> Result<Vec<FileChange>, GitRuntimeError> {
        if paths.is_empty() {
            return Err(GitRuntimeError::NoChanges);
        }
        for path in paths {
            validate_relative_path(path)?;
        }
        let changes = self.changed_paths(workspace).await?;
        let available = changes
            .iter()
            .map(|change| change.path.as_str())
            .collect::<BTreeSet<_>>();
        if !paths.iter().all(|path| available.contains(path.as_str())) {
            return Err(GitRuntimeError::Conflict(
                "selection contains paths that are not exact workspace changes".to_string(),
            ));
        }
        Ok(changes
            .into_iter()
            .filter(|change| paths.contains(&change.path))
            .collect())
    }

    async fn git_log_entries(
        &self,
        workspace: &Path,
        revision: &str,
        limit: usize,
    ) -> Result<Vec<GitLogEntry>, GitRuntimeError> {
        let output = self
            .git_output(
                "read workspace log",
                Some(workspace),
                [
                    OsString::from("log"),
                    OsString::from("-z"),
                    OsString::from("--format=%H%x00%s%x00%an%x00%at"),
                    OsString::from(format!("--max-count={limit}")),
                    OsString::from(revision),
                    OsString::from("--"),
                ],
                &[0],
            )
            .await?;
        let fields = output
            .stdout
            .split(|byte| *byte == 0)
            .map(|field| String::from_utf8_lossy(field).trim().to_string())
            .filter(|field| !field.is_empty())
            .collect::<Vec<_>>();
        if fields.len() % 4 != 0 {
            return Err(GitRuntimeError::Git {
                operation: "read workspace log",
                message: "Git returned malformed log metadata".to_string(),
            });
        }
        Ok(fields
            .chunks(4)
            .map(|fields| GitLogEntry {
                sha: fields[0].clone(),
                summary: fields[1].clone(),
                author: fields[2].clone(),
                timestamp: fields[3].parse().unwrap_or_default(),
            })
            .collect())
    }

    async fn git<I>(
        &self,
        operation: &'static str,
        cwd: Option<&Path>,
        args: I,
    ) -> Result<(), GitRuntimeError>
    where
        I: IntoIterator<Item = OsString>,
    {
        self.git_output(operation, cwd, args, &[0])
            .await
            .map(|_| ())
    }

    async fn git_text<const N: usize, S: AsRef<OsStr>>(
        &self,
        operation: &'static str,
        cwd: Option<&Path>,
        args: [S; N],
    ) -> Result<String, GitRuntimeError> {
        self.git_text_os(operation, cwd, args.map(|value| value.as_ref().to_owned()))
            .await
    }

    async fn git_text_os<I>(
        &self,
        operation: &'static str,
        cwd: Option<&Path>,
        args: I,
    ) -> Result<String, GitRuntimeError>
    where
        I: IntoIterator<Item = OsString>,
    {
        let output = self.git_output(operation, cwd, args, &[0]).await?;
        String::from_utf8(output.stdout)
            .map(|value| value.trim().to_string())
            .map_err(|_| GitRuntimeError::Git {
                operation,
                message: "Git returned non-UTF-8 output".to_string(),
            })
    }

    async fn git_output<I>(
        &self,
        operation: &'static str,
        cwd: Option<&Path>,
        args: I,
        accepted: &[i32],
    ) -> Result<Output, GitRuntimeError>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut command = Command::new(&self.config.git_bin);
        command
            .args(["-c", "core.hooksPath=/dev/null"])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_GLOBAL", null_device())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("LC_ALL", "C");
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let output = command
            .output()
            .await
            .map_err(|source| GitRuntimeError::Io { operation, source })?;
        let code = output.status.code().unwrap_or(-1);
        if accepted.contains(&code) {
            return Ok(output);
        }
        Err(git_failure(operation, &output.status, &output.stderr))
    }

    async fn acquire_project_lock(&self, project_id: Uuid) -> Result<RuntimeLock, GitRuntimeError> {
        self.acquire_lock(format!("project-{project_id}"), true)
            .await
    }

    async fn acquire_workspace_lock(&self, workspace_id: Uuid) -> RuntimeLock {
        self.acquire_lock(format!("workspace-{workspace_id}"), false)
            .await
            .expect("workspace lock path is server-generated")
    }

    async fn acquire_workspace_pair(
        &self,
        first: Uuid,
        second: Uuid,
    ) -> (RuntimeLock, RuntimeLock) {
        if first.as_u128() < second.as_u128() {
            (
                self.acquire_workspace_lock(first).await,
                self.acquire_workspace_lock(second).await,
            )
        } else {
            (
                self.acquire_workspace_lock(second).await,
                self.acquire_workspace_lock(first).await,
            )
        }
    }

    async fn acquire_lock(
        &self,
        name: String,
        file_lock: bool,
    ) -> Result<RuntimeLock, GitRuntimeError> {
        let lock = {
            let mut locks = self.process_locks.lock().expect("Git lock map poisoned");
            locks
                .entry(name.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let guard = lock.lock_owned().await;
        let file = if file_lock {
            Some(acquire_file_lock(self.locks.join(format!("{name}.lock"))).await?)
        } else {
            None
        };
        Ok(RuntimeLock {
            _guard: guard,
            _file: file,
        })
    }
}

struct RuntimeLock {
    _guard: OwnedMutexGuard<()>,
    _file: Option<std::fs::File>,
}

fn git_failure(operation: &'static str, status: &ExitStatus, stderr: &[u8]) -> GitRuntimeError {
    let message = String::from_utf8_lossy(stderr);
    let message = message.lines().take(4).collect::<Vec<_>>().join(" ");
    GitRuntimeError::Git {
        operation,
        message: if message.is_empty() {
            format!("process exited with {}", status.code().unwrap_or(-1))
        } else {
            message
        },
    }
}

fn append_bounded_patch(target: &mut Vec<u8>, chunk: &[u8]) -> Result<(), GitRuntimeError> {
    if target.len().saturating_add(chunk.len()) > MAX_APPLY_PATCH_BYTES {
        return Err(GitRuntimeError::Conflict(
            "workspace patch exceeds the apply limit".to_string(),
        ));
    }
    target.extend_from_slice(chunk);
    Ok(())
}

fn parse_porcelain(bytes: &[u8]) -> Result<Vec<FileChange>, GitRuntimeError> {
    let records = bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    let mut changes = BTreeMap::new();
    let mut index = 0;
    while index < records.len() {
        let record = std::str::from_utf8(records[index]).map_err(|_| GitRuntimeError::Git {
            operation: "workspace status",
            message: "Git returned a non-UTF-8 path".to_string(),
        })?;
        if record.len() < 4 || record.as_bytes()[2] != b' ' {
            return Err(GitRuntimeError::Git {
                operation: "workspace status",
                message: "Git returned an invalid porcelain record".to_string(),
            });
        }
        let status = record[..2].to_string();
        let path = record[3..].to_string();
        validate_relative_path(&path)?;
        changes.insert(
            path.clone(),
            FileChange {
                path,
                status: status.clone(),
                additions: None,
                deletions: None,
                binary: false,
                size_bytes: None,
                large: false,
            },
        );
        if status.contains('R') || status.contains('C') {
            index += 1;
        }
        index += 1;
    }
    Ok(changes.into_values().collect())
}

fn apply_numstat(changes: &mut [FileChange], bytes: &[u8]) -> Result<(), GitRuntimeError> {
    let mut by_path = changes
        .iter_mut()
        .map(|change| (change.path.clone(), change))
        .collect::<BTreeMap<_, _>>();
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let record = std::str::from_utf8(record).map_err(|_| GitRuntimeError::Git {
            operation: "workspace diff",
            message: "Git returned non-UTF-8 diff metadata".to_string(),
        })?;
        let mut fields = record.splitn(3, '\t');
        let additions = fields.next().unwrap_or_default();
        let deletions = fields.next().unwrap_or_default();
        let path = fields.next().unwrap_or_default();
        if let Some(change) = by_path.get_mut(path) {
            change.binary = additions == "-" || deletions == "-";
            change.additions = additions.parse().ok();
            change.deletions = deletions.parse().ok();
        }
    }
    Ok(())
}

fn nul_strings(bytes: &[u8]) -> Result<Vec<String>, GitRuntimeError> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let value = std::str::from_utf8(record).map_err(|_| GitRuntimeError::Git {
                operation: "read Git paths",
                message: "Git returned a non-UTF-8 path".to_string(),
            })?;
            validate_relative_path(value)?;
            Ok(value.to_string())
        })
        .collect()
}

fn validate_commit_input(
    selected_paths: &[String],
    message: &str,
    author: &CommitAuthor,
) -> Result<(), GitRuntimeError> {
    if selected_paths.is_empty() {
        return Err(GitRuntimeError::NoChanges);
    }
    for path in selected_paths {
        validate_relative_path(path)?;
    }
    if message.trim().is_empty() || message.len() > 10_000 || message.contains('\0') {
        return Err(GitRuntimeError::Conflict(
            "invalid commit message".to_string(),
        ));
    }
    if author.name.trim().is_empty()
        || author.name.contains(['\0', '\n', '\r'])
        || author.email.contains(['\0', '\n', '\r'])
        || !author.email.contains('@')
    {
        return Err(GitRuntimeError::Conflict(
            "invalid commit author".to_string(),
        ));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), GitRuntimeError> {
    if value.is_empty() || value.contains('\0') {
        return Err(GitRuntimeError::UnsafePath("empty or NUL path".to_string()));
    }
    let path = Path::new(value);
    if value.starts_with("./")
        || value.contains("//")
        || path.components().any(|component| {
            matches!(
                component,
                Component::CurDir
                    | Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
            )
        })
    {
        return Err(GitRuntimeError::UnsafePath(
            "path escapes the workspace".to_string(),
        ));
    }
    Ok(())
}

fn image_media_type(relative: &str) -> Result<&'static str, GitRuntimeError> {
    let extension = Path::new(relative)
        .extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase);
    match extension.as_deref() {
        Some("png") => Ok("image/png"),
        Some("jpg" | "jpeg") => Ok("image/jpeg"),
        Some("gif") => Ok("image/gif"),
        Some("webp") => Ok("image/webp"),
        Some("avif") => Ok("image/avif"),
        Some("bmp") => Ok("image/bmp"),
        Some("tif" | "tiff") => Ok("image/tiff"),
        Some("heic") => Ok("image/heic"),
        Some("heif") => Ok("image/heif"),
        Some("svg") => Ok("image/svg+xml"),
        _ => Err(GitRuntimeError::UnsupportedImage(
            "file extension is not an allowed image type".to_string(),
        )),
    }
}

fn validate_image_bytes(media_type: &str, bytes: &[u8]) -> Result<(), GitRuntimeError> {
    let valid = match media_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()),
        "image/avif" => is_iso_base_media_image(bytes, &[b"avif", b"avis"]),
        "image/heic" => is_iso_base_media_image(
            bytes,
            &[b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis"],
        ),
        "image/heif" => is_iso_base_media_image(bytes, &[b"mif1", b"msf1", b"heif"]),
        "image/bmp" => bytes.starts_with(b"BM"),
        "image/tiff" => {
            bytes.starts_with(b"II*\0")
                || bytes.starts_with(b"MM\0*")
                || bytes.starts_with(b"II+\0")
                || bytes.starts_with(b"MM\0+")
        }
        "image/svg+xml" => is_svg_document(bytes),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(GitRuntimeError::UnsupportedImage(
            "file contents do not match the declared image type".to_string(),
        ))
    }
}

fn is_iso_base_media_image(bytes: &[u8], brands: &[&[u8; 4]]) -> bool {
    if bytes.get(4..8) != Some(b"ftyp".as_slice()) {
        return false;
    }
    bytes.get(8..bytes.len().min(64)).is_some_and(|header| {
        brands
            .iter()
            .any(|brand| header.windows(4).any(|item| item == *brand))
    })
}

fn is_svg_document(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let prefix = text
        .trim_start_matches('\u{feff}')
        .trim_start()
        .get(..text.len().min(4096))
        .unwrap_or(text)
        .to_ascii_lowercase();
    let Some(svg_index) = prefix.find("<svg") else {
        return false;
    };
    let preamble = prefix[..svg_index].trim();
    preamble.is_empty()
        || preamble.starts_with("<?xml")
        || preamble.starts_with("<!--")
        || preamble.starts_with("<!doctype svg")
}

fn git_marker_is_safe(repository: &Path) -> bool {
    match std::fs::symlink_metadata(repository.join(".git")) {
        Ok(metadata) => {
            !metadata.file_type().is_symlink() && (metadata.is_dir() || metadata.is_file())
        }
        Err(_) => false,
    }
}

fn scan_nested_git_roots(root: &Path, max_depth: usize, max_results: usize) -> Vec<String> {
    let mut results = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth >= max_depth || results.len() >= max_results {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        let mut children = entries.filter_map(Result::ok).collect::<Vec<_>>();
        children.sort_by_key(|entry| entry.file_name());
        for entry in children.into_iter().rev() {
            let name = entry.file_name();
            if matches!(
                name.to_str(),
                Some(".git" | "node_modules" | "dist" | "target" | "release-artifacts")
            ) {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() || file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if git_marker_is_safe(&path) {
                if let Ok(relative) = path.strip_prefix(root) {
                    results.push(relative.to_string_lossy().replace('\\', "/"));
                    if results.len() >= max_results {
                        break;
                    }
                }
            }
            pending.push((path, depth + 1));
        }
    }
    results.sort();
    results
}

fn safe_file_size(workspace: &Path, relative: &str) -> Result<Option<u64>, GitRuntimeError> {
    validate_relative_path(relative)?;
    let path = workspace.join(relative);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(GitRuntimeError::Io {
                operation: "inspect workspace file",
                source,
            })
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Ok(None);
    }
    let canonical = canonicalize(&path, "workspace file")?;
    if !canonical.starts_with(workspace) {
        return Err(GitRuntimeError::UnsafePath(
            "workspace file escaped through a symlink".to_string(),
        ));
    }
    Ok(Some(metadata.len()))
}

fn reject_existing_workspace(path: &Path) -> Result<(), GitRuntimeError> {
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(GitRuntimeError::Conflict(
            "workspace path already exists".to_string(),
        ));
    }
    Ok(())
}

fn ensure_owned_workspace(parent: &Path, path: &Path) -> Result<(), GitRuntimeError> {
    reject_symlink(path, "workspace")?;
    let canonical = canonicalize(path, "workspace")?;
    if canonical.parent() != Some(parent) {
        return Err(GitRuntimeError::UnsafePath(
            "workspace is outside the runner root".to_string(),
        ));
    }
    Ok(())
}

fn remove_internal_dir(
    parent: &Path,
    path: &Path,
    operation: &'static str,
) -> Result<(), GitRuntimeError> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(GitRuntimeError::Io { operation, source }),
    };
    if metadata.file_type().is_symlink() || path.parent() != Some(parent) {
        return Err(GitRuntimeError::UnsafePath(format!(
            "refused {operation} outside owned root"
        )));
    }
    std::fs::remove_dir_all(path).map_err(|source| GitRuntimeError::Io { operation, source })
}

fn create_owned_child(root: &Path, name: &str) -> Result<PathBuf, GitRuntimeError> {
    let path = root.join(name);
    reject_symlink(&path, "runner directory")?;
    create_private_dir(&path, "runner directory")?;
    let canonical = canonicalize(&path, "runner directory")?;
    if canonical.parent() != Some(root) {
        return Err(GitRuntimeError::UnsafePath(format!(
            "runner {name} directory escaped its root"
        )));
    }
    Ok(canonical)
}

fn create_private_dir(path: &Path, operation: &'static str) -> Result<(), GitRuntimeError> {
    std::fs::create_dir_all(path).map_err(|source| GitRuntimeError::Io { operation, source })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|source| GitRuntimeError::Io { operation, source })?;
    }
    Ok(())
}

fn reject_symlink(path: &Path, operation: &'static str) -> Result<(), GitRuntimeError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(GitRuntimeError::UnsafePath(
            format!("{operation} must not be a symlink"),
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(GitRuntimeError::Io { operation, source }),
    }
}

fn validate_commit_sha(value: &str) -> Result<String, GitRuntimeError> {
    let value = value.trim();
    if !(7..=64).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(GitRuntimeError::InvalidRef(
            "commit id must be a hexadecimal object id".to_string(),
        ));
    }
    Ok(value.to_ascii_lowercase())
}

fn canonicalize(path: &Path, operation: &'static str) -> Result<PathBuf, GitRuntimeError> {
    path.canonicalize()
        .map_err(|source| GitRuntimeError::Io { operation, source })
}

async fn acquire_file_lock(path: PathBuf) -> Result<std::fs::File, GitRuntimeError> {
    tokio::task::spawn_blocking(move || {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|source| GitRuntimeError::Io {
                operation: "open mirror lock",
                source,
            })?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result != 0 {
                return Err(GitRuntimeError::Io {
                    operation: "lock repository mirror",
                    source: std::io::Error::last_os_error(),
                });
            }
        }
        Ok(file)
    })
    .await
    .map_err(|error| GitRuntimeError::Conflict(format!("mirror lock task failed: {error}")))?
}

#[cfg(windows)]
fn null_device() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device() -> &'static str {
    "/dev/null"
}
