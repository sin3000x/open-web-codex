use chrono::Utc;
use open_web_codex_adapter::AuthorizedWorkspace;
use sqlx::Row;
use uuid::Uuid;

use crate::{RunLease, RunOrchestrator, RunOrchestratorError};

impl RunOrchestrator {
    pub async fn run_once(&self) -> Result<bool, RunOrchestratorError> {
        let Some(lease) = self.claim_next().await? else {
            return Ok(false);
        };
        if let Err(error) = self.execute_lease(&lease).await {
            self.fail_lease(&lease, failure_code(&error)).await?;
            return Err(error);
        }
        Ok(true)
    }

    pub async fn execute_lease(&self, lease: &RunLease) -> Result<(), RunOrchestratorError> {
        self.heartbeat(lease).await?;
        let source = self.git.validate_source(&lease.git_url)?;
        let git_ref = self.git.validate_ref(&lease.source_ref)?;
        let workspace_id = Uuid::now_v7();
        let mut checkout = self
            .git
            .provision(lease.project_id, workspace_id, &source, &git_ref)
            .await?;
        if lease.workspace_kind == "worktree" {
            if let Err(error) = self
                .git
                .switch_or_create_branch(workspace_id, &lease.source_ref)
                .await
            {
                self.git.remove_workspace(workspace_id).await?;
                return Err(error.into());
            }
            checkout.branch = lease.source_ref.clone();
        }
        if lease.copy_agents_md {
            if let Some(parent_run_id) = lease.workspace_parent_run_id {
                let parent_workspace_id = sqlx::query_scalar::<_, Uuid>(
                    "SELECT parent.workspace_id FROM runs parent \
                     JOIN workspaces workspace ON workspace.id = parent.workspace_id \
                     WHERE parent.id = $1 AND parent.organization_id = $2 \
                       AND workspace.state <> 'retired'",
                )
                .bind(parent_run_id)
                .bind(lease.organization_id)
                .fetch_optional(&self.db)
                .await?;
                if let Some(parent_workspace_id) = parent_workspace_id {
                    if let Err(error) = self
                        .git
                        .copy_agents_md(parent_workspace_id, workspace_id)
                        .await
                    {
                        tracing::warn!(%error, run_id = %lease.run_id, "optional AGENTS.md copy failed");
                    }
                }
            }
        }
        self.heartbeat(lease).await?;

        if let Err(error) = self.record_workspace(lease, workspace_id, &checkout).await {
            self.git.remove_workspace(workspace_id).await?;
            return Err(error);
        }

        let workspace = AuthorizedWorkspace {
            id: workspace_id.to_string(),
            root: checkout.root.clone(),
        };
        let started = if let Some(source_thread_id) = lease.fork_thread_id.as_deref() {
            let source = sqlx::query(
                "SELECT parent.workspace_id, parent_workspace.root_path \
                 FROM runs parent JOIN workspaces parent_workspace \
                   ON parent_workspace.id = parent.workspace_id \
                 WHERE parent.id = $1 AND parent.organization_id = $2 \
                   AND parent.codex_thread_id = $3 AND parent_workspace.state <> 'retired'",
            )
            .bind(lease.fork_source_run_id)
            .bind(lease.organization_id)
            .bind(source_thread_id)
            .fetch_optional(&self.db)
            .await?
            .ok_or(RunOrchestratorError::NotFound)?;
            let source_workspace = AuthorizedWorkspace {
                id: source.get::<Uuid, _>("workspace_id").to_string(),
                root: source.get::<String, _>("root_path").into(),
            };
            self.adapter
                .fork_thread(&source_workspace, &workspace, source_thread_id)
                .await
        } else {
            self.adapter.start_thread(&workspace).await
        };
        let started = match started {
            Ok(started) => started,
            Err(error) => {
                self.queue_cleanup(lease, workspace_id, "thread_start_failed")
                    .await?;
                return Err(error.into());
            }
        };
        let updated = sqlx::query(
            "WITH updated_run AS ( \
                 UPDATE runs SET status = 'running', codex_thread_id = $1, heartbeat_at = now(), \
                                 lease_expires_at = $2, updated_at = now() \
                 WHERE id = $3 AND lease_owner = $4 AND lease_token = $5 AND status = 'provisioning' \
                 RETURNING task_id, workspace_id \
             ), updated_workspace AS ( \
                 UPDATE workspaces SET state = 'busy', updated_at = now() \
                 WHERE id IN (SELECT workspace_id FROM updated_run) \
             ) \
             UPDATE tasks SET status = 'running', updated_at = now() \
             WHERE id IN (SELECT task_id FROM updated_run)",
        )
        .bind(&started.thread_id)
        .bind(Utc::now() + crate::chrono_ttl(self.lease_ttl)?)
        .bind(lease.run_id)
        .bind(&self.worker_id)
        .bind(&lease.token)
        .execute(&self.db)
        .await?
        .rows_affected();
        if updated != 1 {
            self.record_delivery_uncertainty(lease, workspace_id, &started.thread_id)
                .await?;
            return Err(RunOrchestratorError::LeaseLost);
        }
        Ok(())
    }

    async fn record_delivery_uncertainty(
        &self,
        lease: &RunLease,
        workspace_id: Uuid,
        thread_id: &str,
    ) -> Result<(), RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        sqlx::query(
            "UPDATE runs SET codex_thread_id = COALESCE(codex_thread_id, $1), \
                             status = CASE WHEN status = 'cancelled' THEN 'cancelled' \
                                           ELSE 'recovery_pending' END, \
                             failure_code = CASE WHEN status = 'cancelled' THEN failure_code \
                                                 ELSE 'thread_delivery_unknown' END, \
                             lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL, \
                             updated_at = now() \
             WHERE id = $2 AND workspace_id = $3",
        )
        .bind(thread_id)
        .bind(lease.run_id)
        .bind(workspace_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE workspaces SET state = 'ready', updated_at = now() WHERE id = $1")
            .bind(workspace_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn record_workspace(
        &self,
        lease: &RunLease,
        workspace_id: Uuid,
        checkout: &open_web_codex_git_runtime::WorkspaceCheckout,
    ) -> Result<(), RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        let owned: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE id = $1 AND lease_owner = $2 \
             AND lease_token = $3 AND status = 'provisioning')",
        )
        .bind(lease.run_id)
        .bind(&self.worker_id)
        .bind(&lease.token)
        .fetch_one(&mut *transaction)
        .await?;
        if !owned {
            return Err(RunOrchestratorError::LeaseLost);
        }
        sqlx::query(
            "INSERT INTO workspaces (id, organization_id, project_id, profile_id, run_id, root_path, \
                                     state, source_ref, head_commit, branch_name) \
             VALUES ($1, $2, $3, $4, $5, $6, 'ready', $7, $8, $9)",
        )
        .bind(workspace_id)
        .bind(lease.organization_id)
        .bind(lease.project_id)
        .bind(lease.profile_id)
        .bind(lease.run_id)
        .bind(checkout.root.to_string_lossy().as_ref())
        .bind(&lease.source_ref)
        .bind(&checkout.head_commit)
        .bind(&checkout.branch)
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE runs SET workspace_id = $1, updated_at = now() WHERE id = $2")
            .bind(workspace_id)
            .bind(lease.run_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn fail_lease(
        &self,
        lease: &RunLease,
        code: &'static str,
    ) -> Result<(), RunOrchestratorError> {
        sqlx::query(
            "UPDATE runs SET status = 'failed', failure_code = $1, lease_owner = NULL, \
                             lease_token = NULL, lease_expires_at = NULL, updated_at = now() \
             WHERE id = $2 AND lease_owner = $3 AND lease_token = $4 \
               AND status IN ('provisioning', 'cancelling')",
        )
        .bind(code)
        .bind(lease.run_id)
        .bind(&self.worker_id)
        .bind(&lease.token)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    async fn queue_cleanup(
        &self,
        lease: &RunLease,
        workspace_id: Uuid,
        code: &'static str,
    ) -> Result<(), RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        sqlx::query(
            "UPDATE workspaces SET state = 'cleanup_pending', updated_at = now() WHERE id = $1",
        )
        .bind(workspace_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO runner_jobs (organization_id, run_id, workspace_id, kind, last_error_code) \
             VALUES ($1, $2, $3, 'workspace_cleanup', $4) \
             ON CONFLICT (workspace_id, kind) DO UPDATE SET state = 'pending', run_after = now(), \
                 last_error_code = EXCLUDED.last_error_code, updated_at = now()",
        )
        .bind(lease.organization_id)
        .bind(lease.run_id)
        .bind(workspace_id)
        .bind(code)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }

    pub async fn run_cleanup_once(&self) -> Result<bool, RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        let job = sqlx::query(
            "SELECT job.id, job.workspace_id \
             FROM runner_jobs job \
             WHERE job.kind = 'workspace_cleanup' \
               AND (job.state = 'pending' OR (job.state = 'running' AND job.lease_expires_at < now())) \
               AND job.run_after <= now() \
             ORDER BY job.created_at, job.id FOR UPDATE SKIP LOCKED LIMIT 1",
        )
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(job) = job else {
            transaction.commit().await?;
            return Ok(false);
        };
        let job_id: Uuid = job.get("id");
        let workspace_id: Uuid = job.get("workspace_id");
        let token = Uuid::now_v7().to_string();
        sqlx::query(
            "UPDATE runner_jobs SET state = 'running', attempt = attempt + 1, lease_owner = $1, \
                                    lease_token = $2, lease_expires_at = now() + interval '1 minute', \
                                    updated_at = now() WHERE id = $3",
        )
        .bind(&self.worker_id)
        .bind(&token)
        .bind(job_id)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;

        if let Err(error) = self.git.remove_workspace(workspace_id).await {
            sqlx::query(
                "UPDATE runner_jobs SET state = CASE WHEN attempt >= 5 THEN 'failed' ELSE 'pending' END, \
                                        run_after = now() + interval '30 seconds', lease_owner = NULL, \
                                        lease_token = NULL, lease_expires_at = NULL, \
                                        last_error_code = 'workspace_cleanup_failed', updated_at = now() \
                 WHERE id = $1 AND lease_token = $2",
            )
            .bind(job_id)
            .bind(&token)
            .execute(&self.db)
            .await?;
            return Err(error.into());
        }
        let mut transaction = self.db.begin().await?;
        sqlx::query(
            "UPDATE workspaces SET state = 'retired', retired_at = now(), updated_at = now() WHERE id = $1",
        )
        .bind(workspace_id)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE runner_jobs SET state = 'completed', lease_owner = NULL, lease_token = NULL, \
                                    lease_expires_at = NULL, updated_at = now() \
             WHERE id = $1 AND lease_token = $2",
        )
        .bind(job_id)
        .bind(&token)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }
}

fn failure_code(error: &RunOrchestratorError) -> &'static str {
    match error {
        RunOrchestratorError::Invalid(_) => "invalid_run",
        RunOrchestratorError::NotFound => "resource_not_found",
        RunOrchestratorError::Conflict(_) => "run_conflict",
        RunOrchestratorError::LeaseLost => "lease_lost",
        RunOrchestratorError::Database(_) => "database_error",
        RunOrchestratorError::Git(_) => "git_workspace_error",
        RunOrchestratorError::Adapter(_) => "codex_unavailable",
    }
}
