use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use open_web_codex_adapter::real::RealCodexAdapter;
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter, TurnOptions};
use open_web_codex_profile_host::{
    ProfileHost, ProfileHostConfig, ProfileHostEvent, ProfileStartupFile,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, Semaphore};
use tokio::time::{timeout, Duration};

const DATA_SERVER: &str = "supply_chain_data";
const DISCOVER_SOURCES: &str = "discover_workspace_sources";

#[derive(Clone)]
struct ProbeControl {
    release_root_completion: Arc<Semaphore>,
}

#[derive(Default)]
struct ProbeEvidence {
    child_thread_id: Option<String>,
    child_turn_id: Option<String>,
    root_result: Option<Value>,
    child_result: Option<Value>,
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(4)
        .expect("repository root")
        .to_path_buf()
}

fn probe_tempdir() -> tempfile::TempDir {
    let root = repository_root().join("apps/web/target/workspace-probe-tests");
    std::fs::create_dir_all(&root).expect("create test cache root");
    tempfile::Builder::new()
        .prefix("native-workspace-probe-")
        .tempdir_in(root)
        .expect("create Workspace Probe test root")
}

fn toml_string(path: &std::path::Path) -> String {
    serde_json::to_string(
        path.to_str()
            .expect("Workspace Probe paths must be valid UTF-8"),
    )
    .expect("serialize TOML-compatible string")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreparedDescriptor {
    capability_roots: Vec<PreparedCapabilityRoot>,
}

#[derive(Deserialize)]
struct PreparedCapabilityRoot {
    id: String,
    servers: Vec<PreparedServer>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreparedServer {
    id: String,
    command: PathBuf,
    args: Vec<String>,
    env_bindings: Vec<PreparedEnvironmentBinding>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreparedEnvironmentBinding {
    name: String,
    source: String,
    resolved_root: Option<PathBuf>,
}

fn prepared_descriptor_path() -> PathBuf {
    repository_root().join(
        ".local/open-web-codex/tool-environments/warehouse-network-copilot/copilot-sdk/prepared-tools.v1.json",
    )
}

fn probe_mcp_config(profile_home: &std::path::Path) -> String {
    let document: PreparedDescriptor = serde_json::from_slice(
        &std::fs::read(prepared_descriptor_path()).expect("read prepared Copilot descriptor"),
    )
    .expect("parse prepared Copilot descriptor");
    let capability = document
        .capability_roots
        .into_iter()
        .find(|capability| capability.id == "supply_chain")
        .expect("prepared supply_chain capability root");
    let server = capability
        .servers
        .into_iter()
        .find(|server| server.id == DATA_SERVER)
        .expect("prepared supply_chain_data server");
    let mut environment = Vec::new();
    let mut inherited = Vec::new();
    for binding in server.env_bindings {
        match binding.source.as_str() {
            "profile_home" => environment.push((binding.name, profile_home.to_path_buf())),
            "tool_state_root" => environment.push((
                binding.name,
                profile_home
                    .join(".open-web-codex/mcp-state")
                    .join(&capability.id),
            )),
            "dependency_root" => environment.push((
                binding.name,
                binding
                    .resolved_root
                    .expect("prepared dependency_root binding has resolvedRoot"),
            )),
            "host" => inherited.push(binding.name),
            source => panic!("unsupported prepared environment source {source}"),
        }
    }
    let environment = environment
        .into_iter()
        .map(|(name, value)| format!("{name} = {}", toml_string(&value)))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"[mcp_servers.supply_chain_data]
command = {}
args = {}
enabled = true
required = true
enabled_tools = ["discover_workspace_sources"]
default_tools_approval_mode = "approve"
env_vars = {}

[mcp_servers.supply_chain_data.env]
{}
"#,
        toml_string(&server.command),
        serde_json::to_string(&server.args).expect("serialize prepared args"),
        serde_json::to_string(&inherited).expect("serialize inherited environment"),
        environment,
    )
}

fn probe_child_role(mcp_config: &str) -> ProfileStartupFile {
    let config_toml = format!(
        r#"name = "workspace_probe_child"
description = "Calls Workspace source discovery in a native child Thread."
developer_instructions = '''
You are the child Workspace Probe. Call only the Workspace source discovery tool.
'''

[[skills.config]]
name = "workspace-probe-root"
enabled = false

[[skills.config]]
name = "workspace-probe-child"
enabled = true

{mcp_config}
"#
    );
    ProfileStartupFile::agent_role("workspace_probe_child", config_toml.into_bytes())
        .expect("valid native child Role seed")
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let read = stream.read(&mut chunk).await.expect("read mock request");
        assert_ne!(read, 0, "mock Responses request ended before its body");
        request.extend_from_slice(&chunk[..read]);
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length").then(|| {
                    value
                        .trim()
                        .parse::<usize>()
                        .expect("numeric content length")
                })
            })
            .expect("Responses request content length");
        if request.len() >= header_end + 4 + content_length {
            return String::from_utf8(request).expect("UTF-8 Responses request");
        }
    }
}

fn sse(response_id: &str, item: Value) -> String {
    [
        json!({ "type": "response.created", "response": { "id": response_id } }),
        item,
        json!({
            "type": "response.completed",
            "response": {
                "id": response_id,
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
        format!(
            "event: {}\ndata: {event}\n\n",
            event["type"].as_str().unwrap()
        )
    })
    .collect()
}

fn message(response_id: &str, text: &str) -> String {
    sse(
        response_id,
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "id": format!("{response_id}-message"),
                "content": [{ "type": "output_text", "text": text }],
            },
        }),
    )
}

fn mcp_call(response_id: &str, call_id: &str) -> String {
    sse(
        response_id,
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": call_id,
                "namespace": "mcp__supply_chain_data",
                "name": DISCOVER_SOURCES,
                "arguments": "{}",
            },
        }),
    )
}

fn spawn_call(response_id: &str, country: &str) -> String {
    sse(
        response_id,
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": format!("root-{country}-spawn"),
                "namespace": "collaboration",
                "name": "spawn_agent",
                "arguments": json!({
                    "message": format!("workspace-probe-child-{country}"),
                    "task_name": format!("workspace_probe_{country}"),
                    "agent_type": "workspace_probe_child",
                    "fork_turns": "none",
                }).to_string(),
            },
        }),
    )
}

async fn response_for(request: &str, control: &ProbeControl) -> String {
    for country in ["indonesia", "thailand"] {
        let root_mcp = format!("root-{country}-mcp");
        let child_mcp = format!("child-{country}-mcp");
        let spawned = format!("root-{country}-spawn");
        if request.contains(&format!("workspace-probe-root-steer-{country}")) {
            return message(
                &format!("root-{country}-steered"),
                "Root accepted the steer.",
            );
        }
        if request.contains(&child_mcp) {
            return message(&format!("child-{country}-done"), "Child completed.");
        }
        if request.contains(&spawned) {
            control
                .release_root_completion
                .clone()
                .acquire_owned()
                .await
                .expect("Root completion control is open")
                .forget();
            return message(&format!("root-{country}-done"), "Root completed.");
        }
        if request.contains(&root_mcp) {
            return spawn_call(&format!("root-{country}-spawn"), country);
        }
        if request.contains(&format!("workspace-probe-child-{country}")) {
            return mcp_call(&format!("child-{country}-mcp"), &child_mcp);
        }
        if request.contains(&format!("workspace-probe-root-{country}")) {
            return mcp_call(&format!("root-{country}-mcp"), &root_mcp);
        }
    }
    message("unexpected", "No Workspace Probe action requested.")
}

async fn start_mock_responses_server() -> (String, ProbeControl, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Workspace Probe Responses server");
    let address = listener
        .local_addr()
        .expect("Workspace Probe server address");
    let control = ProbeControl {
        release_root_completion: Arc::new(Semaphore::new(0)),
    };
    let server_control = control.clone();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let control = server_control.clone();
            tokio::spawn(async move {
                let request = read_request(&mut stream).await;
                let body = response_for(&request, &control).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.shutdown().await.unwrap();
            });
        }
    });
    (format!("http://{address}"), control, task)
}

fn thread_id(event: &Value) -> Option<&str> {
    event
        .pointer("/params/threadId")
        .or_else(|| event.pointer("/params/thread/id"))
        .and_then(Value::as_str)
}

fn started_child_thread_id<'a>(event: &'a Value, root_thread_id: &str) -> Option<&'a str> {
    if event["method"] == "thread/started"
        && event
            .pointer("/params/thread/parentThreadId")
            .and_then(Value::as_str)
            == Some(root_thread_id)
    {
        return thread_id(event);
    }
    let item = event.pointer("/params/item")?;
    (matches!(
        event["method"].as_str(),
        Some("item/started" | "item/completed")
    ) && thread_id(event) == Some(root_thread_id)
        && item["type"] == "subAgentActivity"
        && item["kind"] == "started")
        .then(|| item["agentThreadId"].as_str())
        .flatten()
}

fn bind_child_thread(evidence: &mut ProbeEvidence, child_thread_id: &str) {
    if let Some(existing) = evidence.child_thread_id.as_deref() {
        assert_eq!(
            existing, child_thread_id,
            "official child identity sources must converge"
        );
    } else {
        evidence.child_thread_id = Some(child_thread_id.to_string());
    }
}

async fn collect_calls(
    events: &mut broadcast::Receiver<ProfileHostEvent>,
    root_thread_id: &str,
) -> ProbeEvidence {
    let mut observed = Vec::new();
    let result = timeout(Duration::from_secs(45), async {
        let mut evidence = ProbeEvidence::default();
        let mut child_mcp_results = BTreeMap::<String, (String, Value)>::new();
        loop {
            let event = events.recv().await.expect("Profile Host event").message;
            if observed.len() < 80 {
                observed.push(format!(
                    "method={} thread={:?} item={} server={} tool={}",
                    event["method"].as_str().unwrap_or("<none>"),
                    thread_id(&event),
                    event
                        .pointer("/params/item/type")
                        .and_then(Value::as_str)
                        .unwrap_or("<none>"),
                    event
                        .pointer("/params/item/server")
                        .and_then(Value::as_str)
                        .unwrap_or("<none>"),
                    event
                        .pointer("/params/item/tool")
                        .and_then(Value::as_str)
                        .unwrap_or("<none>"),
                ));
            }
            if let Some(child_thread_id) = started_child_thread_id(&event, root_thread_id) {
                bind_child_thread(&mut evidence, child_thread_id);
                if let Some((turn_id, result)) = child_mcp_results.remove(child_thread_id) {
                    evidence.child_turn_id = Some(turn_id);
                    evidence.child_result = Some(result);
                }
            }
            if event["method"] != "item/completed" {
                continue;
            }
            let item = &event["params"]["item"];
            if item["type"] != "mcpToolCall"
                || item["server"] != DATA_SERVER
                || item["tool"] != DISCOVER_SOURCES
            {
                continue;
            }
            if thread_id(&event) == Some(root_thread_id) {
                evidence.root_result = Some(item["result"].clone());
            } else if let (Some(item_thread_id), Some(turn_id)) =
                (thread_id(&event), event["params"]["turnId"].as_str())
            {
                if Some(item_thread_id) == evidence.child_thread_id.as_deref() {
                    evidence.child_turn_id = Some(turn_id.to_string());
                    evidence.child_result = Some(item["result"].clone());
                } else {
                    child_mcp_results.insert(
                        item_thread_id.to_string(),
                        (turn_id.to_string(), item["result"].clone()),
                    );
                }
            }
            if evidence.root_result.is_some()
                && evidence.child_result.is_some()
                && evidence.child_thread_id.is_some()
            {
                return evidence;
            }
        }
    })
    .await;
    result.unwrap_or_else(|_| {
        panic!(
            "Root and native child did not both call the Workspace Probe MCP; observed:\n{}",
            observed.join("\n")
        )
    })
}

#[test]
fn child_identity_uses_only_official_thread_or_subagent_activity_fields() {
    let root = "root-thread";
    let child = "child-thread";
    for event in [
        json!({
            "method": "thread/started",
            "params": {"thread": {"id": child, "parentThreadId": root}}
        }),
        json!({
            "method": "item/started",
            "params": {
                "threadId": root,
                "item": {
                    "type": "subAgentActivity",
                    "kind": "started",
                    "agentThreadId": child
                }
            }
        }),
        json!({
            "method": "item/completed",
            "params": {
                "threadId": root,
                "item": {
                    "type": "subAgentActivity",
                    "kind": "started",
                    "agentThreadId": child
                }
            }
        }),
    ] {
        assert_eq!(started_child_thread_id(&event, root), Some(child));
    }
    assert_eq!(
        started_child_thread_id(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": root,
                    "item": {
                        "type": "subAgentActivity",
                        "kind": "interacted",
                        "agentThreadId": child,
                        "description": "started child-thread"
                    }
                }
            }),
            root,
        ),
        None,
        "display text and non-started activity must not bind child identity"
    );
}

fn assert_sources(result: &Value, expected: &str, other: &str) {
    let result = result.to_string();
    assert!(
        result.contains(expected),
        "expected Workspace source: {result}"
    );
    assert!(
        !result.contains(other),
        "cross-Workspace source leak: {result}"
    );
    assert!(
        !result.contains("demand-cities.csv"),
        "Plugin cwd became a business Workspace: {result}"
    );
}

async fn run_probe(
    adapter: &RealCodexAdapter,
    host: &ProfileHost,
    workspace: &AuthorizedWorkspace,
    country: &str,
    expected: &str,
    other: &str,
    control: &ProbeControl,
) {
    let mut events = host.subscribe();
    let root = adapter
        .start_thread(workspace, None)
        .await
        .expect("start Standard Root Workspace Probe");
    let turn = adapter
        .send_user_message(
            workspace,
            &root.thread_id,
            &format!("workspace-probe-root-{country}"),
            &TurnOptions {
                access_mode: Some("read-only".to_string()),
                ..TurnOptions::default()
            },
        )
        .await
        .expect("start Root Workspace Probe Turn");
    let root_turn_id = turn["turnId"].as_str().expect("Root Turn id");
    let evidence = collect_calls(&mut events, &root.thread_id).await;
    assert_sources(evidence.root_result.as_ref().unwrap(), expected, other);
    assert_sources(evidence.child_result.as_ref().unwrap(), expected, other);
    timeout(
        Duration::from_secs(10),
        adapter.steer_turn(
            workspace,
            None,
            &root.thread_id,
            root_turn_id,
            &format!("workspace-probe-root-steer-{country}"),
            &[],
        ),
    )
    .await
    .expect("Root remains steerable while native child exists")
    .expect("Root accepts steer while native child exists");
    control.release_root_completion.add_permits(1);

    let child_thread_id = evidence.child_thread_id.unwrap();
    let child_turn_id = evidence.child_turn_id.unwrap();
    timeout(Duration::from_secs(30), async {
        let mut root_done = false;
        let mut child_done = false;
        loop {
            let event = events.recv().await.expect("Profile Host event").message;
            if event["method"] == "turn/completed" {
                let turn_id = event["params"]["turn"]["id"].as_str();
                root_done |= thread_id(&event) == Some(root.thread_id.as_str())
                    && turn_id == Some(root_turn_id);
                child_done |= thread_id(&event) == Some(child_thread_id.as_str())
                    && turn_id == Some(child_turn_id.as_str());
            }
            if root_done && child_done {
                return;
            }
        }
    })
    .await
    .expect("Root and child Workspace Probe Turns complete");
}

/// Run explicitly with:
///
/// `CODEX_BIN=/absolute/path/to/codex cargo test -p open-web-codex-adapter --test native_workspace_probe native_root_and_child_fastmcp_use_authorized_workspace -- --ignored --exact`
#[tokio::test]
#[ignore = "requires the checked-out Codex CLI and Python FastMCP dependencies"]
async fn native_root_and_child_fastmcp_use_authorized_workspace() {
    let codex_bin = PathBuf::from(std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set"));
    let test_root = probe_tempdir();
    let runner_root = test_root.path().join("runner");
    let indonesia_root = runner_root.join("indonesia");
    let thailand_root = runner_root.join("thailand");
    std::fs::create_dir_all(&indonesia_root).unwrap();
    std::fs::create_dir_all(&thailand_root).unwrap();
    std::fs::write(
        indonesia_root.join("indonesia-root-child.csv"),
        "city_id,demand_quantity\nJKT,10\n",
    )
    .unwrap();
    std::fs::write(
        thailand_root.join("thailand-root-child.csv"),
        "city_id,demand_quantity\nBKK,20\n",
    )
    .unwrap();

    let (model_uri, control, model_server) = start_mock_responses_server().await;
    let profile_home = test_root.path().join("profile");
    std::fs::create_dir_all(&profile_home).unwrap();
    let mcp_config = probe_mcp_config(&profile_home);
    std::fs::write(
        profile_home.join("config.toml"),
        format!(
            r#"model = "mock-model"
model_provider = "mock_provider"
sandbox_mode = "read-only"

[features]
multi_agent_v2 = true

[skills]
include_instructions = true

[model_providers.mock_provider]
name = "Native Workspace Probe provider"
base_url = "{model_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
models = [{{ model_id = "mock-model", context_window = 25600 }}]

{mcp_config}
"#
        ),
    )
    .unwrap();
    let startup_files = [
        ProfileStartupFile::skill(
            "workspace-probe-root",
            b"---\nname: workspace-probe-root\ndescription: Root Workspace Probe\n---\nCall Workspace discovery, then spawn workspace_probe_child.\n".to_vec(),
        )
        .expect("valid Root Skill seed"),
        ProfileStartupFile::skill(
            "workspace-probe-child",
            b"---\nname: workspace-probe-child\ndescription: Child Workspace Probe\n---\nCall only discover_workspace_sources in the authorized Workspace.\n".to_vec(),
        )
        .expect("valid child Skill seed"),
        probe_child_role(&mcp_config),
    ];
    let host = ProfileHost::spawn(
        ProfileHostConfig::new("native-workspace-probe", &profile_home, &runner_root)
            .with_codex_bin(&codex_bin)
            .with_startup_files(startup_files),
    )
    .await
    .expect("spawn real Profile Host");
    let adapter =
        RealCodexAdapter::from_host(host.clone(), "workspace-probe-runner", runner_root.clone())
            .expect("construct real adapter");

    let indonesia = AuthorizedWorkspace {
        id: "indonesia".to_string(),
        root: indonesia_root.canonicalize().unwrap(),
    };
    let thailand = AuthorizedWorkspace {
        id: "thailand".to_string(),
        root: thailand_root.canonicalize().unwrap(),
    };
    run_probe(
        &adapter,
        &host,
        &indonesia,
        "indonesia",
        "indonesia-root-child.csv",
        "thailand-root-child.csv",
        &control,
    )
    .await;
    run_probe(
        &adapter,
        &host,
        &thailand,
        "thailand",
        "thailand-root-child.csv",
        "indonesia-root-child.csv",
        &control,
    )
    .await;

    for workspace in [&indonesia, &thailand] {
        assert!(
            !workspace.root.join(".codex").exists(),
            "native Profile Role/Skill/MCP activation must not write capability files into {}",
            workspace.root.display(),
        );
    }

    host.shutdown().await.expect("shutdown Profile Host");
    model_server.abort();
}
