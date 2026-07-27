use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use axum::body::{to_bytes, Body};
use axum::http::{HeaderMap, Request, StatusCode};
use axum::Router;
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
use sqlx::postgres::PgPoolOptions;
use sqlx::Row;
use tower::ServiceExt;
use uuid::Uuid;

use crate::ensure_transitional_profile_binding;
use crate::routes::{self, RuntimeProfileBinding};

#[tokio::test]
#[ignore = "requires TEST_DATABASE_URL pointing at a disposable PostgreSQL database"]
async fn organization_and_profile_authorization_prevent_cross_tenant_access() {
    let database_url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await
        .expect("connect disposable PostgreSQL database");
    open_web_codex_platform_store::migrate::run(&pool)
        .await
        .expect("migrate database");

    let profile = RuntimeProfileBinding {
        runtime_key: "security-test-profile".to_string(),
        name: "Security Test Profile".to_string(),
        codex_home: None,
        capabilities: routes::RuntimeCapabilityState::default(),
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
    sqlx::query(
        "INSERT INTO tasks (id, organization_id, project_id, title) \
         VALUES ($1, $2, $3, 'Approval Task')",
    )
    .bind(first_task_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO runs (id, organization_id, task_id, status, codex_thread_id) \
         VALUES ($1, $2, $3, 'running', 'approval-thread')",
    )
    .bind(first_run_id)
    .bind(first_organization_id)
    .bind(first_task_id)
    .execute(&pool)
    .await
    .unwrap();

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
         (id, organization_id, project_id, profile_id, run_id, root_path, state, source_ref, head_commit, branch_name) \
         VALUES ($1, $2, $3, $4, $5, $6, 'ready', 'main', $7, 'main')",
    )
    .bind(workspace_id)
    .bind(first_organization_id)
    .bind(Uuid::parse_str(&first_project_id).unwrap())
    .bind(profile_id)
    .bind(first_run_id)
    .bind(checkout.root.to_string_lossy().as_ref())
    .bind(&checkout.head_commit)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE runs SET workspace_id = $1 WHERE id = $2")
        .bind(workspace_id)
        .bind(first_run_id)
        .execute(&pool)
        .await
        .unwrap();
    let artifact_id = Uuid::now_v7();
    let artifact_bytes = br#"{"type":"FeatureCollection","features":[]}"#;
    sqlx::query(
        "INSERT INTO reply_artifacts (
            id, organization_id, run_id, thread_id, turn_id, producer_item_id,
            source_server, source_uri, mime_type, content, state
         ) VALUES (
            $1, $2, $3, 'approval-thread', 'turn-map', 'item-data',
            'map_utils', 'maps-data://geojson/map-data-security',
            'application/geo+json', $4, 'ready'
         )",
    )
    .bind(artifact_id)
    .bind(first_organization_id)
    .bind(first_run_id)
    .bind(artifact_bytes.as_slice())
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO run_events (
            run_id, event_type, projection_version, thread_id, turn_id, item_id, payload
         ) VALUES (
            $1, 'codex.item.completed', 1, 'approval-thread',
            'turn-map-producer', 'item-inline-map', '{}'::jsonb
         )",
    )
    .bind(first_run_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO inline_visualization_artifacts (
            organization_id, run_id, thread_id, producer_turn_id, producer_item_id,
            artifact_ref, renderer_kind, renderer_payload
         ) VALUES (
            $1, $2, 'approval-thread', 'turn-map-producer', 'item-inline-map',
            'map-cross-turn', 'map.v3', $3
         )",
    )
    .bind(first_organization_id)
    .bind(first_run_id)
    .bind(json!({
        "type": "card",
        "kind": "map.v3",
        "id": "map-cross-turn",
        "title": "Cross-turn map",
        "intent": "test",
        "status": "ready",
        "sources": {},
        "layers": []
    }))
    .execute(&pool)
    .await
    .unwrap();
    let cross_turn_artifacts = crate::event_projection::resolve_inline_artifacts(
        &pool,
        first_run_id,
        "Before\n\n::codex-inline-vis{artifact=\"map-cross-turn\"}\n\nAfter",
    )
    .await
    .unwrap();
    assert_eq!(cross_turn_artifacts.len(), 1);
    assert_eq!(cross_turn_artifacts[0]["ref"], "map-cross-turn");

    let artifact = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/artifacts/{artifact_id}"),
            &first_token,
        ),
    )
    .await;
    assert_eq!(artifact.0, StatusCode::OK);
    assert_eq!(artifact.1["type"], "FeatureCollection");

    let image_response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/runs/{first_run_id}/workspace/assets?path=icon.png"
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
                "/api/runs/{first_run_id}/workspace/assets?path=icon.png"
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
    assert_eq!(pending.1[0]["command"], "git status");
    assert!(!pending.1.to_string().contains("/private/server/path"));
    assert!(!pending.1.to_string().contains("\"77\""));

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

    let cross_tenant_asset = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/workspace/assets?path=icon.png"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_asset.0, StatusCode::NOT_FOUND);
    let cross_tenant_artifact = call(
        &app,
        authenticated(
            "GET",
            &format!("/api/runs/{first_run_id}/artifacts/{artifact_id}"),
            second_token,
        ),
    )
    .await;
    assert_eq!(cross_tenant_artifact.0, StatusCode::NOT_FOUND);

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
        serde_json::from_slice(&body).expect("JSON response")
    };
    (status, value)
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
        serde_json::from_slice(&body).expect("JSON response")
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
