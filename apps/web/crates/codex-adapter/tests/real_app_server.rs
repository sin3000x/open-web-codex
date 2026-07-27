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
        .start_thread(&workspace)
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
        .list_thread_turns(&workspace, &started.thread_id)
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
