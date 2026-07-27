use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use crate::{
    chrono_ttl, validate_idempotency_key, CancelRunRequest, EnqueueRunRequest, RecoverRunRequest,
    RetireWorkspaceRequest, RunLease, RunOrchestrator, RunOrchestratorError, RunRecord,
};

impl RunOrchestrator {
    pub async fn enqueue_run(
        &self,
        request: EnqueueRunRequest,
    ) -> Result<RunRecord, RunOrchestratorError> {
        validate_idempotency_key(&request.idempotency_key)?;
        if !matches!(
            request.workspace_kind.as_str(),
            "main" | "worktree" | "clone"
        ) {
            return Err(RunOrchestratorError::Invalid(
                "workspace kind is invalid".to_string(),
            ));
        }
        let workspace_name = request
            .workspace_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if workspace_name
            .as_ref()
            .is_some_and(|value| value.len() > 128)
        {
            return Err(RunOrchestratorError::Invalid(
                "workspace name is too long".to_string(),
            ));
        }
        let row = sqlx::query(
            "SELECT t.id AS task_id, t.project_id, project.git_url, project.default_branch, profile.id AS profile_id \
             FROM tasks t \
             JOIN projects project ON project.id = t.project_id AND project.organization_id = t.organization_id \
             JOIN profiles profile ON profile.organization_id = t.organization_id \
               AND profile.owner_user_id = $2 AND profile.runtime_key = $4 AND profile.status = 'active' \
             WHERE t.id = $1 AND t.organization_id = $3",
        )
        .bind(request.task_id)
        .bind(request.actor_id)
        .bind(request.organization_id)
        .bind(&self.runtime_key)
        .fetch_optional(&self.db)
        .await?
        .ok_or(RunOrchestratorError::NotFound)?;
        let git_url: String = row.get("git_url");
        self.git.validate_source(&git_url)?;
        let source_ref = request
            .git_ref
            .unwrap_or_else(|| row.get::<String, _>("default_branch"));
        self.git.validate_ref(&source_ref)?;
        let profile_id: Uuid = row.get("profile_id");
        if request.workspace_kind == "main" && request.workspace_parent_run_id.is_some() {
            return Err(RunOrchestratorError::Invalid(
                "main workspaces cannot have a parent Run".to_string(),
            ));
        }
        if request.workspace_kind == "main" && request.workspace_group_run_id.is_some() {
            return Err(RunOrchestratorError::Invalid(
                "main workspaces cannot have a derived workspace group".to_string(),
            ));
        }
        if request.workspace_kind == "worktree" && request.workspace_parent_run_id.is_none() {
            return Err(RunOrchestratorError::Invalid(
                "worktree workspaces require a parent Run".to_string(),
            ));
        }
        if let Some(parent_run_id) = request.workspace_parent_run_id {
            let parent_is_authorized: bool = sqlx::query_scalar(
                "SELECT EXISTS( \
                   SELECT 1 FROM runs parent \
                   JOIN tasks parent_task ON parent_task.id = parent.task_id \
                   JOIN workspaces parent_workspace ON parent_workspace.id = parent.workspace_id \
                   WHERE parent.id = $1 AND parent.organization_id = $2 \
                     AND parent_task.project_id = $3 AND parent.requested_by = $4 \
                     AND parent_workspace.state <> 'retired' \
                 )",
            )
            .bind(parent_run_id)
            .bind(request.organization_id)
            .bind(row.get::<Uuid, _>("project_id"))
            .bind(request.actor_id)
            .fetch_one(&self.db)
            .await?;
            if !parent_is_authorized {
                return Err(RunOrchestratorError::NotFound);
            }
        }
        if let Some(group_run_id) = request.workspace_group_run_id {
            let group_is_authorized: bool = sqlx::query_scalar(
                "SELECT EXISTS( \
                   SELECT 1 FROM runs root \
                   JOIN tasks root_task ON root_task.id = root.task_id \
                   JOIN workspaces root_workspace ON root_workspace.id = root.workspace_id \
                   WHERE root.id = $1 AND root.organization_id = $2 \
                     AND root_task.project_id = $3 AND root.requested_by = $4 \
                     AND root.workspace_kind = $5 AND root.workspace_kind <> 'main' \
                     AND root.workspace_group_run_id IS NULL \
                     AND root_workspace.state <> 'retired' \
                 )",
            )
            .bind(group_run_id)
            .bind(request.organization_id)
            .bind(row.get::<Uuid, _>("project_id"))
            .bind(request.actor_id)
            .bind(&request.workspace_kind)
            .fetch_one(&self.db)
            .await?;
            if !group_is_authorized {
                return Err(RunOrchestratorError::NotFound);
            }
        }
        if let Some(fork_thread_id) = request.fork_thread_id.as_deref() {
            if fork_thread_id.trim().is_empty() || fork_thread_id.len() > 256 {
                return Err(RunOrchestratorError::Invalid(
                    "fork source Thread id is invalid".to_string(),
                ));
            }
            let Some(source_run_id) = request.fork_source_run_id else {
                return Err(RunOrchestratorError::Invalid(
                    "forked Runs require their source parent Run".to_string(),
                ));
            };
            let source_matches: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM runs source \
                 WHERE source.id = $1 AND source.organization_id = $2 \
                   AND source.requested_by = $3 AND source.codex_thread_id = $4)",
            )
            .bind(source_run_id)
            .bind(request.organization_id)
            .bind(request.actor_id)
            .bind(fork_thread_id)
            .fetch_one(&self.db)
            .await?;
            if !source_matches {
                return Err(RunOrchestratorError::NotFound);
            }
        } else if request.fork_source_run_id.is_some() {
            return Err(RunOrchestratorError::Invalid(
                "fork source Run requires a source Thread id".to_string(),
            ));
        }

        let inserted = sqlx::query(
            "INSERT INTO runs (organization_id, task_id, requested_by, requested_profile_id, \
                               source_ref, idempotency_key, workspace_kind, workspace_name, \
                               workspace_parent_run_id, workspace_group_run_id, \
                               workspace_copy_agents_md, fork_thread_id, fork_source_run_id, status) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'pending') \
             ON CONFLICT (organization_id, requested_by, idempotency_key) \
               WHERE requested_by IS NOT NULL AND idempotency_key IS NOT NULL \
             DO NOTHING \
             RETURNING id, task_id, status, codex_thread_id, active_turn_id, workspace_id, source_ref, \
                       workspace_kind, workspace_name, workspace_parent_run_id, \
                       workspace_group_run_id, attempt, created_at, updated_at",
        )
        .bind(request.organization_id)
        .bind(request.task_id)
        .bind(request.actor_id)
        .bind(profile_id)
        .bind(&source_ref)
        .bind(&request.idempotency_key)
        .bind(&request.workspace_kind)
        .bind(&workspace_name)
        .bind(request.workspace_parent_run_id)
        .bind(request.workspace_group_run_id)
        .bind(request.copy_agents_md)
        .bind(&request.fork_thread_id)
        .bind(request.fork_source_run_id)
        .fetch_optional(&self.db)
        .await;

        let row = match inserted {
            Ok(Some(row)) => row,
            Ok(None) => {
                let existing = sqlx::query(
                    "SELECT id, task_id, status, codex_thread_id, active_turn_id, workspace_id, source_ref, \
                            workspace_kind, workspace_name, workspace_parent_run_id, \
                            workspace_group_run_id, attempt, created_at, updated_at \
                     FROM runs WHERE organization_id = $1 AND requested_by = $2 \
                       AND idempotency_key = $3",
                )
                .bind(request.organization_id)
                .bind(request.actor_id)
                .bind(&request.idempotency_key)
                .fetch_one(&self.db)
                .await?;
                if existing.get::<Uuid, _>("task_id") != request.task_id {
                    return Err(RunOrchestratorError::Conflict(
                        "idempotency key was already used for another Task".to_string(),
                    ));
                }
                existing
            }
            Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
                return Err(RunOrchestratorError::Conflict(
                    "Task already has an active Run".to_string(),
                ));
            }
            Err(error) => return Err(error.into()),
        };
        Ok(run_record(&row))
    }

    pub async fn cancel_run(
        &self,
        request: CancelRunRequest,
    ) -> Result<RunRecord, RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        let row = sqlx::query(
            "SELECT r.id, r.task_id, r.status, r.codex_thread_id, r.active_turn_id, \
                    r.workspace_id, r.source_ref, r.workspace_kind, r.workspace_name, \
                    r.workspace_parent_run_id, r.workspace_group_run_id, \
                    r.attempt, r.created_at, r.updated_at, \
                    r.requested_by, w.root_path \
             FROM runs r LEFT JOIN workspaces w ON w.id = r.workspace_id \
             WHERE r.id = $1 AND r.organization_id = $2 FOR UPDATE OF r",
        )
        .bind(request.run_id)
        .bind(request.organization_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(RunOrchestratorError::NotFound)?;
        let requested_by: Option<Uuid> = row.get("requested_by");
        if requested_by != Some(request.actor_id) && !request.allow_organization_admin {
            return Err(RunOrchestratorError::NotFound);
        }
        let status: String = row.get("status");
        if matches!(status.as_str(), "completed" | "cancelled" | "failed") {
            return Err(RunOrchestratorError::Conflict(
                "Run is already in a terminal state".to_string(),
            ));
        }
        sqlx::query(
            "UPDATE runs SET status = 'cancelled', active_turn_id = NULL, failure_code = NULL, \
                             lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL, \
                             updated_at = now() WHERE id = $1",
        )
        .bind(request.run_id)
        .execute(&mut *transaction)
        .await?;
        if let Some(workspace_id) = row.get::<Option<Uuid>, _>("workspace_id") {
            sqlx::query(
                "UPDATE workspaces SET state = 'ready', updated_at = now() \
                 WHERE id = $1 AND state <> 'retired'",
            )
            .bind(workspace_id)
            .execute(&mut *transaction)
            .await?;
        }
        sqlx::query("UPDATE tasks SET status = 'cancelled', updated_at = now() WHERE id = $1")
            .bind(row.get::<Uuid, _>("task_id"))
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;

        let thread_id: Option<String> = row.get("codex_thread_id");
        let turn_id: Option<String> = row.get("active_turn_id");
        let workspace_id: Option<Uuid> = row.get("workspace_id");
        let root_path: Option<String> = row.get("root_path");
        if let (Some(thread_id), Some(turn_id), Some(workspace_id), Some(root_path)) =
            (thread_id, turn_id, workspace_id, root_path)
        {
            let workspace = open_web_codex_adapter::AuthorizedWorkspace {
                id: workspace_id.to_string(),
                root: root_path.into(),
            };
            if let Err(error) = self
                .adapter
                .interrupt_turn(&workspace, &thread_id, &turn_id)
                .await
            {
                sqlx::query(
                    "UPDATE runs SET status = 'recovery_pending', failure_code = 'interrupt_failed', \
                                     updated_at = now() WHERE id = $1 AND status = 'cancelled'",
                )
                .bind(request.run_id)
                .execute(&self.db)
                .await?;
                return Err(error.into());
            }
        }
        self.get_run(request.organization_id, request.run_id).await
    }

    pub async fn retire_workspace(
        &self,
        request: RetireWorkspaceRequest,
    ) -> Result<RunRecord, RunOrchestratorError> {
        let row = sqlx::query(
            "SELECT r.requested_by, r.workspace_kind, r.workspace_id, r.codex_thread_id, \
                    r.active_turn_id, w.root_path, w.state \
             FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
             WHERE r.id = $1 AND r.organization_id = $2",
        )
        .bind(request.run_id)
        .bind(request.organization_id)
        .fetch_optional(&self.db)
        .await?
        .ok_or(RunOrchestratorError::NotFound)?;
        let requested_by: Option<Uuid> = row.get("requested_by");
        if requested_by != Some(request.actor_id) && !request.allow_organization_admin {
            return Err(RunOrchestratorError::NotFound);
        }
        if row.get::<String, _>("workspace_kind") == "main" {
            return Err(RunOrchestratorError::Conflict(
                "the main Run workspace cannot be removed".to_string(),
            ));
        }
        if row.get::<String, _>("state") == "retired" {
            return self.get_run(request.organization_id, request.run_id).await;
        }
        let workspace_id: Uuid = row.get("workspace_id");
        if let (Some(thread_id), Some(turn_id)) = (
            row.get::<Option<String>, _>("codex_thread_id"),
            row.get::<Option<String>, _>("active_turn_id"),
        ) {
            let workspace = open_web_codex_adapter::AuthorizedWorkspace {
                id: workspace_id.to_string(),
                root: row.get::<String, _>("root_path").into(),
            };
            self.adapter
                .interrupt_turn(&workspace, &thread_id, &turn_id)
                .await?;
        }

        let mut transaction = self.db.begin().await?;
        let locked = sqlx::query(
            "SELECT r.task_id, r.requested_by, r.workspace_kind, w.state \
             FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
             WHERE r.id = $1 AND r.organization_id = $2 AND w.id = $3 \
             FOR UPDATE OF r, w",
        )
        .bind(request.run_id)
        .bind(request.organization_id)
        .bind(workspace_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(RunOrchestratorError::NotFound)?;
        if locked.get::<Option<Uuid>, _>("requested_by") != Some(request.actor_id)
            && !request.allow_organization_admin
        {
            return Err(RunOrchestratorError::NotFound);
        }
        if locked.get::<String, _>("workspace_kind") == "main" {
            return Err(RunOrchestratorError::Conflict(
                "the main Run workspace cannot be removed".to_string(),
            ));
        }
        if locked.get::<String, _>("state") != "retired" {
            sqlx::query(
                "UPDATE runs SET status = 'cancelled', active_turn_id = NULL, failure_code = NULL, \
                                 lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL, \
                                 updated_at = now() WHERE id = $1",
            )
            .bind(request.run_id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query("UPDATE tasks SET status = 'archived', updated_at = now() WHERE id = $1")
                .bind(locked.get::<Uuid, _>("task_id"))
                .execute(&mut *transaction)
                .await?;
            sqlx::query(
                "UPDATE workspaces SET state = 'cleanup_pending', updated_at = now() WHERE id = $1",
            )
            .bind(workspace_id)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO runner_jobs (organization_id, run_id, workspace_id, kind) \
                 VALUES ($1, $2, $3, 'workspace_cleanup') \
                 ON CONFLICT (workspace_id, kind) DO UPDATE SET state = 'pending', \
                   run_after = now(), lease_owner = NULL, lease_token = NULL, \
                   lease_expires_at = NULL, updated_at = now()",
            )
            .bind(request.organization_id)
            .bind(request.run_id)
            .bind(workspace_id)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        self.get_run(request.organization_id, request.run_id).await
    }

    pub async fn get_run(
        &self,
        organization_id: Uuid,
        run_id: Uuid,
    ) -> Result<RunRecord, RunOrchestratorError> {
        let row = sqlx::query(
            "SELECT id, task_id, status, codex_thread_id, active_turn_id, workspace_id, source_ref, \
                    workspace_kind, workspace_name, workspace_parent_run_id, workspace_group_run_id, \
                    attempt, created_at, updated_at FROM runs \
             WHERE id = $1 AND organization_id = $2",
        )
        .bind(run_id)
        .bind(organization_id)
        .fetch_optional(&self.db)
        .await?
        .ok_or(RunOrchestratorError::NotFound)?;
        Ok(run_record(&row))
    }

    pub async fn recover_run(
        &self,
        request: RecoverRunRequest,
    ) -> Result<RunRecord, RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        let row = sqlx::query(
            "SELECT r.status, r.requested_by, r.workspace_id, r.codex_thread_id, w.state, \
                    profile.runtime_key \
             FROM runs r JOIN workspaces w ON w.id = r.workspace_id \
             JOIN profiles profile ON profile.id = r.requested_profile_id \
             WHERE r.id = $1 AND r.organization_id = $2 AND w.organization_id = $2 \
             FOR UPDATE OF r",
        )
        .bind(request.run_id)
        .bind(request.organization_id)
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(RunOrchestratorError::NotFound)?;
        let requested_by: Option<Uuid> = row.get("requested_by");
        if requested_by != Some(request.actor_id) && !request.allow_organization_admin {
            return Err(RunOrchestratorError::NotFound);
        }
        if row.get::<String, _>("runtime_key") != self.runtime_key
            || row.get::<String, _>("state") == "retired"
            || row.get::<Option<Uuid>, _>("workspace_id").is_none()
            || row.get::<Option<String>, _>("codex_thread_id").is_none()
        {
            return Err(RunOrchestratorError::NotFound);
        }
        let status: String = row.get("status");
        if status == "running" {
            transaction.commit().await?;
            return self.get_run(request.organization_id, request.run_id).await;
        }
        if status != "recovery_pending" {
            return Err(RunOrchestratorError::Conflict(format!(
                "Run cannot recover from status '{status}'"
            )));
        }

        let token = Uuid::now_v7().to_string();
        let expires_at = Utc::now() + chrono_ttl(self.lease_ttl)?;
        let task_id: Uuid = sqlx::query_scalar(
            "UPDATE runs SET status = 'running', active_turn_id = NULL, failure_code = NULL, \
                             lease_owner = $1, lease_token = $2, lease_expires_at = $3, \
                             heartbeat_at = now(), updated_at = now() \
             WHERE id = $4 AND status = 'recovery_pending' RETURNING task_id",
        )
        .bind(&self.worker_id)
        .bind(&token)
        .bind(expires_at)
        .bind(request.run_id)
        .fetch_one(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE workspaces SET state = 'busy', updated_at = now() \
             WHERE id = $1 AND state <> 'retired'",
        )
        .bind(row.get::<Uuid, _>("workspace_id"))
        .execute(&mut *transaction)
        .await?;
        sqlx::query("UPDATE tasks SET status = 'running', updated_at = now() WHERE id = $1")
            .bind(task_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.get_run(request.organization_id, request.run_id).await
    }

    pub async fn claim_next(&self) -> Result<Option<RunLease>, RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        let candidate = sqlx::query(
            "SELECT r.id, r.task_id, r.organization_id, r.requested_profile_id, r.source_ref, \
                    r.workspace_kind, r.workspace_parent_run_id, r.workspace_copy_agents_md, \
                    r.fork_thread_id, r.fork_source_run_id, \
                    t.project_id, p.git_url \
             FROM runs r \
             JOIN tasks t ON t.id = r.task_id AND t.organization_id = r.organization_id \
             JOIN projects p ON p.id = t.project_id AND p.organization_id = r.organization_id \
             JOIN profiles profile ON profile.id = r.requested_profile_id \
               AND profile.runtime_key = $1 AND profile.status = 'active' \
             WHERE r.status = 'pending' AND (r.lease_expires_at IS NULL OR r.lease_expires_at < now()) \
             ORDER BY r.created_at, r.id \
             FOR UPDATE OF r SKIP LOCKED LIMIT 1",
        )
        .bind(&self.runtime_key)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some(candidate) = candidate else {
            transaction.commit().await?;
            return Ok(None);
        };
        let run_id: Uuid = candidate.get("id");
        let token = Uuid::now_v7().to_string();
        let expires_at = Utc::now() + chrono_ttl(self.lease_ttl)?;
        let attempt: i32 = sqlx::query_scalar(
            "UPDATE runs SET status = 'provisioning', lease_owner = $1, lease_token = $2, \
                             lease_expires_at = $3, heartbeat_at = now(), attempt = attempt + 1, \
                             failure_code = NULL, updated_at = now() \
             WHERE id = $4 RETURNING attempt",
        )
        .bind(&self.worker_id)
        .bind(&token)
        .bind(expires_at)
        .bind(run_id)
        .fetch_one(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(Some(RunLease {
            run_id,
            task_id: candidate.get("task_id"),
            project_id: candidate.get("project_id"),
            organization_id: candidate.get("organization_id"),
            profile_id: candidate.get("requested_profile_id"),
            git_url: candidate.get("git_url"),
            source_ref: candidate.get("source_ref"),
            workspace_kind: candidate.get("workspace_kind"),
            workspace_parent_run_id: candidate.get("workspace_parent_run_id"),
            fork_thread_id: candidate.get("fork_thread_id"),
            fork_source_run_id: candidate.get("fork_source_run_id"),
            copy_agents_md: candidate.get("workspace_copy_agents_md"),
            token,
            expires_at,
            attempt,
        }))
    }

    pub async fn heartbeat(&self, lease: &RunLease) -> Result<(), RunOrchestratorError> {
        let expires_at = Utc::now() + chrono_ttl(self.lease_ttl)?;
        let updated = sqlx::query(
            "UPDATE runs SET heartbeat_at = now(), lease_expires_at = $1, updated_at = now() \
             WHERE id = $2 AND lease_owner = $3 AND lease_token = $4 \
               AND status IN ('provisioning', 'running', 'cancelling')",
        )
        .bind(expires_at)
        .bind(lease.run_id)
        .bind(&self.worker_id)
        .bind(&lease.token)
        .execute(&self.db)
        .await?
        .rows_affected();
        if updated != 1 {
            return Err(RunOrchestratorError::LeaseLost);
        }
        Ok(())
    }

    pub async fn heartbeat_owned_runs(&self) -> Result<u64, RunOrchestratorError> {
        let expires_at = Utc::now() + chrono_ttl(self.lease_ttl)?;
        Ok(sqlx::query(
            "UPDATE runs SET heartbeat_at = now(), lease_expires_at = $1, updated_at = now() \
             WHERE lease_owner = $2 AND lease_token IS NOT NULL \
               AND status IN ('provisioning', 'running', 'cancelling')",
        )
        .bind(expires_at)
        .bind(&self.worker_id)
        .execute(&self.db)
        .await?
        .rows_affected())
    }

    pub async fn reap_expired(&self) -> Result<u64, RunOrchestratorError> {
        let mut transaction = self.db.begin().await?;
        let expired = sqlx::query(
            "UPDATE runs SET status = 'recovery_pending', failure_code = 'lease_expired', \
                             lease_owner = NULL, lease_token = NULL, lease_expires_at = NULL, \
                             updated_at = now() \
             WHERE status IN ('provisioning', 'running', 'cancelling') \
               AND lease_expires_at < now() \
             RETURNING id, organization_id, workspace_id, codex_thread_id",
        )
        .fetch_all(&mut *transaction)
        .await?;
        for row in &expired {
            let workspace_id: Option<Uuid> = row.get("workspace_id");
            let thread_id: Option<String> = row.get("codex_thread_id");
            if let Some(workspace_id) = workspace_id.filter(|_| thread_id.is_none()) {
                sqlx::query(
                    "UPDATE workspaces SET state = 'cleanup_pending', updated_at = now() WHERE id = $1",
                )
                .bind(workspace_id)
                .execute(&mut *transaction)
                .await?;
                sqlx::query(
                    "INSERT INTO runner_jobs (organization_id, run_id, workspace_id, kind) \
                     VALUES ($1, $2, $3, 'workspace_cleanup') \
                     ON CONFLICT (workspace_id, kind) DO UPDATE SET state = 'pending', run_after = now(), updated_at = now()",
                )
                .bind(row.get::<Uuid, _>("organization_id"))
                .bind(row.get::<Uuid, _>("id"))
                .bind(workspace_id)
                .execute(&mut *transaction)
                .await?;
            }
        }
        transaction.commit().await?;
        Ok(expired.len() as u64)
    }
}

pub(crate) fn run_record(row: &sqlx::postgres::PgRow) -> RunRecord {
    RunRecord {
        id: row.get("id"),
        task_id: row.get("task_id"),
        status: row.get("status"),
        codex_thread_id: row.get("codex_thread_id"),
        active_turn_id: row.get("active_turn_id"),
        workspace_id: row.get("workspace_id"),
        source_ref: row.get("source_ref"),
        workspace_kind: row.get("workspace_kind"),
        workspace_name: row.get("workspace_name"),
        workspace_parent_run_id: row.get("workspace_parent_run_id"),
        workspace_group_run_id: row.get("workspace_group_run_id"),
        attempt: row.get("attempt"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
