use std::path::{Path, PathBuf};

use open_web_codex_adapter::real::RealCodexAdapter;
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter, TurnOptions};
use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig, ProfileHostEvent};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::time::{timeout, Duration};

async fn start_mock_responses_server() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock Responses server");
    let address = listener.local_addr().expect("mock server address");
    let body = [
        json!({
            "type": "response.created",
            "response": { "id": "response-1" },
        }),
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "id": "message-1",
                "content": [{ "type": "output_text", "text": "Done" }],
            },
        }),
        json!({
            "type": "response.completed",
            "response": {
                "id": "response-1",
                "usage": {
                    "input_tokens": 0,
                    "input_tokens_details": null,
                    "output_tokens": 0,
                    "output_tokens_details": null,
                    "total_tokens": 0,
                },
            },
        }),
    ]
    .into_iter()
    .map(|event| {
        let kind = event["type"].as_str().expect("response event type");
        format!("event: {kind}\ndata: {event}\n\n")
    })
    .collect::<String>();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut request = vec![0_u8; 32 * 1024];
                let _ = stream.read(&mut request).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                let _ = stream.write_all(response.as_bytes()).await;
                let _ = stream.shutdown().await;
            });
        }
    });
    (format!("http://{address}"), task)
}

async fn wait_for_turn_context_window(
    events: &mut broadcast::Receiver<ProfileHostEvent>,
    thread_id: &str,
    turn_id: &str,
) -> i64 {
    timeout(Duration::from_secs(10), async {
        let mut context_window = None;
        let mut completed = false;
        loop {
            let event = events.recv().await.expect("Profile Host event").message;
            if event["params"]["threadId"] != thread_id {
                continue;
            }
            if event["method"] == "thread/tokenUsage/updated"
                && event["params"]["turnId"] == turn_id
            {
                context_window = event["params"]["tokenUsage"]["modelContextWindow"].as_i64();
            } else if event["method"] == "turn/completed"
                && event["params"]["turn"]["id"] == turn_id
            {
                completed = true;
            }
            if let (true, Some(context_window)) = (completed, context_window) {
                return context_window;
            }
        }
    })
    .await
    .expect("turn completion and context window")
}

fn host_config(codex_bin: &Path, home: &Path, workspace: &Path) -> ProfileHostConfig {
    ProfileHostConfig::new("real-adapter-profile", home, workspace).with_codex_bin(codex_bin)
}

/// Exercises the next-Turn refresh path against the checked-out Codex binary.
/// Run with:
///
/// `CODEX_BIN=/absolute/path/to/codex cargo test -p open-web-codex-adapter --test real_app_server -- --ignored`
#[tokio::test]
#[ignore = "requires a real Codex CLI binary"]
async fn next_turn_restarts_runtime_and_resumes_the_same_thread() {
    let codex_bin = PathBuf::from(std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set"));
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/web workspace root")
        .canonicalize()
        .expect("canonical workspace");
    let profile_home = tempfile::tempdir().expect("temporary Profile home");
    let (model_server_uri, model_server) = start_mock_responses_server().await;
    std::fs::write(
        profile_home.path().join("config.toml"),
        format!(
            r#"
model = "mock-model"
model_provider = "mock_provider"
sandbox_mode = "read-only"

[model_providers.mock_provider]
name = "Adapter context refresh provider"
base_url = "{}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
models = [{{ model_id = "mock-model", context_window = 25600 }}]
"#,
            model_server_uri
        ),
    )
    .expect("write Profile config");

    let host = ProfileHost::spawn(host_config(
        &codex_bin,
        profile_home.path(),
        &workspace_root,
    ))
    .await
    .expect("spawn Profile Host");
    let adapter =
        RealCodexAdapter::from_host(host.clone(), "workspace-one", workspace_root.clone())
            .expect("construct real adapter");
    let workspace = AuthorizedWorkspace {
        id: "workspace-one".to_string(),
        root: workspace_root.clone(),
    };
    let mut events = host.subscribe();
    let started = adapter
        .start_thread(&workspace, None)
        .await
        .expect("start Thread");
    let first_runtime_instance = host.runtime_instance_id().await;
    let options = TurnOptions {
        access_mode: Some("read-only".to_string()),
        ..TurnOptions::default()
    };
    let first_turn = adapter
        .send_user_message(
            &workspace,
            &started.thread_id,
            "Use the initial context window",
            &options,
        )
        .await
        .expect("start first Turn");
    let first_turn_id = first_turn["turnId"].as_str().expect("first Turn id");
    assert_eq!(
        wait_for_turn_context_window(&mut events, &started.thread_id, first_turn_id).await,
        24_320
    );
    let persisted_turns = adapter
        .list_thread_turns(&workspace, None, &started.thread_id)
        .await
        .expect("read paginated Thread history");
    assert_eq!(persisted_turns.len(), 1);
    assert!(
        persisted_turns[0]["items"]
            .as_array()
            .is_some_and(|items| !items.is_empty()),
        "paginated Turn items should be joined into the full browser projection"
    );

    host.request(
        "config/batchWrite",
        json!({
            "edits": [{
                "keyPath": "model_providers.\"mock_provider\".models",
                "value": [{
                    "model_id": "mock-model",
                    "context_window": 256000
                }],
                "mergeStrategy": "replace"
            }],
            "reloadUserConfig": true
        }),
    )
    .await
    .expect("persist updated model context");
    host.schedule_restart(host_config(
        &codex_bin,
        profile_home.path(),
        &workspace_root,
    ))
    .await
    .expect("schedule Runtime refresh");

    let second_turn = adapter
        .send_user_message(
            &workspace,
            &started.thread_id,
            "Use the updated context window",
            &options,
        )
        .await
        .expect("resume the same Thread and start its next Turn");
    let second_turn_id = second_turn["turnId"].as_str().expect("second Turn id");
    assert_ne!(host.runtime_instance_id().await, first_runtime_instance);
    assert_eq!(
        wait_for_turn_context_window(&mut events, &started.thread_id, second_turn_id).await,
        243_200
    );

    host.shutdown().await.expect("shutdown Profile Host");
    model_server.abort();
}

#[tokio::test]
#[ignore = "requires a real Codex CLI binary"]
async fn archive_explicitly_abandons_an_unmaterialized_thread() {
    let codex_bin = PathBuf::from(std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set"));
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/web workspace root")
        .canonicalize()
        .expect("canonical workspace");
    let profile_home = tempfile::tempdir().expect("temporary Profile home");
    let config = host_config(&codex_bin, profile_home.path(), &workspace_root);
    let host = ProfileHost::spawn(config.clone())
        .await
        .expect("spawn Profile Host");
    let adapter =
        RealCodexAdapter::from_host(host.clone(), "workspace-one", workspace_root.clone())
            .expect("construct real adapter");
    let workspace = AuthorizedWorkspace {
        id: "workspace-one".to_string(),
        root: workspace_root,
    };

    let started = adapter
        .start_thread(&workspace, None)
        .await
        .expect("start unmaterialized Thread");
    host.schedule_restart(config)
        .await
        .expect("schedule Profile refresh");
    adapter
        .archive_thread(&workspace, None, &started.thread_id)
        .await
        .expect("explicit archive abandons the unmaterialized Thread");
    assert!(host
        .apply_scheduled_restart()
        .await
        .expect("Runtime restart is allowed after explicit abandon"));

    host.shutdown().await.expect("shutdown Profile Host");
}

/// Cold history reads must restore the Task's selected package before they
/// mark the Thread bound. Otherwise Chat history can advertise MCP functions
/// while the authoritative Runtime has no corresponding server/handler.
#[tokio::test]
#[ignore = "requires a real Codex CLI binary and python3"]
async fn cold_resume_restores_selected_package_before_history_and_turns() {
    use open_web_codex_adapter::real::{RootExecutionConfig, ThreadSkillConfig};

    let codex_bin = PathBuf::from(std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set"));
    let temp = tempfile::tempdir().expect("temporary recovery fixture");
    let runner_root = temp.path().canonicalize().unwrap();
    let home = runner_root.join("profile");
    let workspace_root = runner_root.join("workspace");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&workspace_root).unwrap();
    let mcp_script = runner_root.join("recovery_mcp.py");
    std::fs::write(
        &mcp_script,
        r#"
import json, sys
for line in sys.stdin:
    message = json.loads(line)
    if 'id' not in message:
        continue
    method = message['method']
    if method == 'initialize':
        result = {'protocolVersion': message['params']['protocolVersion'],
                  'serverInfo': {'name': 'recovery-probe', 'version': '1'},
                  'capabilities': {'tools': {}}}
    elif method == 'tools/list':
        result = {'tools': [{'name': 'probe', 'description': 'Return recovery evidence',
                            'inputSchema': {'type': 'object', 'properties': {}}}]}
    elif method == 'tools/call':
        result = {'content': [{'type': 'text', 'text': 'recovered-package-tool-executed'}],
                  'isError': False}
    else:
        result = {}
    print(json.dumps({'jsonrpc': '2.0', 'id': message['id'], 'result': result}), flush=True)
"#,
    )
    .unwrap();
    let (model_uri, model_server) = start_mock_responses_server().await;
    std::fs::write(
        home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
model_provider = "mock_provider"
sandbox_mode = "read-only"
[model_providers.mock_provider]
name = "Recovery fixture"
base_url = "{model_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#
        ),
    )
    .unwrap();
    let root_skill = home.join("skills/recovery-root/SKILL.md");
    std::fs::create_dir_all(root_skill.parent().unwrap()).unwrap();
    std::fs::write(&root_skill, "---\nname: recovery-root\ndescription: Recovery test root\n---\nUse the recovery probe for this test.\n").unwrap();
    let config = host_config(&codex_bin, &home, &runner_root);
    let host = ProfileHost::spawn(config.clone())
        .await
        .expect("spawn recovery Runtime");
    let adapter = RealCodexAdapter::from_host_with_root_executions(
        host.clone(),
        "runner",
        runner_root.clone(),
        vec![RootExecutionConfig {
            id: "recovery-package".to_string(),
            skill_config: vec![ThreadSkillConfig {
                name: "recovery-root".into(),
                enabled: true,
                main_prompt: Some(root_skill),
            }],
            runtime_config: json!({
                "mcp_servers.recovery_probe": {
                    "command": "python3", "args": [mcp_script],
                    "required": true, "startup_timeout_sec": 10,
                },
            }),
        }],
        None,
    )
    .unwrap();
    let workspace = AuthorizedWorkspace {
        id: "recovery-workspace".into(),
        root: workspace_root,
    };
    let package = Some("recovery-package");
    let started = adapter.start_thread(&workspace, package).await.unwrap();
    let options = TurnOptions {
        copilot_package_id: package.map(str::to_string),
        ..Default::default()
    };
    let mut events = host.subscribe();
    let turn = adapter
        .send_user_message(
            &workspace,
            &started.thread_id,
            "Persist recovery fixture",
            &options,
        )
        .await
        .unwrap();
    wait_for_turn_context_window(
        &mut events,
        &started.thread_id,
        turn["turnId"].as_str().unwrap(),
    )
    .await;

    // Reproduce the original cold-resume failure: omitting the Task's
    // package starts a plain Runtime session even though history still
    // contains the package's tool definitions/results.
    host.schedule_restart(config.clone()).await.unwrap();
    adapter
        .read_thread(&workspace, None, &started.thread_id)
        .await
        .unwrap();
    let missing = host
        .request_long_running(
            "mcpServer/tool/call",
            json!({
                "threadId": started.thread_id, "server": "recovery_probe",
                "tool": "probe", "arguments": {},
            }),
        )
        .await
        .expect_err("a plain resume must not invent a package MCP server");
    assert!(missing.to_string().contains("recovery_probe"));

    for entry in ["history", "turn"] {
        let previous_instance = host.runtime_instance_id().await;
        host.schedule_restart(config.clone()).await.unwrap();
        if entry == "history" {
            let (read, turns) = tokio::join!(
                adapter.read_thread(&workspace, package, &started.thread_id),
                adapter.list_thread_turns(&workspace, package, &started.thread_id),
            );
            assert_eq!(read.unwrap()["thread"]["id"], started.thread_id);
            assert_eq!(turns.unwrap().len(), 1);
        } else {
            let turn = adapter
                .send_user_message(
                    &workspace,
                    &started.thread_id,
                    "Continue recovered package",
                    &options,
                )
                .await
                .unwrap();
            wait_for_turn_context_window(
                &mut events,
                &started.thread_id,
                turn["turnId"].as_str().unwrap(),
            )
            .await;
        }
        assert_ne!(previous_instance, host.runtime_instance_id().await);
        // Use the official diagnostic Tool call, without re-resuming or
        // injecting config here: only the entry point under test may restore it.
        let result = host
            .request_long_running(
                "mcpServer/tool/call",
                json!({
                    "threadId": started.thread_id, "server": "recovery_probe",
                    "tool": "probe", "arguments": {},
                }),
            )
            .await
            .expect("cold entry restored the actual MCP handler");
        assert!(result
            .to_string()
            .contains("recovered-package-tool-executed"));
        assert!(!result.to_string().contains("\"isError\":true"));
    }
    assert!(adapter
        .read_thread(&workspace, Some("missing-package"), &started.thread_id)
        .await
        .is_err());
    let other = AuthorizedWorkspace {
        id: "another-workspace".into(),
        root: workspace.root.clone(),
    };
    assert!(adapter
        .read_thread(&other, package, &started.thread_id)
        .await
        .is_err());
    host.shutdown().await.unwrap();
    model_server.abort();
}
