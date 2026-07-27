mod execution;
mod scheduler;

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use open_web_codex_adapter::{AdapterError, CodexAdapter};
use open_web_codex_git_runtime::{GitRuntime, GitRuntimeError};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum RunOrchestratorError {
    #[error("invalid Run request: {0}")]
    Invalid(String),
    #[error("Run resource was not found")]
    NotFound,
    #[error("Run request conflicts with current state: {0}")]
    Conflict(String),
    #[error("Run lease is no longer owned by this worker")]
    LeaseLost,
    #[error("database operation failed: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Git workspace operation failed: {0}")]
    Git(#[from] GitRuntimeError),
    #[error("Codex Runtime operation failed: {0}")]
    Adapter(#[from] AdapterError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnqueueRunRequest {
    pub organization_id: Uuid,
    pub actor_id: Uuid,
    pub task_id: Uuid,
    pub idempotency_key: String,
    pub git_ref: Option<String>,
    pub workspace_kind: String,
    pub workspace_name: Option<String>,
    pub workspace_parent_run_id: Option<Uuid>,
    pub workspace_group_run_id: Option<Uuid>,
    pub copy_agents_md: bool,
    pub fork_thread_id: Option<String>,
    pub fork_source_run_id: Option<Uuid>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub id: Uuid,
    pub task_id: Uuid,
    pub status: String,
    pub codex_thread_id: Option<String>,
    pub active_turn_id: Option<String>,
    pub workspace_id: Option<Uuid>,
    pub source_ref: Option<String>,
    pub workspace_kind: String,
    pub workspace_name: Option<String>,
    pub workspace_parent_run_id: Option<Uuid>,
    pub workspace_group_run_id: Option<Uuid>,
    pub attempt: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelRunRequest {
    pub organization_id: Uuid,
    pub actor_id: Uuid,
    pub allow_organization_admin: bool,
    pub run_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverRunRequest {
    pub organization_id: Uuid,
    pub actor_id: Uuid,
    pub allow_organization_admin: bool,
    pub run_id: Uuid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetireWorkspaceRequest {
    pub organization_id: Uuid,
    pub actor_id: Uuid,
    pub allow_organization_admin: bool,
    pub run_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct RunLease {
    pub run_id: Uuid,
    pub task_id: Uuid,
    pub project_id: Uuid,
    pub organization_id: Uuid,
    pub profile_id: Uuid,
    pub git_url: String,
    pub source_ref: String,
    pub workspace_kind: String,
    pub workspace_parent_run_id: Option<Uuid>,
    pub fork_thread_id: Option<String>,
    pub fork_source_run_id: Option<Uuid>,
    pub copy_agents_md: bool,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub attempt: i32,
}

#[derive(Clone)]
pub struct RunOrchestrator {
    pub(crate) db: PgPool,
    pub(crate) git: Arc<GitRuntime>,
    pub(crate) adapter: Arc<dyn CodexAdapter>,
    pub(crate) runtime_key: String,
    pub(crate) worker_id: String,
    pub(crate) lease_ttl: Duration,
}

impl RunOrchestrator {
    pub fn new(
        db: PgPool,
        git: Arc<GitRuntime>,
        adapter: Arc<dyn CodexAdapter>,
        runtime_key: impl Into<String>,
        worker_id: impl Into<String>,
        lease_ttl: Duration,
    ) -> Result<Self, RunOrchestratorError> {
        if lease_ttl < Duration::from_secs(5) {
            return Err(RunOrchestratorError::Invalid(
                "lease TTL must be at least five seconds".to_string(),
            ));
        }
        let worker_id = worker_id.into();
        if worker_id.trim().is_empty() || worker_id.len() > 128 {
            return Err(RunOrchestratorError::Invalid(
                "worker id must contain 1 to 128 characters".to_string(),
            ));
        }
        Ok(Self {
            db,
            git,
            adapter,
            runtime_key: runtime_key.into(),
            worker_id,
            lease_ttl,
        })
    }

    pub async fn run_worker(self: Arc<Self>, mut shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(Duration::from_secs(2));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(error) = self.heartbeat_owned_runs().await {
                        tracing::warn!(%error, "Runner heartbeat failed");
                    }
                    if let Err(error) = self.reap_expired().await {
                        tracing::warn!(%error, "Runner lease recovery failed");
                    }
                    if let Err(error) = self.run_once().await {
                        tracing::warn!(%error, "Runner execution failed");
                    }
                    if let Err(error) = self.run_cleanup_once().await {
                        tracing::warn!(%error, "Runner cleanup failed");
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
    }
}

fn validate_idempotency_key(key: &str) -> Result<(), RunOrchestratorError> {
    if !(8..=128).contains(&key.len())
        || !key
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-._:".contains(character))
    {
        return Err(RunOrchestratorError::Invalid(
            "idempotency key must contain 8 to 128 safe ASCII characters".to_string(),
        ));
    }
    Ok(())
}

fn chrono_ttl(ttl: Duration) -> Result<chrono::Duration, RunOrchestratorError> {
    chrono::Duration::from_std(ttl)
        .map_err(|_| RunOrchestratorError::Invalid("lease TTL is too large".to_string()))
}
