use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use open_web_codex_adapter::fake::FakeCodexAdapter;
use open_web_codex_adapter::CodexAdapter;
use open_web_codex_git_runtime::{GitRuntime, GitRuntimeConfig};
use open_web_codex_platform_store::migrate;
use open_web_codex_run_orchestrator::{
    CancelRunRequest, EnqueueRunRequest, RecoverRunRequest, RunOrchestrator, RunOrchestratorError,
};
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use tempfile::TempDir;
use uuid::Uuid;

fn git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(["-c", "core.hooksPath=/dev/null"])
        .args(args)
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run Git fixture command");
    assert!(
        output.status.success(),
        "Git fixture failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn source_repository(root: &TempDir) -> String {
    let source = root.path().join("source");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-b", "main"]);
    std::fs::write(source.join("README.md"), "runner fixture\n").unwrap();
    git(&source, &["add", "README.md"]);
    git(
        &source,
        &[
            "-c",
            "user.name=Runner Fixture",
            "-c",
            "user.email=runner@example.invalid",
            "commit",
            "-m",
            "initial",
        ],
    );
    source.to_string_lossy().to_string()
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
async fn idempotent_enqueue_single_lease_and_workspace_provisioning() {
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&database_url)
        .await
        .expect("connect disposable PostgreSQL database");
    migrate::run(&pool).await.expect("run migrations");

    let fixture = TempDir::new().unwrap();
    let git_url = source_repository(&fixture);
    let organization_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let profile_id = Uuid::now_v7();
    let project_id = Uuid::now_v7();
    let task_id = Uuid::now_v7();
    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, 'Runner', $2)")
        .bind(organization_id)
        .bind(format!("runner-{organization_id}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO users (id, username, name, email, password_hash, role) \
         VALUES ($1, $2, 'Runner', $3, 'not-a-password', 'owner')",
    )
    .bind(user_id)
    .bind(format!("runner-{user_id}"))
    .bind(format!("{user_id}@example.invalid"))
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO memberships (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO profiles (id, organization_id, owner_user_id, runtime_key, name) \
         VALUES ($1, $2, $3, 'runner-profile', 'Runner Profile')",
    )
    .bind(profile_id)
    .bind(organization_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO projects (id, organization_id, created_by, name, git_url, default_branch) \
         VALUES ($1, $2, $3, 'Project', $4, 'main')",
    )
    .bind(project_id)
    .bind(organization_id)
    .bind(user_id)
    .bind(&git_url)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tasks (id, organization_id, project_id, created_by, title) \
         VALUES ($1, $2, $3, $4, 'Task')",
    )
    .bind(task_id)
    .bind(organization_id)
    .bind(project_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    let git_runtime = Arc::new(
        GitRuntime::new(GitRuntimeConfig::new(fixture.path().join("runner")).with_local_sources())
            .unwrap(),
    );
    let adapter: Arc<dyn CodexAdapter> = Arc::new(FakeCodexAdapter::new());
    let first = RunOrchestrator::new(
        pool.clone(),
        git_runtime.clone(),
        adapter.clone(),
        "runner-profile",
        "worker-a",
        Duration::from_secs(30),
    )
    .unwrap();
    let second = RunOrchestrator::new(
        pool.clone(),
        git_runtime.clone(),
        adapter,
        "runner-profile",
        "worker-b",
        Duration::from_secs(30),
    )
    .unwrap();
    let request = EnqueueRunRequest {
        organization_id,
        actor_id: user_id,
        task_id,
        idempotency_key: "runner-idempotency-0001".to_string(),
        git_ref: None,
        workspace_kind: "main".to_string(),
        workspace_name: None,
        workspace_parent_run_id: None,
        workspace_group_run_id: None,
        copy_agents_md: false,
        fork_thread_id: None,
        fork_source_run_id: None,
    };
    let enqueued = first.enqueue_run(request.clone()).await.unwrap();
    let replayed = first.enqueue_run(request).await.unwrap();
    assert_eq!(enqueued, replayed);
    let conflict = first
        .enqueue_run(EnqueueRunRequest {
            organization_id,
            actor_id: user_id,
            task_id,
            idempotency_key: "runner-idempotency-0002".to_string(),
            git_ref: None,
            workspace_kind: "main".to_string(),
            workspace_name: None,
            workspace_parent_run_id: None,
            workspace_group_run_id: None,
            copy_agents_md: false,
            fork_thread_id: None,
            fork_source_run_id: None,
        })
        .await
        .unwrap_err();
    assert!(matches!(conflict, RunOrchestratorError::Conflict(_)));

    let (claimed_first, claimed_second) = tokio::join!(first.claim_next(), second.claim_next());
    let claimed_first = claimed_first.unwrap();
    let claimed_second = claimed_second.unwrap();
    assert_ne!(claimed_first.is_some(), claimed_second.is_some());
    let (owner, lease) = if let Some(lease) = claimed_first {
        (&first, lease)
    } else {
        (&second, claimed_second.unwrap())
    };
    owner.execute_lease(&lease).await.unwrap();

    let row = sqlx::query(
        "SELECT r.status, r.codex_thread_id, r.workspace_id, w.root_path, w.state \
         FROM runs r JOIN workspaces w ON w.id = r.workspace_id WHERE r.id = $1",
    )
    .bind(enqueued.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<String, _>("status"), "running");
    assert!(row.get::<Option<String>, _>("codex_thread_id").is_some());
    assert_eq!(row.get::<String, _>("state"), "busy");
    let workspace_id: Uuid = row.get("workspace_id");
    let root_path: String = row.get("root_path");
    assert_eq!(
        Path::new(&root_path),
        git_runtime.workspace_path(workspace_id)
    );
    assert!(Path::new(&root_path).is_dir());

    sqlx::query("UPDATE runs SET active_turn_id = 'turn-to-cancel' WHERE id = $1")
        .bind(enqueued.id)
        .execute(&pool)
        .await
        .unwrap();
    let cancelled = owner
        .cancel_run(CancelRunRequest {
            organization_id,
            actor_id: user_id,
            allow_organization_admin: false,
            run_id: enqueued.id,
        })
        .await
        .unwrap();
    assert_eq!(cancelled.status, "cancelled");
    assert!(cancelled.active_turn_id.is_none());

    let recovery_run = first
        .enqueue_run(EnqueueRunRequest {
            organization_id,
            actor_id: user_id,
            task_id,
            idempotency_key: "runner-idempotency-0003".to_string(),
            git_ref: None,
            workspace_kind: "main".to_string(),
            workspace_name: None,
            workspace_parent_run_id: None,
            workspace_group_run_id: None,
            copy_agents_md: false,
            fork_thread_id: None,
            fork_source_run_id: None,
        })
        .await
        .unwrap();
    let recovery_lease = second.claim_next().await.unwrap().unwrap();
    second.execute_lease(&recovery_lease).await.unwrap();
    let recovery_root: String = sqlx::query_scalar(
        "SELECT w.root_path FROM runs r JOIN workspaces w ON w.id = r.workspace_id WHERE r.id = $1",
    )
    .bind(recovery_run.id)
    .fetch_one(&pool)
    .await
    .unwrap();

    sqlx::query("UPDATE runs SET lease_expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(recovery_run.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(second.reap_expired().await.unwrap(), 1);
    let status: String = sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
        .bind(recovery_run.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "recovery_pending");
    assert!(Path::new(&recovery_root).exists());
    let recovered = owner
        .recover_run(RecoverRunRequest {
            organization_id,
            actor_id: user_id,
            allow_organization_admin: false,
            run_id: recovery_run.id,
        })
        .await
        .unwrap();
    assert_eq!(recovered.status, "running");
    let recovery_owner: Option<String> =
        sqlx::query_scalar("SELECT lease_owner FROM runs WHERE id = $1")
            .bind(recovery_run.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(recovery_owner.as_deref(), Some("worker-a"));
    let cleanup_jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runner_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        cleanup_jobs, 0,
        "running workspaces are not deleted on lease expiry"
    );
}
