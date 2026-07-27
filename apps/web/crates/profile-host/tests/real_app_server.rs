use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use open_web_codex_profile_host::{ProfileHost, ProfileHostConfig, ProfileHostState};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

fn temporary_profile_home() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "open-web-codex-real-profile-host-{}-{timestamp}",
        std::process::id()
    ))
}

fn thread_id(response: &Value) -> &str {
    response
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .expect("thread response contains thread.id")
}

async fn spawn_host(codex_bin: &Path, home: &Path, workspace: &Path) -> ProfileHost {
    ProfileHost::spawn(
        ProfileHostConfig::new("real-smoke-profile", home, workspace).with_codex_bin(codex_bin),
    )
    .await
    .expect("spawn native Profile Host")
}

async fn start_mock_responses_server(body: String) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock Responses server");
    let address = listener.local_addr().expect("mock server address");
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

fn completed_response_body() -> String {
    [
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
    .collect()
}

async fn wait_for_turn_context_window(
    events: &mut tokio::sync::broadcast::Receiver<open_web_codex_profile_host::ProfileHostEvent>,
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

/// Exercises the actual Profile Host implementation against the checked-out
/// Codex app-server binary. Run with:
///
/// `CODEX_BIN=/absolute/path/to/codex cargo test -p open-web-codex-profile-host --test real_app_server -- --ignored`
#[tokio::test]
#[ignore = "requires a real Codex CLI binary"]
async fn restarts_with_the_same_profile_and_recovers_a_thread() {
    let codex_bin = PathBuf::from(std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set"));
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/web workspace root")
        .canonicalize()
        .expect("canonical workspace");
    let home = temporary_profile_home();
    std::fs::create_dir_all(&home).expect("create Profile home");
    let response_body = completed_response_body();
    let (model_server_uri, model_server) = start_mock_responses_server(response_body).await;
    std::fs::write(
        home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
model_provider = "mock_provider"
sandbox_mode = "read-only"

[model_providers.mock_provider]
name = "Profile Host smoke provider"
base_url = "{}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
"#,
            model_server_uri
        ),
    )
    .expect("write smoke config");

    let first = spawn_host(&codex_bin, &home, &workspace).await;
    let first_runtime_instance_id = first.runtime_instance_id().await;
    let snapshot = first.snapshot().await;
    assert_eq!(snapshot.state, ProfileHostState::Ready);
    assert!(snapshot.capability_count >= 3);
    let started = first
        .request(
            "thread/start",
            json!({
                "cwd": workspace,
                "approvalPolicy": "never",
                "sandbox": "read-only",
            }),
        )
        .await
        .expect("start thread");
    let expected_thread_id = thread_id(&started).to_string();
    let mut events = first.subscribe();
    let turn = first
        .request(
            "turn/start",
            json!({
                "threadId": expected_thread_id,
                "input": [{ "type": "text", "text": "Persist this thread" }],
            }),
        )
        .await
        .expect("start persistence turn");
    assert!(turn.pointer("/turn/id").and_then(Value::as_str).is_some());
    timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.expect("Profile Host event");
            if event.message["method"] == "turn/completed"
                && event.message["params"]["threadId"] == expected_thread_id
            {
                return;
            }
        }
    })
    .await
    .expect("turn completed before restart");
    let first_turn_page = first
        .request(
            "thread/turns/list",
            json!({
                "threadId": expected_thread_id,
                "limit": 100,
                "sortDirection": "asc",
                "itemsView": "full",
            }),
        )
        .await
        .expect("list persisted Turns before restart");
    assert_eq!(
        first_turn_page
            .get("data")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );
    first
        .restart(
            ProfileHostConfig::new("real-smoke-profile", &home, &workspace)
                .with_codex_bin(&codex_bin),
        )
        .await
        .expect("restart Profile Host in place");
    let second = first;
    assert_ne!(
        second.runtime_instance_id().await,
        first_runtime_instance_id
    );
    let resumed = second
        .request(
            "thread/resume",
            json!({
                "threadId": expected_thread_id,
            }),
        )
        .await
        .expect("resume thread after host restart");
    assert_eq!(thread_id(&resumed), expected_thread_id);
    let recovered = second
        .request(
            "thread/read",
            json!({
                "threadId": expected_thread_id,
                "includeTurns": false,
            }),
        )
        .await
        .expect("read thread after host restart");
    assert_eq!(thread_id(&recovered), expected_thread_id);
    let recovered_turn_page = second
        .request(
            "thread/turns/list",
            json!({
                "threadId": expected_thread_id,
                "limit": 100,
                "sortDirection": "asc",
                "itemsView": "full",
            }),
        )
        .await
        .expect("list persisted Turns after restart");
    assert_eq!(
        recovered_turn_page
            .get("data")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );

    second.shutdown().await.expect("shutdown second host");
    drop(second);
    model_server.abort();
    std::fs::remove_dir_all(home).expect("remove smoke Profile home");
}

#[tokio::test]
#[ignore = "requires a real Codex CLI binary"]
async fn scheduled_restart_applies_updated_model_context_to_next_turn() {
    let codex_bin = PathBuf::from(std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set"));
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/web workspace root")
        .canonicalize()
        .expect("canonical workspace");
    let home = temporary_profile_home();
    std::fs::create_dir_all(&home).expect("create Profile home");
    let (model_server_uri, model_server) =
        start_mock_responses_server(completed_response_body()).await;
    std::fs::write(
        home.join("config.toml"),
        format!(
            r#"
model = "mock-model"
model_provider = "mock_provider"
sandbox_mode = "read-only"

[model_providers.mock_provider]
name = "Profile Host context refresh provider"
base_url = "{}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
models = [{{ model_id = "mock-model", context_window = 25600 }}]
"#,
            model_server_uri
        ),
    )
    .expect("write smoke config");

    let host = spawn_host(&codex_bin, &home, &workspace).await;
    let started = host
        .request(
            "thread/start",
            json!({
                "cwd": workspace,
                "approvalPolicy": "never",
                "sandbox": "read-only",
            }),
        )
        .await
        .expect("start thread");
    let expected_thread_id = thread_id(&started).to_string();
    let mut events = host.subscribe();
    let first_turn = host
        .request(
            "turn/start",
            json!({
                "threadId": expected_thread_id,
                "input": [{ "type": "text", "text": "Use the initial context window" }],
            }),
        )
        .await
        .expect("start first turn");
    let first_turn_id = first_turn["turn"]["id"].as_str().expect("first turn id");
    assert_eq!(
        wait_for_turn_context_window(&mut events, &expected_thread_id, first_turn_id).await,
        24_320
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
    host.schedule_restart(
        ProfileHostConfig::new("real-smoke-profile", &home, &workspace).with_codex_bin(&codex_bin),
    )
    .await
    .expect("schedule model catalog refresh");
    assert!(host
        .apply_scheduled_restart()
        .await
        .expect("apply model catalog refresh"));
    host.request(
        "thread/resume",
        json!({
            "threadId": expected_thread_id,
            "cwd": workspace,
            "approvalPolicy": "never",
        }),
    )
    .await
    .expect("resume thread after model catalog refresh");

    let second_turn = host
        .request(
            "turn/start",
            json!({
                "threadId": expected_thread_id,
                "input": [{ "type": "text", "text": "Use the updated context window" }],
            }),
        )
        .await
        .expect("start second turn");
    let second_turn_id = second_turn["turn"]["id"].as_str().expect("second turn id");
    assert_eq!(
        wait_for_turn_context_window(&mut events, &expected_thread_id, second_turn_id).await,
        243_200
    );

    host.shutdown().await.expect("shutdown Profile Host");
    drop(host);
    model_server.abort();
    std::fs::remove_dir_all(home).expect("remove smoke Profile home");
}
