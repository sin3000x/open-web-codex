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
        let workspace = AuthorizedWorkspace {
            id: lease.workspace_id.to_string(),
            root: lease.workspace_root.clone(),
        };
        let source_workspace = if let Some(source_thread_id) = lease.fork_thread_id.as_deref() {
            let source = sqlx::query(
                "SELECT source_workspace.id AS workspace_id, source_workspace.root_path \
                 FROM runs source_run \
                 JOIN runs target_run ON target_run.id = $5 \
                   AND target_run.organization_id = source_run.organization_id \
                   AND target_run.task_id = source_run.task_id \
                   AND target_run.workspace_id = source_run.workspace_id \
                 JOIN workspaces source_workspace ON source_workspace.id = source_run.workspace_id \
                 JOIN workspace_grants source_grant \
                   ON source_grant.workspace_id = source_workspace.id \
                  AND source_grant.organization_id = source_workspace.organization_id \
                  AND source_grant.user_id = $4 \
                  AND source_grant.profile_id = source_workspace.profile_id \
                 WHERE source_run.id = $1 AND source_run.organization_id = $2 \
                   AND source_run.codex_thread_id = $3 \
                   AND target_run.workspace_id = $6 \
                   AND source_workspace.state IN ('ready', 'retained')",
            )
            .bind(lease.fork_source_run_id)
            .bind(lease.organization_id)
            .bind(source_thread_id)
            .bind(lease.actor_id)
            .bind(lease.run_id)
            .bind(lease.workspace_id)
            .fetch_optional(&self.db)
            .await?
            .ok_or(RunOrchestratorError::NotFound)?;
            let source_workspace = AuthorizedWorkspace {
                id: source.get::<Uuid, _>("workspace_id").to_string(),
                root: source.get::<String, _>("root_path").into(),
            };
            Some(source_workspace)
        } else {
            None
        };
        let started = match (source_workspace, lease.fork_thread_id.as_deref()) {
            (Some(source_workspace), Some(source_thread_id)) => {
                self.adapter
                    .fork_thread(
                        &source_workspace,
                        &workspace,
                        lease.copilot_package_id.as_deref(),
                        source_thread_id,
                    )
                    .await?
            }
            (None, None) => {
                self.adapter
                    .start_thread(&workspace, lease.copilot_package_id.as_deref())
                    .await?
            }
            _ => {
                return Err(RunOrchestratorError::Conflict(
                    "fork source workspace did not match the leased Run".to_string(),
                ));
            }
        };

        match self
            .persist_thread_delivery(lease, &started.thread_id)
            .await
        {
            Ok(()) => Ok(()),
            Err(error) => {
                self.record_delivery_uncertainty(lease, &started.thread_id)
                    .await?;
                Err(error)
            }
        }
    }

    async fn persist_thread_delivery(
        &self,
        lease: &RunLease,
        thread_id: &str,
    ) -> Result<(), RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        let task_id = sqlx::query_scalar::<_, Uuid>(
            "UPDATE runs SET status = 'running', codex_thread_id = $1, heartbeat_at = now(), \
                             lease_expires_at = $2, updated_at = now() \
             WHERE id = $3 AND workspace_id = $4 \
               AND lease_owner = $5 AND lease_token = $6 AND status = 'provisioning' \
             RETURNING task_id",
        )
        .bind(thread_id)
        .bind(Utc::now() + crate::chrono_ttl(self.lease_ttl)?)
        .bind(lease.run_id)
        .bind(lease.workspace_id)
        .bind(&self.worker_id)
        .bind(&lease.token)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(task_id) = task_id else {
            transaction.rollback().await?;
            return Err(RunOrchestratorError::LeaseLost);
        };

        insert_root_agent_projection(&mut transaction, lease, thread_id).await?;
        sqlx::query("UPDATE tasks SET status = 'running', updated_at = now() WHERE id = $1")
            .bind(task_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn record_delivery_uncertainty(
        &self,
        lease: &RunLease,
        thread_id: &str,
    ) -> Result<(), RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        let updated_run = sqlx::query(
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
        .bind(lease.workspace_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if updated_run != 1 {
            transaction.rollback().await?;
            return Err(RunOrchestratorError::LeaseLost);
        }
        insert_root_agent_projection(&mut transaction, lease, thread_id).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn fail_lease(
        &self,
        lease: &RunLease,
        code: &'static str,
    ) -> Result<(), RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
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
            let state: String = sqlx::query_scalar(
                "UPDATE runner_jobs \
                 SET state = CASE WHEN attempt >= 5 THEN 'failed' ELSE 'pending' END, \
                     run_after = now() + interval '30 seconds', lease_owner = NULL, \
                     lease_token = NULL, lease_expires_at = NULL, \
                     last_error_code = 'workspace_cleanup_failed', updated_at = now() \
                 WHERE id = $1 AND lease_token = $2 RETURNING state",
            )
            .bind(job_id)
            .bind(&token)
            .fetch_one(&self.db)
            .await?;
            if state == "failed" {
                sqlx::query(
                    "UPDATE workspaces SET state = 'cleanup_failed', updated_at = now() \
                     WHERE id = $1 AND state = 'removing'",
                )
                .bind(workspace_id)
                .execute(&self.db)
                .await?;
            }
            return Err(error.into());
        }
        let mut transaction = self.db.begin().await?;
        sqlx::query(
            "UPDATE workspaces \
             SET state = 'removed', removed_at = now(), updated_at = now() WHERE id = $1",
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

async fn insert_root_agent_projection(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    lease: &RunLease,
    thread_id: &str,
) -> Result<(), RunOrchestratorError> {
    let inserted = sqlx::query(
        "INSERT INTO runtime_agent_projections (
            organization_id, profile_id, workspace_id, root_run_id, thread_id, source_kind
         ) VALUES ($1, $2, $3, $4, $5, 'root')
         ON CONFLICT (profile_id, thread_id) DO UPDATE
           SET last_observed_at = now()
           WHERE runtime_agent_projections.root_run_id = EXCLUDED.root_run_id
             AND runtime_agent_projections.workspace_id = EXCLUDED.workspace_id
         RETURNING root_run_id",
    )
    .bind(lease.organization_id)
    .bind(lease.profile_id)
    .bind(lease.workspace_id)
    .bind(lease.run_id)
    .bind(thread_id)
    .fetch_optional(&mut **transaction)
    .await?;
    if inserted.is_none() {
        return Err(RunOrchestratorError::Conflict(
            "Runtime Thread is already projected under another Run or Workspace".to_string(),
        ));
    }
    Ok(())
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
