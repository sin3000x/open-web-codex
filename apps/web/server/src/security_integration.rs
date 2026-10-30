use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
use chrono::DateTime;
use futures_util::{SinkExt, StreamExt};
use open_web_codex_adapter::{fake::FakeCodexAdapter, CodexAdapter};
use open_web_codex_approval_service::{ApprovalActor, ApprovalService};
use open_web_codex_auth::hash_password;
use open_web_codex_git_runtime::{GitRuntime, GitRuntimeConfig};
use open_web_codex_platform_contracts::{ApprovalDecision, DecideApprovalRequest};
use open_web_codex_platform_store::AppState;
use open_web_codex_provider_service::secured::InMemoryAuthorizedProviderService;
use open_web_codex_run_orchestrator::RunOrchestrator;
use open_web_codex_secret_store::{PostgresSecretStore, SecretCipher};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};
use sqlx::{AssertSqlSafe, Row};
use tower::ServiceExt;
use uuid::Uuid;

use crate::ensure_transitional_profile_binding;
use crate::routes::{self, RuntimeProfileBinding};

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
async fn task_creation_binds_only_an_authorized_project_workspace() {
    let pool = isolated_database_pool().await;
    for retired_table in [
        "data_intake_sessions",
        "data_intake_input_requests",
        "workspace_data_drafts",
        "workspace_data_source_assets",
        "workspace_dataset_releases",
        "workspace_dataset_release_files",
        "task_dataset_bindings",
        "task_analysis_execution_snapshots",
        "task_intake_artifact_projections",
        "task_policy_agent_producers",
        "agent_release_dataset_dependencies",
        "capability_catalog_installation_events",
        "capability_catalog_installations",
        "capability_catalog_release_dependencies",
        "capability_catalog_releases",
        "capability_catalog_drafts",
        "agent_run_bindings",
        "agent_run_snapshots",
        "supervisor_release_agent_dependencies",
        "agent_release_capability_package_dependencies",
        "agent_definition_releases",
        "agent_definition_revisions",
        "agent_definitions",
        "workspace_capability_package_releases",
        "supervisor_instruction_policy_releases",
        "work_operation_inputs",
        "work_operation_outputs",
        "work_state_events",
        "work_deliverables",
        "work_blocking_inputs",
        "work_component_dependencies",
        "work_components",
        "work_operations",
        "work_states",
        "work_state_definitions",
        "supervisor_policy_bindings",
        "supervisor_run_continuations",
        "supervisor_policy_snapshots",
        "supervisor_releases",
        "supervisor_revisions",
        "supervisor_definitions",
        "profile_capabilities",
    ] {
        let relation: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(retired_table)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(relation, None, "retired table remains: {retired_table}");
    }
    for retained_table in [
        "provider_call_metrics",
        "runtime_agent_projections",
        "runtime_agent_execution_projections",
    ] {
        let relation: Option<String> = sqlx::query_scalar("SELECT to_regclass($1)::text")
            .bind(retained_table)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            relation.as_deref(),
            Some(retained_table),
            "retained table is missing: {retained_table}"
        );
    }
    for (table, column) in [
        ("workspaces", "source_revision"),
        ("artifacts", "intake_envelope"),
        ("provider_call_metrics", "stable_prefix_sha256"),
        ("provider_call_metrics", "tool_inventory_sha256"),
        ("provider_call_metrics", "skill_set_sha256"),
        ("provider_call_metrics", "runtime_role_sha256"),
    ] {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2",
        )
        .bind(table)
        .bind(column)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "retired column remains: {table}.{column}");
    }
    let runtime_key = "task-workspace-test-profile";
    let profile = RuntimeProfileBinding {
        runtime_key: runtime_key.to_string(),
        name: "Task Workspace Test Profile".to_string(),
        codex_home: None,
    };
    let runner_root = tempfile::tempdir().expect("runner root");
    let git = Arc::new(
        GitRuntime::new(GitRuntimeConfig::new(runner_root.path()).with_local_sources())
            .expect("Git runtime"),
    );
    let adapter = Arc::new(FakeCodexAdapter::new().with_demo_workspace().await);
    let orchestrator = Arc::new(
        RunOrchestrator::new(
            pool.clone(),
            git.clone(),
            adapter.clone(),
            runtime_key,
            "task-workspace-test-worker",
            std::time::Duration::from_secs(30),
        )
        .expect("Run orchestrator"),
    );
    let app = Router::new()
        .nest(
            "/api",
            routes::router(
                adapter,
                Arc::new(InMemoryAuthorizedProviderService::default()),
                Arc::new(ApprovalService::new(pool.clone(), runtime_key)),
                git.clone(),
                orchestrator,
                Arc::new(PostgresSecretStore::new(
                    pool.clone(),
                    SecretCipher::generate("task-workspace-test-v1").expect("test Secret cipher"),
                )),
                profile,
                Arc::new(crate::delivery_contracts::DeliveryRegistry::default()),
                Arc::new(
                    crate::copilot_installation::CopilotInstallationService::new(
                        crate::copilot_installation::CopilotInstallationStore::new(
                            pool.clone(),
                            runtime_key,
                        ),
                        crate::copilot_installation::CopilotSourceRegistry::default(),
                        None,
                        None,
                    ),
                ),
            ),
        )
        .with_state(AppState::new(pool.clone()));

    let bootstrap = call(
        &app,
        Request::post("/api/bootstrap")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "name": "Task Workspace Owner",
                    "username": "task-workspace-owner",
                    "email": "task-workspace@example.invalid",
                    "password": "task-workspace-password"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(bootstrap.0, StatusCode::OK);
    let token = bootstrap.1["session_token"].as_str().unwrap();
    let user_id = Uuid::parse_str(bootstrap.1["user"]["id"].as_str().unwrap()).unwrap();
    let organization_id =
        Uuid::parse_str(bootstrap.1["organization"]["id"].as_str().unwrap()).unwrap();

    let missing_task_run = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/tasks/{}/runs", Uuid::now_v7()),
            token,
            json!({
                "idempotency_key": "missing-task-run"
            }),
        ),
    )
    .await;
    assert_eq!(missing_task_run.0, StatusCode::CONFLICT);
    assert_eq!(missing_task_run.1["kind"], "workspace_unavailable");
    assert_eq!(missing_task_run.1["message"], "task_workspace_unavailable");

    let profile_id: Uuid = sqlx::query_scalar("SELECT id FROM profiles WHERE runtime_key = $1")
        .bind(runtime_key)
        .fetch_one(&pool)
        .await
        .unwrap();

    let project = call(
        &app,
        authenticated_json(
            "POST",
            "/api/projects",
            token,
            json!({"name": "Bound Project", "git_url": "https://example.invalid/bound.git"}),
        ),
    )
    .await;
    assert_eq!(project.0, StatusCode::OK);
    let project_id = Uuid::parse_str(project.1["id"].as_str().unwrap()).unwrap();
    let workspace_id = Uuid::now_v7();
    let workspace_root = git.workspace_path(workspace_id);
    std::fs::create_dir(&workspace_root).unwrap();
    fixture_git(&workspace_root, &["init", "-b", "main"]);
    sqlx::query(
        "INSERT INTO workspaces \
         (id, organization_id, project_id, profile_id, created_by, kind, name, root_path, source_ref, state) \
         VALUES ($1, $2, $3, $4, $5, 'main', 'Authorized Workspace', $6, 'main', 'ready')",
    )
    .bind(workspace_id)
    .bind(organization_id)
    .bind(project_id)
    .bind(profile_id)
    .bind(user_id)
    .bind(workspace_root.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .unwrap();

    let create = |project_id: Uuid, title: &'static str| {
        authenticated_json(
            "POST",
            "/api/tasks",
            token,
            json!({
                "project_id": project_id,
                "workspace_id": workspace_id,
                "title": title
            }),
        )
    };
    assert_eq!(
        call(&app, create(project_id, "No Grant")).await.0,
        StatusCode::NOT_FOUND
    );

    sqlx::query(
        "INSERT INTO workspace_grants \
         (workspace_id, organization_id, user_id, profile_id, role) \
         VALUES ($1, $2, $3, $4, 'owner')",
    )
    .bind(workspace_id)
    .bind(organization_id)
    .bind(user_id)
    .bind(profile_id)
    .execute(&pool)
    .await
    .unwrap();

    let other_project = call(
        &app,
        authenticated_json(
            "POST",
            "/api/projects",
            token,
            json!({"name": "Other Project", "git_url": "https://example.invalid/other.git"}),
        ),
    )
    .await;
    let other_project_id = Uuid::parse_str(other_project.1["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        call(&app, create(other_project_id, "Wrong Project"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );

    let created = call(&app, create(project_id, "Authorized Task")).await;
    assert_eq!(created.0, StatusCode::OK);
    assert_eq!(created.1["workspace_id"], workspace_id.to_string());
    assert!(created.1.get("root_path").is_none());
    let task_id = created.1["id"].as_str().unwrap();
    let loaded = call(
        &app,
        authenticated("GET", &format!("/api/tasks/{task_id}"), token),
    )
    .await;
    assert_eq!(loaded.0, StatusCode::OK);
    assert_eq!(loaded.1["workspace_id"], workspace_id.to_string());

    let retired_run_contract = app
        .clone()
        .oneshot(authenticated_json(
            "POST",
            &format!("/api/tasks/{task_id}/runs"),
            token,
            json!({
                "idempotency_key": "retired-run-contract",
                "readiness_fingerprint": "retired"
            }),
        ))
        .await
        .expect("retired Run contract response");
    assert_eq!(
        retired_run_contract.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    let second = call(&app, create(project_id, "Second Shared-Workspace Task")).await;
    assert_eq!(second.0, StatusCode::OK);
    let second_task_id = Uuid::parse_str(second.1["id"].as_str().unwrap()).unwrap();
    let task_workspaces = sqlx::query_scalar::<_, Uuid>(
        "SELECT workspace_id FROM tasks WHERE id = ANY($1) ORDER BY id",
    )
    .bind(vec![Uuid::parse_str(task_id).unwrap(), second_task_id])
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(task_workspaces, vec![workspace_id, workspace_id]);

    let shared_path = "shared/planning-input.csv";
    let shared_bytes = b"city_id,demand_quantity\n3171,10\n";
    git.write_file(workspace_id, shared_path, shared_bytes, false)
        .await
        .expect("write ordinary shared Workspace file");
    assert!(matches!(
        git.write_file(workspace_id, shared_path, b"replacement", false)
            .await,
        Err(open_web_codex_git_runtime::GitRuntimeError::FileAlreadyExists(path))
            if path == shared_path
    ));
    let shared = call(
        &app,
        authenticated(
            "GET",
            &format!(
                "/api/workspaces/{workspace_id}/files/content?path=shared%2Fplanning-input.csv"
            ),
            token,
        ),
    )
    .await;
    assert_eq!(shared.0, StatusCode::OK);
    assert_eq!(
        shared.1["content"].as_str(),
        Some(String::from_utf8_lossy(shared_bytes).as_ref())
    );

    let task_file_api = app
        .clone()
        .oneshot(authenticated(
            "GET",
            &format!("/api/tasks/{second_task_id}/files"),
            token,
        ))
        .await;
    assert_eq!(task_file_api.unwrap().status(), StatusCode::NOT_FOUND);

    for retired_route in [
        "/api/agent-definitions",
        "/api/agent-definition-resources",
        "/api/capability-packages",
        "/api/supervisor-definitions",
        "/api/supervisor-instruction-policies",
        "/api/supervisor-policies",
    ] {
        let response = app
            .clone()
            .oneshot(authenticated("GET", retired_route, token))
            .await
            .expect("retired catalog route response");
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "retired route remains: {retired_route}"
        );
    }
    let retired_python_route =
        format!("/api/workspaces/{workspace_id}/python-capabilities/validate");
    for retired_route in ["/api/catalog/drafts", retired_python_route.as_str()] {
        let response = app
            .clone()
            .oneshot(authenticated_json("POST", retired_route, token, json!({})))
            .await
            .expect("retired authoring route response");
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "retired route remains: {retired_route}"
        );
    }

    let boundary = "slice2-workspace-upload";
    let multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"batch-one.csv\"\r\nContent-Type: text/csv\r\n\r\none\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"files\"; filename=\"batch-two.csv\"\r\nContent-Type: text/csv\r\n\r\ntwo\r\n--{boundary}--\r\n"
    );
    let multi_file_upload = call(
        &app,
        Request::post(format!("/api/workspaces/{workspace_id}/files"))
            .header("authorization", format!("Bearer {token}"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(multipart))
            .unwrap(),
    )
    .await;
    assert_eq!(multi_file_upload.0, StatusCode::BAD_REQUEST);
    assert!(!workspace_root.join("batch-one.csv").exists());
    assert!(!workspace_root.join("batch-two.csv").exists());

    for escaped in ["%2Fetc%2Fpasswd", "..%2Foutside.csv"] {
        let response = app
            .clone()
            .oneshot(authenticated(
                "GET",
                &format!("/api/workspaces/{workspace_id}/files/content?path={escaped}"),
                token,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
async fn organization_and_profile_authorization_prevent_cross_tenant_access() {
    let pool = isolated_database_pool().await;

    let codex_home = tempfile::tempdir().expect("Codex home");
    let profile = RuntimeProfileBinding {
        runtime_key: "security-test-profile".to_string(),
        name: "Security Test Profile".to_string(),
        codex_home: Some(Arc::new(codex_home.path().to_path_buf())),
    };
    let state = AppState::new(pool.clone());
    let approval_service = Arc::new(ApprovalService::new(pool.clone(), "security-test-profile"));
    let runner_root = tempfile::tempdir().expect("runner root");
    let git = Arc::new(
        GitRuntime::new(GitRuntimeConfig::new(runner_root.path()).with_local_sources())
            .expect("Git runtime"),
    );
    let adapter = Arc::new(FakeCodexAdapter::new().with_demo_workspace().await);
    let orchestrator = Arc::new(
        RunOrchestrator::new(
            pool.clone(),
            git.clone(),
            adapter.clone(),
            "security-test-profile",
            "security-test-worker",
            std::time::Duration::from_secs(30),
        )
        .expect("Run orchestrator"),
    );
    let app = Router::new()
        .nest(
            "/api",
            routes::router(
                adapter.clone(),
                Arc::new(InMemoryAuthorizedProviderService::default()),
                approval_service.clone(),
                git.clone(),
                orchestrator,
                Arc::new(PostgresSecretStore::new(
                    pool.clone(),
                    SecretCipher::generate("security-test-v1").expect("test Secret cipher"),
                )),
                profile,
                Arc::new(crate::delivery_contracts::DeliveryRegistry::default()),
                Arc::new(
                    crate::copilot_installation::CopilotInstallationService::new(
                        crate::copilot_installation::CopilotInstallationStore::new(
                            pool.clone(),
                            "security-test-profile",
                        ),
                        crate::copilot_installation::CopilotSourceRegistry::default(),
                        None,
                        None,
                    ),
                ),
            ),
        )
        .with_state(state.clone());

    let bootstrap = call_with_headers(
        &app,
        Request::post("/api/bootstrap")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "name": "First Owner",
                    "username": "first-owner",
                    "email": "first@example.invalid",
                    "password": "first-password"
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(bootstrap.0, StatusCode::OK);
    assert!(bootstrap
        .1
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.contains("session_token=")
                && value.contains("HttpOnly")
                && value.contains("SameSite=Strict")
                && value.contains("Path=/api/")
        }));
    let first_token = bootstrap.2["session_token"]
        .as_str()
        .expect("bootstrap session token")
        .to_string();
    let first_user_id = Uuid::parse_str(bootstrap.2["user"]["id"].as_str().unwrap()).unwrap();
    let first_organization_id =
        Uuid::parse_str(bootstrap.2["organization"]["id"].as_str().unwrap()).unwrap();

    let local_session = call_with_headers(
        &app,
        Request::post("/api/sessions/local")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(local_session.0, StatusCode::OK);
    assert_eq!(
        local_session.2["user"]["id"].as_str(),
        bootstrap.2["user"]["id"].as_str()
    );
    assert_eq!(
        local_session.2["organization"]["id"].as_str(),
        bootstrap.2["organization"]["id"].as_str()
    );
    let local_token = local_session.2["session_token"]
        .as_str()
        .expect("implicit local session token");
    let local_me = call(&app, authenticated("GET", "/api/me", local_token)).await;
    assert_eq!(local_me.0, StatusCode::OK);

    ensure_transitional_profile_binding(&pool, "legacy-profile", "Legacy Profile")
        .await
        .expect("repair legacy Profile binding");
    let repaired_owner: (Uuid, Uuid) = sqlx::query_as(
        "SELECT organization_id, owner_user_id FROM profiles WHERE runtime_key = $1",
    )
    .bind("legacy-profile")
    .fetch_one(&pool)
    .await
    .expect("load repaired Profile binding");
    assert_eq!(repaired_owner, (first_organization_id, first_user_id));

    let first_project = call(
        &app,
        authenticated_json(
            "POST",
            "/api/projects",
            &first_token,
            json!({"name": "First Project", "git_url": "https://example.invalid/first.git"}),
        ),
    )
    .await;
    assert_eq!(first_project.0, StatusCode::OK);
    let first_project_id = first_project.1["id"].as_str().unwrap().to_string();

    let first_task_id = Uuid::now_v7();
    let first_run_id = Uuid::now_v7();
    let source = runner_root.path().join("image-source");
    std::fs::create_dir(&source).unwrap();
    fixture_git(&source, &["init", "-b", "main"]);
    let image_bytes = b"\x89PNG\r\n\x1a\nroute-integration";
    std::fs::write(source.join("icon.png"), image_bytes).unwrap();
    fixture_git(&source, &["add", "icon.png"]);
    fixture_git(
        &source,
        &[
            "-c",
            "user.name=Asset Test",
            "-c",
            "user.email=asset@example.invalid",
            "commit",
            "-m",
            "image fixture",
        ],
    );
    let workspace_id = Uuid::now_v7();
    let checkout = git
        .provision(
            Uuid::parse_str(&first_project_id).unwrap(),
            workspace_id,
            &git.validate_source(source.to_string_lossy().as_ref())
                .unwrap(),
            &git.validate_ref("main").unwrap(),
        )
        .await
        .unwrap();
    let profile_id: Uuid =
        sqlx::query_scalar("SELECT id FROM profiles WHERE runtime_key = 'security-test-profile'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO workspaces \
         (id, organization_id, project_id, profile_id, created_by, root_path, state, \
          source_ref, head_commit, branch_name, kind, name) \
         VALUES ($1, $2, $3, $4, $5, $6, 'ready', 'main', $7, 'main', 'main', 'First Workspace')",
    )
    .bind(workspace_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(profile_id)
    .bind(first_user_id)
    .bind(checkout.root.to_string_lossy().as_ref())
    .bind(&checkout.head_commit)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO workspace_grants \
         (workspace_id, organization_id, user_id, profile_id, role) \
         VALUES ($1, $2, $3, $4, 'owner')",
    )
    .bind(workspace_id)
    .bind(first_organization_id)
    .bind(first_user_id)
    .bind(profile_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tasks (id, organization_id, project_id, workspace_id, title) \
         VALUES ($1, $2, $3, $4, 'Approval Task')",
    )
    .bind(first_task_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(workspace_id)
    .execute(&pool)
    .await
    .unwrap();

    let completed_followup_task_id = Uuid::now_v7();
    let completed_followup_run_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tasks (id, organization_id, project_id, workspace_id, created_by, title, status) \
         VALUES ($1, $2, $3, $4, $5, 'Completed Followup Task', 'completed')",
    )
    .bind(completed_followup_task_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(workspace_id)
    .bind(first_user_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs \
         (id, organization_id, task_id, requested_by, requested_profile_id, workspace_id, \
          status, codex_thread_id) \
         VALUES ($1, $2, $3, $4, $5, $6, 'completed', 'completed-followup-thread')",
    )
    .bind(completed_followup_run_id)
    .bind(first_organization_id)
    .bind(completed_followup_task_id)
    .bind(first_user_id)
    .bind(profile_id)
    .bind(workspace_id)
    .execute(&pool)
    .await
    .unwrap();
    let followup_response = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/tasks/{completed_followup_task_id}/messages"),
            &first_token,
            json!({ "text": "continue after completion", "images": [] }),
        ),
    )
    .await;
    assert_eq!(followup_response.0, StatusCode::OK);
    assert_eq!(
        followup_response.1["thread_id"].as_str(),
        Some("completed-followup-thread")
    );
    let reopened: (String, Option<String>, String) = sqlx::query_as(
        "SELECT run.status, run.active_turn_id, task.status \
         FROM runs run JOIN tasks task ON task.id = run.task_id \
         WHERE run.id = $1",
    )
    .bind(completed_followup_run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(reopened.0, "running");
    assert!(reopened.1.is_some());
    assert_eq!(reopened.2, "running");

    sqlx::query(
        "INSERT INTO runs \
         (id, organization_id, task_id, requested_by, requested_profile_id, workspace_id, \
          status, codex_thread_id) \
         VALUES ($1, $2, $3, $4, $5, $6, 'running', 'approval-thread')",
    )
    .bind(first_run_id)
    .bind(first_organization_id)
    .bind(first_task_id)
    .bind(first_user_id)
    .bind(profile_id)
    .bind(workspace_id)
    .execute(&pool)
    .await
    .unwrap();
    let artifact_id = Uuid::now_v7();
    let artifact_bytes =
        b"# Warehouse network planning report\n\n<!-- network_planning_report_markdown.v2 -->\n";
    let artifact_digest = hex::encode(Sha256::digest(artifact_bytes));
    sqlx::query(
        "INSERT INTO artifacts (
            id, organization_id, profile_id, workspace_id, artifact_schema, display_name,
            source_relative_path, mime_type, expected_size, byte_size,
            content, content_sha256, state, delivery_verifier
         ) VALUES (
            $1, $2, $3, $4, 'network_planning_report_markdown.v2',
            'Warehouse network planning report', 'deliverables/security-report.md',
            'text/markdown', $5, $5, $6, $7, 'ready', jsonb_build_object(
                'kind', 'markdown_marker',
                'marker', '<!-- network_planning_report_markdown.v2 -->'
            )
         )",
    )
    .bind(artifact_id)
    .bind(first_organization_id)
    .bind(profile_id)
    .bind(workspace_id)
    .bind(i64::try_from(artifact_bytes.len()).unwrap())
    .bind(artifact_bytes.as_slice())
    .bind(artifact_digest)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO artifact_task_grants (
            artifact_id, organization_id, task_id, permission
         ) VALUES ($1, $2, $3, 'read')",
    )
    .bind(artifact_id)
    .bind(first_organization_id)
    .bind(first_task_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO artifact_provenance (
            artifact_id, organization_id, producer_task_id, producer_run_id,
            producer_thread_id, producer_turn_id, producer_item_id
         ) VALUES (
            $1, $2, $3, $4, 'approval-thread', 'turn-map', 'item-data'
         )",
    )
    .bind(artifact_id)
    .bind(first_organization_id)
    .bind(first_task_id)
    .bind(first_run_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO artifact_task_grants (
            artifact_id, organization_id, task_id, permission
         ) VALUES ($1, $2, $3, 'read')",
    )
    .bind(artifact_id)
    .bind(first_organization_id)
    .bind(completed_followup_task_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO artifact_provenance (
            artifact_id, organization_id, producer_task_id, producer_run_id,
            producer_thread_id, producer_turn_id, producer_item_id
         ) VALUES (
            $1, $2, $3, $4, 'completed-followup-thread',
            'completed-followup-turn', 'completed-followup-item'
         )",
    )
    .bind(artifact_id)
    .bind(first_organization_id)
    .bind(completed_followup_task_id)
    .bind(completed_followup_run_id)
    .execute(&pool)
    .await
    .unwrap();
    let first_task_artifacts = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/tasks/{first_task_id}/artifacts"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(first_task_artifacts.0, StatusCode::OK);
    assert_eq!(
        first_task_artifacts.1[0]["producer_run_id"],
        first_run_id.to_string()
    );
    let reused_task_artifacts = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/tasks/{completed_followup_task_id}/artifacts"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(reused_task_artifacts.0, StatusCode::OK);
    assert_eq!(
        reused_task_artifacts.1[0]["producer_run_id"],
        completed_followup_run_id.to_string()
    );
    let artifact_detail = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/artifacts/{artifact_id}"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(artifact_detail.0, StatusCode::OK);
    assert_eq!(
        artifact_detail.1["task_id"],
        completed_followup_task_id.to_string()
    );
    assert_eq!(
        artifact_detail.1["producer_run_id"],
        completed_followup_run_id.to_string()
    );
    assert!(artifact_detail.1.get("source_server").is_none());
    assert!(artifact_detail.1.get("source_uri").is_none());

    let artifact = app
        .clone()
        .oneshot(authenticated(
            "GET",
            &format!("/api/artifacts/{artifact_id}/content"),
            &first_token,
        ))
        .await
        .expect("Artifact content response");
    assert_eq!(artifact.status(), StatusCode::OK);
    let artifact_body = to_bytes(artifact.into_body(), 1024 * 1024)
        .await
        .expect("Artifact content body");
    assert_eq!(artifact_body.as_ref(), artifact_bytes);

    let image_response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/workspaces/{workspace_id}/assets?path=icon.png"
            ))
            .header("cookie", format!("session_token={first_token}"))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(image_response.status(), StatusCode::OK);
    assert_eq!(image_response.headers()["content-type"], "image/png");
    assert_eq!(
        image_response.headers()["cache-control"],
        "private, no-store"
    );
    assert_eq!(
        image_response.headers()["x-content-type-options"],
        "nosniff"
    );
    assert_eq!(
        image_response.headers()["cross-origin-resource-policy"],
        "same-origin"
    );
    assert!(image_response.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("sandbox"));
    assert_eq!(
        to_bytes(image_response.into_body(), 1024).await.unwrap(),
        image_bytes.as_slice()
    );

    let missing_auth = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/workspaces/{workspace_id}/assets?path=icon.png"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing_auth.status(), StatusCode::UNAUTHORIZED);
    let runtime_instance_id = adapter.runtime_instance_id().await;
    let approval_id = approval_service
        .capture_message(
            runtime_instance_id,
            &json!({
                "id": 77,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "approval-thread",
                    "turnId": "turn-1",
                    "itemId": "item-1",
                    "command": "git status",
                    "cwd": "/private/server/path",
                    "reason": "inspect changes"
                }
            }),
        )
        .await
        .unwrap()
        .expect("captured approval");

    let pending = call(&app, authenticated("GET", "/api/approvals", &first_token)).await;
    assert_eq!(pending.0, StatusCode::OK);
    assert_eq!(pending.1[0]["id"], approval_id.to_string());
    assert!(!pending.1.to_string().contains("/private/server/path"));
    assert!(!pending.1.to_string().contains("\"77\""));

    let run_pending = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/approval-requests"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(run_pending.0, StatusCode::OK);
    assert_eq!(run_pending.1[0]["subject"]["kind"], "command");
    assert_eq!(run_pending.1[0]["subject"]["action"], "execute");
    assert!(!run_pending.1.to_string().contains("git status"));
    assert!(!run_pending.1.to_string().contains("/private/server/path"));

    let decided = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/approvals/{approval_id}/decision"),
            &first_token,
            json!({"decision": "accept", "version": 0}),
        ),
    )
    .await;
    assert_eq!(decided.0, StatusCode::NO_CONTENT);
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM audit_log WHERE target_id = $1 AND action = 'approval.decide'",
    )
    .bind(approval_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit_count, 1);

    sqlx::query(
        "INSERT INTO runtime_agent_execution_projections (
            organization_id, profile_id, workspace_id, root_run_id, agent_thread_id,
            turn_id, ordinal, task, display_title, status, current_behavior,
            first_observed_sequence, last_observed_sequence
         ) VALUES ($1, $2, $3, $4, 'approval-child-thread', 'child-turn-1', 1,
                   'Prepare data', 'Agent · use this ResourceRef for the full assigned task',
                   'waiting', 'Waiting for input', 1, 1)",
    )
    .bind(first_organization_id)
    .bind(profile_id)
    .bind(workspace_id)
    .bind(first_run_id)
    .execute(&pool)
    .await
    .unwrap();
    let child_command_approval_id = approval_service
        .capture_message(
            runtime_instance_id,
            &json!({
                "id": 771,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "approval-child-thread",
                    "turnId": "child-turn-1",
                    "itemId": "child-item-1",
                    "commandActions": [{"type": "read", "path": "data/input.csv"}]
                }
            }),
        )
        .await
        .unwrap()
        .expect("captured child generic approval");
    let child_generic_pending = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/approval-requests"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(child_generic_pending.0, StatusCode::OK);
    let child_generic = child_generic_pending
        .1
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == child_command_approval_id.to_string())
        .expect("child generic approval projection");
    assert_eq!(child_generic["source"]["kind"], "agent");
    assert_eq!(child_generic["source"]["displayTitle"], "Agent");
    assert!(!child_generic.to_string().contains("ResourceRef"));
    let early_child_frame = format!(
        "data: {}\n\n",
        json!({
            "method": "app-server-event",
            "params": {
                "workspace_id": workspace_id,
                "message": {
                    "method": "platform/approvalRequested",
                    "params": {
                        "approvalId": child_command_approval_id,
                        "threadId": "approval-child-thread",
                        "turnId": "child-turn-1",
                        "itemId": "child-item-1",
                        "requestMethod": "item/commandExecution/requestApproval",
                        "requestParams": {
                            "threadId": "approval-child-thread",
                            "turnId": "child-turn-1",
                            "itemId": "child-item-1"
                        }
                    }
                }
            }
        })
    );
    let early_child_projection =
        crate::event_projection::persist_frame(early_child_frame.as_bytes(), &pool)
            .await
            .unwrap()
            .expect("early child approval event projection");
    assert_eq!(
        early_child_projection.organization_id,
        first_organization_id
    );
    let projected_child_approval_events: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM run_events
         WHERE run_id = $1 AND event_type = 'platform.approval.requested'
           AND thread_id = 'approval-child-thread'",
    )
    .bind(first_run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(projected_child_approval_events, 1);
    sqlx::query(
        "INSERT INTO runtime_agent_projections (
            organization_id, profile_id, workspace_id, root_run_id, thread_id,
            parent_thread_id, source_kind, agent_path, agent_nickname, agent_role
         ) VALUES ($1, $2, $3, $4, 'approval-child-thread', 'approval-thread',
                   'thread_spawn', '/root/data', 'Wanwan', 'data_agent')",
    )
    .bind(first_organization_id)
    .bind(profile_id)
    .bind(workspace_id)
    .bind(first_run_id)
    .execute(&pool)
    .await
    .unwrap();

    let inline_task_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tasks (id, organization_id, project_id, workspace_id, title) \
         VALUES ($1, $2, $3, $4, 'Inline Visualization Task')",
    )
    .bind(inline_task_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(workspace_id)
    .execute(&pool)
    .await
    .unwrap();
    let inline_root_thread_id = Uuid::now_v7().to_string();
    let inline_child_thread_id = Uuid::now_v7().to_string();
    let inline_run_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO runs \
         (id, organization_id, task_id, requested_by, requested_profile_id, workspace_id, \
          status, codex_thread_id) \
         VALUES ($1, $2, $3, $4, $5, $6, 'running', $7)",
    )
    .bind(inline_run_id)
    .bind(first_organization_id)
    .bind(inline_task_id)
    .bind(first_user_id)
    .bind(profile_id)
    .bind(workspace_id)
    .bind(&inline_root_thread_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runtime_agent_projections (
            organization_id, profile_id, workspace_id, root_run_id, thread_id,
            parent_thread_id, source_kind, agent_path, agent_nickname, agent_role
         ) VALUES ($1, $2, $3, $4, $5, $6,
                   'thread_spawn', '/root/data', 'Inline Child', 'data_agent')",
    )
    .bind(first_organization_id)
    .bind(profile_id)
    .bind(workspace_id)
    .bind(inline_run_id)
    .bind(&inline_child_thread_id)
    .bind(&inline_root_thread_id)
    .execute(&pool)
    .await
    .unwrap();

    let inline_root_dir = visualization_thread_dir(codex_home.path(), &inline_root_thread_id);
    let inline_child_dir = visualization_thread_dir(codex_home.path(), &inline_child_thread_id);
    std::fs::create_dir_all(&inline_root_dir).unwrap();
    std::fs::create_dir_all(&inline_child_dir).unwrap();
    std::fs::write(
        inline_root_dir.join("root.html"),
        "<div id=\"inline-root\">root visualization</div>",
    )
    .unwrap();
    let inline_child_png = b"\x89PNG\r\n\x1a\ninline-child";
    std::fs::write(inline_child_dir.join("child.png"), inline_child_png).unwrap();

    let other_profile_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO profiles (id, organization_id, owner_user_id, runtime_key, name) \
         VALUES ($1, $2, $3, 'security-other-profile', 'Other Security Profile')",
    )
    .bind(other_profile_id)
    .bind(first_organization_id)
    .bind(first_user_id)
    .execute(&pool)
    .await
    .unwrap();
    let other_workspace_id = Uuid::now_v7();
    let other_workspace_root = runner_root.path().join("other-profile-workspace");
    std::fs::create_dir_all(&other_workspace_root).unwrap();
    sqlx::query(
        "INSERT INTO workspaces \
         (id, organization_id, project_id, profile_id, created_by, root_path, state, \
          source_ref, kind, name) \
         VALUES ($1, $2, $3, $4, $5, $6, 'ready', 'main', 'main', 'Other Profile Workspace')",
    )
    .bind(other_workspace_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(other_profile_id)
    .bind(first_user_id)
    .bind(other_workspace_root.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspace_grants \
         (workspace_id, organization_id, user_id, profile_id, role) \
         VALUES ($1, $2, $3, $4, 'owner')",
    )
    .bind(other_workspace_id)
    .bind(first_organization_id)
    .bind(first_user_id)
    .bind(other_profile_id)
    .execute(&pool)
    .await
    .unwrap();
    let other_profile_task_id = Uuid::now_v7();
    let other_profile_thread_id = Uuid::now_v7().to_string();
    let other_profile_run_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO tasks (id, organization_id, project_id, workspace_id, title) \
         VALUES ($1, $2, $3, $4, 'Other Profile Task')",
    )
    .bind(other_profile_task_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(other_workspace_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs \
         (id, organization_id, task_id, requested_by, requested_profile_id, workspace_id, \
          status, codex_thread_id) \
         VALUES ($1, $2, $3, $4, $5, $6, 'running', $7)",
    )
    .bind(other_profile_run_id)
    .bind(first_organization_id)
    .bind(other_profile_task_id)
    .bind(first_user_id)
    .bind(other_profile_id)
    .bind(other_workspace_id)
    .bind(&other_profile_thread_id)
    .execute(&pool)
    .await
    .unwrap();

    let non_ready_workspace_id = Uuid::now_v7();
    let non_ready_task_id = Uuid::now_v7();
    let non_ready_run_id = Uuid::now_v7();
    let non_ready_thread_id = Uuid::now_v7().to_string();
    let non_ready_workspace_root = runner_root.path().join("non-ready-workspace");
    sqlx::query(
        "INSERT INTO workspaces \
         (id, organization_id, project_id, profile_id, created_by, root_path, state, \
          source_ref, kind, name) \
         VALUES ($1, $2, $3, $4, $5, $6, 'creating', 'main', 'main', 'Non-ready Workspace')",
    )
    .bind(non_ready_workspace_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(profile_id)
    .bind(first_user_id)
    .bind(non_ready_workspace_root.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspace_grants \
         (workspace_id, organization_id, user_id, profile_id, role) \
         VALUES ($1, $2, $3, $4, 'owner')",
    )
    .bind(non_ready_workspace_id)
    .bind(first_organization_id)
    .bind(first_user_id)
    .bind(profile_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO tasks (id, organization_id, project_id, workspace_id, title) \
         VALUES ($1, $2, $3, $4, 'Non-ready Visualization Task')",
    )
    .bind(non_ready_task_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(non_ready_workspace_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs \
         (id, organization_id, task_id, requested_by, requested_profile_id, workspace_id, \
          status, codex_thread_id) \
         VALUES ($1, $2, $3, $4, $5, $6, 'running', $7)",
    )
    .bind(non_ready_run_id)
    .bind(first_organization_id)
    .bind(non_ready_task_id)
    .bind(first_user_id)
    .bind(profile_id)
    .bind(non_ready_workspace_id)
    .bind(&non_ready_thread_id)
    .execute(&pool)
    .await
    .unwrap();

    let child_approval_id = approval_service
        .capture_message(
            runtime_instance_id,
            &json!({
                "id": 770,
                "method": "mcpServer/elicitation/request",
                "params": {
                    "threadId": "approval-child-thread",
                    "turnId": "child-turn-1",
                    "serverName": "supply_chain_data",
                    "mode": "form",
                    "message": "Provide the route method and detour factor.",
                    "requestedSchema": {
                        "type": "object",
                        "properties": {
                            "method": {
                                "type": "string",
                                "enum": ["curve", "navigation"],
                                "default": "curve"
                            },
                            "factor": {
                                "type": "number",
                                "minimum": 1.0,
                                "maximum": 2.0,
                                "default": 1.2
                            }
                        },
                        "required": ["method", "factor"]
                    }
                }
            }),
        )
        .await
        .unwrap()
        .expect("captured child Thread approval");
    let child_run_id: Uuid = sqlx::query_scalar("SELECT run_id FROM approvals WHERE id = $1")
        .bind(child_approval_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(child_run_id, first_run_id);
    let generic_pending = call(&app, authenticated("GET", "/api/approvals", &first_token)).await;
    assert!(!generic_pending
        .1
        .to_string()
        .contains(&child_approval_id.to_string()));
    let child_forms = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/mcp-form-requests"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(child_forms.0, StatusCode::OK);
    assert_eq!(child_forms.1[0]["id"], child_approval_id.to_string());
    assert_eq!(child_forms.1[0]["source"]["kind"], "agent");
    assert_eq!(child_forms.1[0]["source"]["displayTitle"], "Wanwan");
    assert!(!child_forms.1.to_string().contains("ResourceRef"));
    assert_eq!(child_forms.1[0]["fields"].as_array().unwrap().len(), 2);
    assert!(!child_forms.1.to_string().contains("requestedSchema"));
    assert!(!child_forms.1.to_string().contains("approval-child-thread"));
    let accepted_child_form = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/approvals/{child_approval_id}/mcp-form"),
            &first_token,
            json!({
                "action": "accept",
                "content": { "method": "navigation", "factor": 1.35 },
                "version": 0
            }),
        ),
    )
    .await;
    assert_eq!(accepted_child_form.0, StatusCode::NO_CONTENT);
    let child_audit: Value = sqlx::query_scalar(
        "SELECT metadata FROM audit_log WHERE target_id = $1 AND action = 'approval.decide'",
    )
    .bind(child_approval_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(child_audit["action"], "accept");
    assert_eq!(child_audit["fieldCount"], 2);
    assert!(!child_audit.to_string().contains("navigation"));
    assert!(!child_audit.to_string().contains("1.35"));

    for (request_id, params) in [
        (
            772,
            json!({
                "threadId": "approval-thread",
                "turnId": "turn-unsupported-mode",
                "itemId": "item-unsupported-mode",
                "mode": "json",
                "message": "unsupported mode must remain bounded"
            }),
        ),
        (
            773,
            json!({
                "threadId": "approval-thread",
                "turnId": "turn-missing-mode",
                "itemId": "item-missing-mode",
                "message": "missing mode must remain bounded"
            }),
        ),
    ] {
        let unavailable_id = approval_service
            .capture_message(
                runtime_instance_id,
                &json!({
                    "id": request_id,
                    "method": "mcpServer/elicitation/request",
                    "params": params
                }),
            )
            .await
            .unwrap()
            .expect("captured generic unsupported MCP approval");
        let unsupported_pending = call(
            &app,
            authenticated(
                "GET",
                &format!("/api/runs/{first_run_id}/approval-requests"),
                &first_token,
            ),
        )
        .await;
        assert_eq!(unsupported_pending.0, StatusCode::OK);
        let unsupported = unsupported_pending
            .1
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["id"] == unavailable_id.to_string())
            .expect("unsupported MCP approval projection");
        assert_eq!(unsupported["subject"]["kind"], "unavailable");
        assert_eq!(unsupported["subject"]["reason"], "unsupported_request");
        assert!(!unsupported.to_string().contains("unsupported mode"));
        let canceled = call(
            &app,
            authenticated_json(
                "POST",
                &format!("/api/approvals/{unavailable_id}/decision"),
                &first_token,
                json!({"decision": "cancel", "version": 0}),
            ),
        )
        .await;
        assert_eq!(canceled.0, StatusCode::NO_CONTENT);
    }

    let unsafe_url_id = approval_service
        .capture_message(
            runtime_instance_id,
            &json!({
                "id": 774,
                "method": "mcpServer/elicitation/request",
                "params": {
                    "threadId": "approval-thread",
                    "turnId": "turn-unsafe-url",
                    "itemId": "item-unsafe-url",
                    "serverName": "maps",
                    "mode": "url",
                    "url": "https://example.invalid/credential?token=redacted"
                }
            }),
        )
        .await
        .unwrap()
        .expect("captured unsafe URL approval");
    let unsafe_url_pending = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/approval-requests"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(unsafe_url_pending.0, StatusCode::OK);
    let unsafe_url = unsafe_url_pending
        .1
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == unsafe_url_id.to_string())
        .expect("unsafe URL approval projection");
    assert_eq!(unsafe_url["subject"]["kind"], "url");
    assert_eq!(unsafe_url["subject"]["available"], false);
    assert!(unsafe_url["subject"].get("url").is_none());
    assert!(!unsafe_url.to_string().contains("credential?token=redacted"));
    let unsafe_url_accept = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/approvals/{unsafe_url_id}/decision"),
            &first_token,
            json!({"decision": "accept", "version": 0}),
        ),
    )
    .await;
    assert_eq!(unsafe_url_accept.0, StatusCode::UNPROCESSABLE_ENTITY);
    let unsafe_url_cancel = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/approvals/{unsafe_url_id}/decision"),
            &first_token,
            json!({"decision": "cancel", "version": 0}),
        ),
    )
    .await;
    assert_eq!(unsafe_url_cancel.0, StatusCode::NO_CONTENT);

    let malformed_permissions_id = approval_service
        .capture_message(
            runtime_instance_id,
            &json!({
                "id": 775,
                "method": "item/permissions/requestApproval",
                "params": {
                    "threadId": "approval-thread",
                    "turnId": "turn-invalid-permissions",
                    "itemId": "item-invalid-permissions",
                    "permissions": {"network": {"enabled": true, "scope": "all"}}
                }
            }),
        )
        .await
        .unwrap()
        .expect("captured malformed permissions approval");
    let malformed_permissions_pending = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/approval-requests"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(malformed_permissions_pending.0, StatusCode::OK);
    let malformed_permissions = malformed_permissions_pending
        .1
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == malformed_permissions_id.to_string())
        .expect("malformed permissions projection");
    assert_eq!(malformed_permissions["subject"]["kind"], "unavailable");
    assert_eq!(
        malformed_permissions["subject"]["reason"],
        "invalid_permissions"
    );
    let malformed_permissions_accept = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/approvals/{malformed_permissions_id}/decision"),
            &first_token,
            json!({"decision": "accept", "version": 0}),
        ),
    )
    .await;
    assert_eq!(
        malformed_permissions_accept.0,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    let malformed_permissions_cancel = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/approvals/{malformed_permissions_id}/decision"),
            &first_token,
            json!({"decision": "cancel", "version": 0}),
        ),
    )
    .await;
    assert_eq!(malformed_permissions_cancel.0, StatusCode::NO_CONTENT);

    let retry_approval_id = approval_service
        .capture_message(
            runtime_instance_id,
            &json!({
                "id": 78,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "approval-thread",
                    "turnId": "turn-2",
                    "itemId": "item-2",
                    "command": "git diff"
                }
            }),
        )
        .await
        .unwrap()
        .expect("captured retryable approval");
    let actor = ApprovalActor {
        user_id: first_user_id,
        organization_id: first_organization_id,
    };
    let first_dispatch = approval_service
        .begin_decision(
            actor,
            retry_approval_id,
            runtime_instance_id,
            DecideApprovalRequest {
                decision: ApprovalDecision::Accept,
                version: 0,
            },
        )
        .await
        .expect("begin first delivery");
    approval_service
        .mark_delivery_unknown(actor, &first_dispatch)
        .await
        .expect("mark uncertain delivery");
    let delivery_unknown = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/approval-requests"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(delivery_unknown.0, StatusCode::OK);
    let retry_pending = delivery_unknown
        .1
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["id"] == retry_approval_id.to_string())
        .expect("delivery-unknown approval projection");
    assert_eq!(retry_pending["state"], "delivery_unknown");
    assert_eq!(retry_pending["attemptedDecision"], "accept");
    assert_eq!(retry_pending["version"], 2);
    let retry_dispatch = approval_service
        .begin_decision(
            actor,
            retry_approval_id,
            runtime_instance_id,
            DecideApprovalRequest {
                decision: ApprovalDecision::Accept,
                version: 2,
            },
        )
        .await
        .expect("retry the same uncertain decision");
    approval_service
        .complete_decision(actor, &retry_dispatch)
        .await
        .expect("complete retried decision");
    let terminal = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/approval-requests"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(terminal.0, StatusCode::OK);
    assert!(!terminal
        .1
        .to_string()
        .contains(&retry_approval_id.to_string()));

    let next_runtime_instance_id = Uuid::now_v7();
    let reused_request_id = approval_service
        .capture_message(
            next_runtime_instance_id,
            &json!({
                "id": 77,
                "method": "item/commandExecution/requestApproval",
                "params": {
                    "threadId": "approval-thread",
                    "turnId": "turn-after-restart",
                    "itemId": "item-after-restart",
                    "command": "git status"
                }
            }),
        )
        .await
        .unwrap()
        .expect("request ids may be reused by a new Runtime instance");
    assert_ne!(reused_request_id, approval_id);
    assert_eq!(
        approval_service
            .cancel_stale_runtime_requests(runtime_instance_id)
            .await
            .expect("cancel requests from another Runtime instance"),
        1
    );
    let restarted_state: String = sqlx::query_scalar("SELECT state FROM approvals WHERE id = $1")
        .bind(reused_request_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(restarted_state, "cancelled");

    let second_user_id = Uuid::now_v7();
    let second_organization_id = Uuid::now_v7();
    let second_token = "second-session-token";
    sqlx::query(
        "INSERT INTO users (id, username, name, email, password_hash, role) \
         VALUES ($1, 'second-owner', 'Second Owner', 'second@example.invalid', $2, 'owner')",
    )
    .bind(second_user_id)
    .bind(hash_password("second-password").unwrap())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, 'Second', $2)")
        .bind(second_organization_id)
        .bind(format!("second-{second_organization_id}"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO memberships (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(second_organization_id)
    .bind(second_user_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO memberships (organization_id, user_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(second_organization_id)
    .bind(first_user_id)
    .execute(&pool)
    .await
    .unwrap();
    let argon_login = call(
        &app,
        Request::post("/api/sessions")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "username": "second-owner",
                    "password": "second-password",
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(argon_login.0, StatusCode::OK);

    let legacy_password_hash = "a".repeat(64);
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(&legacy_password_hash)
        .bind(second_user_id)
        .execute(&pool)
        .await
        .unwrap();
    let legacy_login = call(
        &app,
        Request::post("/api/sessions")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "username": "second-owner",
                    "password": "second-password",
                })
                .to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(legacy_login.0, StatusCode::UNAUTHORIZED);
    let stored_legacy_password_hash: String =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1")
            .bind(second_user_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored_legacy_password_hash, legacy_password_hash);
    sqlx::query(
        "INSERT INTO sessions (user_id, organization_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '1 hour')",
    )
    .bind(second_user_id)
    .bind(second_organization_id)
    .bind(hex::encode(Sha256::digest(second_token.as_bytes())))
    .execute(&pool)
    .await
    .unwrap();

    let second_project = call(
        &app,
        authenticated_json(
            "POST",
            "/api/projects",
            second_token,
            json!({"name": "Second Project", "git_url": "https://example.invalid/second.git"}),
        ),
    )
    .await;
    assert_eq!(second_project.0, StatusCode::OK);

    let second_list = call(&app, authenticated("GET", "/api/projects", second_token)).await;
    assert_eq!(second_list.0, StatusCode::OK);
    assert_eq!(second_list.1.as_array().unwrap().len(), 1);
    assert_eq!(second_list.1[0]["name"], "Second Project");

    let (root_status, root_headers, root_body) = raw_call(
        &app,
        authenticated(
            "GET",
            &format!("/api/threads/{inline_root_thread_id}/inline-visualizations/root.html"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(root_status, StatusCode::OK);
    assert_eq!(root_headers["content-type"], "text/html; charset=utf-8");
    assert_eq!(root_headers["cache-control"], "private, no-store");
    assert_eq!(root_headers["x-content-type-options"], "nosniff");
    assert!(String::from_utf8_lossy(&root_body).contains("inline-root"));
    assert!(!String::from_utf8_lossy(&root_body).contains(&codex_home.path().display().to_string()));

    let (child_status, child_headers, child_body) = raw_call(
        &app,
        authenticated(
            "GET",
            &format!("/api/threads/{inline_child_thread_id}/inline-visualizations/child.png"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(child_status, StatusCode::OK);
    assert_eq!(child_headers["content-type"], "image/png");
    assert_eq!(child_body, inline_child_png);
    assert!(
        !String::from_utf8_lossy(&child_body).contains(&codex_home.path().display().to_string())
    );

    let unknown_thread_id = Uuid::now_v7().to_string();
    for (label, thread_id) in [
        ("other-profile", other_profile_thread_id.as_str()),
        ("unknown-thread", unknown_thread_id.as_str()),
        ("non-ready-workspace", non_ready_thread_id.as_str()),
    ] {
        let response = call(
            &app,
            authenticated(
                "GET",
                &format!("/api/threads/{thread_id}/inline-visualizations/root.html"),
                &first_token,
            ),
        )
        .await;
        assert_eq!(response.0, StatusCode::NOT_FOUND, "{label}");
        assert!(!response
            .1
            .to_string()
            .contains(&codex_home.path().display().to_string()));
    }

    let cross_tenant = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/projects/{first_project_id}"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant.0, StatusCode::NOT_FOUND);

    let cross_tenant_inline = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/threads/{inline_root_thread_id}/inline-visualizations/root.html"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_inline.0, StatusCode::NOT_FOUND);
    assert!(!cross_tenant_inline
        .1
        .to_string()
        .contains(&codex_home.path().display().to_string()));

    let cross_tenant_asset = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/workspaces/{workspace_id}/assets?path=icon.png"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_asset.0, StatusCode::NOT_FOUND);
    let cross_tenant_file_list = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/workspaces/{workspace_id}/files"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_file_list.0, StatusCode::NOT_FOUND);
    let cross_tenant_file_read = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/workspaces/{workspace_id}/files/content?path=icon.png"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_file_read.0, StatusCode::NOT_FOUND);
    let cross_tenant_file_delete = call(
        &app,
        authenticated_json(
            "DELETE",
            &format!("/api/workspaces/{workspace_id}/files"),
            second_token,
            json!({ "path": "icon.png" }),
        ),
    )
    .await;
    assert_eq!(cross_tenant_file_delete.0, StatusCode::NOT_FOUND);
    let cross_tenant_artifact = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/artifacts/{artifact_id}/content"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_artifact.0, StatusCode::NOT_FOUND);
    let cross_tenant_agent_executions = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/agent-executions"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_agent_executions.0, StatusCode::NOT_FOUND);
    let cross_tenant_mcp_forms = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/mcp-form-requests"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_mcp_forms.0, StatusCode::NOT_FOUND);
    let cross_tenant_approval_requests = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/approval-requests"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_approval_requests.0, StatusCode::NOT_FOUND);
    let cross_tenant_mcp_form_response = call(
        &app,
        authenticated_json(
            "POST",
            &format!("/api/approvals/{child_approval_id}/mcp-form"),
            second_token,
            json!({ "action": "cancel", "version": 2 }),
        ),
    )
    .await;
    assert_eq!(cross_tenant_mcp_form_response.0, StatusCode::NOT_FOUND);
    let cross_tenant_agent_history = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/agents/approval-child-thread/turns"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_agent_history.0, StatusCode::NOT_FOUND);

    let legacy_runtime = call(
        &app,
        authenticated_json(
            "POST",
            "/api/rpc",
            second_token,
            json!({"method": "list_workspaces", "params": {}}),
        ),
    )
    .await;
    assert_eq!(legacy_runtime.0, StatusCode::NOT_FOUND);

    let switched = call(
        &app,
        authenticated_json(
            "PUT",
            "/api/sessions/organization",
            &first_token,
            json!({"organization_id": second_organization_id}),
        ),
    )
    .await;
    assert_eq!(switched.0, StatusCode::OK);
    assert_eq!(switched.1["role"], "member");

    let member_create = call(
        &app,
        authenticated_json(
            "POST",
            "/api/projects",
            &first_token,
            json!({"name": "Denied", "git_url": "https://example.invalid/denied.git"}),
        ),
    )
    .await;
    assert_eq!(member_create.0, StatusCode::FORBIDDEN);

    let logout = app
        .clone()
        .oneshot(authenticated(
            "DELETE",
            "/api/sessions/current",
            &first_token,
        ))
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::NO_CONTENT);
    assert!(logout
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("session_token=;") && value.contains("Max-Age=0")));
    let revoked_session = call(&app, authenticated("GET", "/api/me", &first_token)).await;
    assert_eq!(revoked_session.0, StatusCode::UNAUTHORIZED);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/api/events/ws"))
        .await
        .unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            json!({"type": "authenticate", "token": second_token})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let ready = socket.next().await.unwrap().unwrap().into_text().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&ready).unwrap()["type"],
        "ready"
    );
    state
        .event_bus
        .send(open_web_codex_platform_store::LiveEvent {
            organization_id: first_organization_id,
            payload: br#"{"type":"run.event","event":{"sequence":1}}"#.to_vec(),
        })
        .unwrap();
    state
        .event_bus
        .send(open_web_codex_platform_store::LiveEvent {
            organization_id: second_organization_id,
            payload: br#"{"type":"run.event","event":{"sequence":2}}"#.to_vec(),
        })
        .unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .into_text()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&event).unwrap()["event"]["sequence"],
        2
    );
    server.abort();

    let password_hash: String = sqlx::query(
        "SELECT password_hash FROM users WHERE id = $1 AND id IN \
         (SELECT user_id FROM memberships WHERE organization_id = $2)",
    )
    .bind(first_user_id)
    .bind(first_organization_id)
    .fetch_one(&pool)
    .await
    .unwrap()
    .get("password_hash");
    assert!(password_hash.starts_with("$argon2id$"));

    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(first_run_id)
        .execute(&pool)
        .await
        .unwrap();
    let retained_artifact_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM artifacts artifact
         JOIN artifact_task_grants artifact_grant
           ON artifact_grant.artifact_id = artifact.id
         JOIN artifact_provenance provenance
           ON provenance.artifact_id = artifact.id
         WHERE artifact.id = $1
           AND artifact_grant.task_id = $2
           AND provenance.producer_run_id = $3",
    )
    .bind(artifact_id)
    .bind(first_task_id)
    .bind(first_run_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(retained_artifact_count, 1);
}

/// Creates and migrates a fresh database on the `TEST_DATABASE_URL` server.
///
/// Each test bootstraps its own installation, and bootstrap succeeds only
/// while no user exists, so tests that share one database conflict when run
/// in parallel.
async fn isolated_database_pool() -> PgPool {
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let server_options: PgConnectOptions = database_url.parse().expect("parse TEST_DATABASE_URL");
    let database_name = format!("owc_security_{}", Uuid::now_v7().simple());
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(server_options.clone())
        .await
        .expect("connect disposable PostgreSQL server");
    sqlx::query(AssertSqlSafe(format!(
        "CREATE DATABASE \"{database_name}\""
    )))
    .execute(&admin)
    .await
    .expect("create isolated test database");
    admin.close().await;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(server_options.database(&database_name))
        .await
        .expect("connect isolated test database");
    open_web_codex_platform_store::migrate::run(&pool)
        .await
        .expect("migrate database");
    pool
}

async fn call(app: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = app.clone().oneshot(request).await.expect("HTTP response");
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body");
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or_else(|error| {
            panic!(
                "JSON response for status {status} ({error}): {}",
                String::from_utf8_lossy(&body)
            )
        })
    };
    (status, value)
}

async fn raw_call(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Vec<u8>) {
    let response = app.clone().oneshot(request).await.expect("HTTP response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("response body");
    (status, headers, body.into_iter().collect())
}

async fn call_with_headers(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let response = app.clone().oneshot(request).await.expect("HTTP response");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("response body");
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or_else(|error| {
            panic!(
                "JSON response for status {status} ({error}): {}",
                String::from_utf8_lossy(&body)
            )
        })
    };
    (status, headers, value)
}

fn fixture_git(cwd: &Path, args: &[&str]) {
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

fn visualization_thread_dir(codex_home: &Path, thread_id: &str) -> PathBuf {
    let uuid = Uuid::parse_str(thread_id).unwrap();
    let timestamp = uuid.get_timestamp().unwrap();
    let (seconds, nanos) = timestamp.to_unix();
    let created_at = DateTime::from_timestamp(i64::try_from(seconds).unwrap(), nanos).unwrap();
    codex_home
        .join("visualizations")
        .join(created_at.format("%Y/%m/%d").to_string())
        .join(thread_id)
}

fn authenticated(method: &str, uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn authenticated_json(method: &str, uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}
