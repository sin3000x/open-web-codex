use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use open_web_codex_adapter::real::RealCodexAdapter;
use open_web_codex_adapter::{AuthorizedWorkspace, CodexAdapter, ProfileQuery, TurnOptions};
use open_web_codex_approval_service::{ApprovalActor, ApprovalService};
use open_web_codex_platform_contracts::{
    McpFormRequestSource, McpFormResponseAction, RespondMcpFormRequest,
};
use open_web_codex_profile_host::{
    ProfileHost, ProfileHostConfig, ProfileHostError, ProfileHostEvent,
};
use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, CreateElicitationRequestParams,
    ElicitationAction, ElicitationSchema, EnumSchema, InitializeRequestParams, InitializeResult,
    JsonObject, ListToolsResult, NumberSchema, ProtocolVersion, ServerCapabilities, ServerInfo,
    Tool, ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::{StreamableHttpServerConfig, StreamableHttpService};
use serde_json::{json, Value};
use sqlx::postgres::PgPoolOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, mpsc, Mutex, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout, Duration};
use toml_edit::{value, Array, DocumentMut, Item, Table};
use uuid::Uuid;

use crate::copilot_package::CopilotPackageAssets;

const WAREHOUSE_SKILLS: [&str; 4] = [
    "warehouse-data",
    "warehouse-network-planning",
    "warehouse-map-delivery",
    "warehouse-supervisor",
];
const WAREHOUSE_MCP_SERVERS: [&str; 3] = ["map_utils", "supply_chain", "supply_chain_data"];
const DATA_TOOLS: [&str; 4] = [
    "discover_workspace_sources",
    "inspect_workspace_sources",
    "prepare_network_input",
    "prepare_network_geography",
];
const NETWORK_TOOLS: [&str; 12] = [
    "assess_facility_change",
    "compare_network_scenarios",
    "evaluate_network_baseline",
    "plan_cost_matrix",
    "create_navigation_matrix_request",
    "import_navigation_matrix",
    "prepare_route_matrix",
    "prepare_network_comparison_map",
    "prepare_network_coverage_map",
    "prepare_network_distribution_map",
    "publish_network_planning_report",
    "solve_p_median",
];
const MAP_TOOLS: [&str; 9] = [
    "batch_geocode",
    "batch_reverse_geocode",
    "create_map_card",
    "create_network_map_card",
    "distance_matrix",
    "execute_navigation_matrix",
    "get_route",
    "revise_map_card",
    "publish_workspace_geojson",
];
const ROOT_PROMPT: &str = "runtime-gate-root-spawn-data-and-network";
const DATA_CHILD_PROMPT: &str = "runtime-gate-data-child";
const NETWORK_CHILD_PROMPT: &str = "runtime-gate-network-child";
const DATA_SPAWN_CALL: &str = "runtime-gate-data-spawn";
const NETWORK_SPAWN_CALL: &str = "runtime-gate-network-spawn";
const INITIAL_WAIT_CALL: &str = "runtime-gate-initial-wait";
const DATA_FOLLOWUP_CALL: &str = "runtime-gate-data-followup-call";
const FOLLOWUP_WAIT_CALL: &str = "runtime-gate-followup-wait";
const DATA_FOLLOWUP_PROMPT: &str = "runtime-gate-data-followup-after-reload";
const HOT_ROOT_PROMPT: &str = "runtime-gate-root-spawn-hot-data";
const HOT_DATA_PROMPT: &str = "runtime-gate-hot-data-child";
const HOT_DATA_SPAWN_CALL: &str = "runtime-gate-hot-data-spawn";
const SKILL_HOT_MARKER: &str = "RUNTIME_GATE_SKILL_REFRESHED";
const ROLE_HOT_MARKER: &str = "RUNTIME_GATE_ROLE_REFRESHED";
const MALFORMED_ROOT_PROMPT: &str = "runtime-gate-root-spawn-malformed-role";
const MALFORMED_SPAWN_CALL: &str = "runtime-gate-malformed-role-spawn";
const FORM_ROOT_PROMPT: &str = "runtime-gate-root-spawn-form-child";
const FORM_CHILD_PROMPT: &str = "runtime-gate-form-data-child";
const FORM_SPAWN_CALL: &str = "runtime-gate-form-data-spawn";
const FORM_ROOT_STEER: &str = "runtime-gate-root-steer-during-child-form";
const FORM_ACCEPT_CALL: &str = "runtime-gate-form-accept-call";
const FORM_DECLINE_CALL: &str = "runtime-gate-form-decline-call";
const FORM_CANCEL_CALL: &str = "runtime-gate-form-cancel-call";
const FORM_RESTART_CALL: &str = "runtime-gate-form-restart-call";
const FORM_SERVER: &str = "typed_form_gate";
const FORM_TOOL: &str = "collect_route_options";
const FORM_RUNTIME_KEY: &str = "package-network-form-runtime-gate";

#[derive(Clone)]
struct RuntimeModelControl {
    requests: Arc<Mutex<Vec<Value>>>,
    release_data_child: Arc<Semaphore>,
    release_data_followup: Arc<Semaphore>,
    release_network_child: Arc<Semaphore>,
    release_form_child: Arc<Semaphore>,
    release_form_root: Arc<Semaphore>,
}

/// This fixture deliberately uses the same RMCP 1.8 streamable-HTTP server
/// shape as the upstream app-server elicitation round-trip. It is a normal
/// Role-local MCP configuration, not a Codex app/connector transport.
#[derive(Clone, Default)]
struct FormGateMcpServer {
    initialize_requests: Arc<Mutex<Vec<Value>>>,
    elicitation_requests: Arc<Mutex<Vec<Value>>>,
}

impl ServerHandler for FormGateMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_protocol_version(ProtocolVersion::V_2025_06_18)
    }

    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<InitializeResult, rmcp::ErrorData>> + Send + '_
    {
        let initialize_requests = Arc::clone(&self.initialize_requests);
        let server_info = self.get_info();
        async move {
            let snapshot = serde_json::to_value(&request)
                .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
            initialize_requests.lock().await.push(snapshot);
            context.peer.set_peer_info(request);
            Ok(server_info)
        }
    }

    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, rmcp::ErrorData> {
        let input_schema: JsonObject = serde_json::from_value(json!({
            "type": "object",
            "properties": {
                "scenario": {
                    "type": "string",
                    "enum": ["accept", "decline", "cancel", "restart"]
                }
            },
            "required": ["scenario"],
            "additionalProperties": false
        }))
        .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
        let mut tool = Tool::new(
            Cow::Borrowed(FORM_TOOL),
            Cow::Borrowed("Request route options from the user."),
            Arc::new(input_schema),
        );
        tool.annotations = Some(ToolAnnotations::new().read_only(true));
        Ok(ListToolsResult {
            tools: vec![tool],
            next_cursor: None,
            meta: None,
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        if request.name.as_ref() != FORM_TOOL {
            return Err(rmcp::ErrorData::invalid_params(
                format!("expected {FORM_TOOL}"),
                None,
            ));
        }
        let scenario = request
            .arguments
            .as_ref()
            .and_then(|arguments| arguments.get("scenario"))
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "accept" | "decline" | "cancel" | "restart"))
            .ok_or_else(|| rmcp::ErrorData::invalid_params("scenario is required", None))?;
        let method = EnumSchema::builder(vec!["curve".to_string(), "navigation".to_string()])
            .with_default("curve")
            .expect("fixture default is a declared enum value")
            .enum_titles(vec![
                "Curved distance".to_string(),
                "Navigation".to_string(),
            ])
            .expect("fixture enum titles match declared values")
            .build();
        let requested_schema = ElicitationSchema::builder()
            .required_enum_schema("method", method)
            .required_number_property("factor", |field: NumberSchema| {
                field
                    .title("Detour factor")
                    .range(1.0, 2.0)
                    .with_default(1.2)
            })
            .build()
            .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
        let elicitation_request = CreateElicitationRequestParams::FormElicitationParams {
            meta: None,
            message: format!("Route options for {scenario}"),
            requested_schema,
        };
        // Record the typed request constructed by this fixture. This is not a
        // raw transport capture; real wire-level diagnostics must observe the
        // Streamable HTTP boundary separately.
        self.elicitation_requests.lock().await.push(json!({
            "jsonrpc": "2.0",
            "method": "elicitation/create",
            "params": serde_json::to_value(&elicitation_request)
                .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?,
        }));
        let response = context
            .peer
            .create_elicitation(elicitation_request)
            .await
            .map_err(|error| rmcp::ErrorData::internal_error(error.to_string(), None))?;
        let output = match response.action {
            ElicitationAction::Accept => format!(
                "accepted: {}",
                response.content.unwrap_or_else(|| json!({}))
            ),
            ElicitationAction::Decline => "declined".to_string(),
            ElicitationAction::Cancel => "cancelled".to_string(),
        };
        Ok(CallToolResult::success(vec![Content::text(output)]))
    }
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repository root")
        .to_path_buf()
}

fn prepared_network_copilot_descriptor(repository: &Path) -> PathBuf {
    repository.join(
        ".local/open-web-codex/tool-environments/warehouse-network-copilot/copilot-sdk/prepared-tools.v1.json",
    )
}

fn prepared_network_copilot_build_store(repository: &Path) -> PathBuf {
    repository.join(".local/open-web-codex/tool-builds")
}

fn warehouse_copilot_package(repository: &Path) -> PathBuf {
    repository.join("copilots/warehouse-network")
}

fn runtime_probe_tempdir() -> tempfile::TempDir {
    let root = repository_root().join("apps/web/target/package-network-runtime-tests");
    std::fs::create_dir_all(&root).expect("create Runtime probe root");
    tempfile::Builder::new()
        .prefix("clean-profile-")
        .tempdir_in(root)
        .expect("create Runtime probe directory")
}

fn required_path(path: &Path, label: &str) -> PathBuf {
    path.canonicalize()
        .unwrap_or_else(|error| panic!("{label} {} is unavailable: {error}", path.display()))
}

fn response_skill_names(response: &Value, scope: &str) -> BTreeSet<String> {
    response["data"]
        .as_array()
        .expect("skills/list data")
        .iter()
        .flat_map(|entry| {
            entry["skills"]
                .as_array()
                .expect("skills/list entry skills")
        })
        .filter(|skill| skill["scope"].as_str() == Some(scope))
        .map(|skill| {
            skill["name"]
                .as_str()
                .expect("Runtime Skill name")
                .to_string()
        })
        .collect()
}

fn response_skills(response: &Value) -> impl Iterator<Item = &Value> {
    response["data"]
        .as_array()
        .expect("skills/list data")
        .iter()
        .flat_map(|entry| {
            entry["skills"]
                .as_array()
                .expect("skills/list entry skills")
        })
}

fn response_mcp_server_names(response: &Value) -> BTreeSet<String> {
    response["data"]
        .as_array()
        .expect("mcpServerStatus/list data")
        .iter()
        .map(|server| {
            server["name"]
                .as_str()
                .expect("Runtime MCP Server name")
                .to_string()
        })
        .collect()
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
            event["type"].as_str().expect("SSE event type")
        )
    })
    .collect()
}

fn assistant_message(response_id: &str, text: &str) -> String {
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

fn spawn_agent_call(
    response_id: &str,
    call_id: &str,
    prompt: &str,
    task_name: &str,
    agent_type: &str,
) -> String {
    sse(
        response_id,
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": call_id,
                "namespace": "collaboration",
                "name": "spawn_agent",
                "arguments": json!({
                    "message": prompt,
                    "task_name": task_name,
                    "agent_type": agent_type,
                    "fork_turns": "none",
                }).to_string(),
            },
        }),
    )
}

fn collaboration_call(response_id: &str, call_id: &str, name: &str, arguments: Value) -> String {
    sse(
        response_id,
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": call_id,
                "namespace": "collaboration",
                "name": name,
                "arguments": arguments.to_string(),
            },
        }),
    )
}

fn mcp_call(
    response_id: &str,
    call_id: &str,
    server_name: &str,
    tool_name: &str,
    arguments: Value,
) -> String {
    sse(
        response_id,
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": call_id,
                "namespace": format!("mcp__{server_name}"),
                "name": tool_name,
                "arguments": arguments.to_string(),
            },
        }),
    )
}

async fn read_responses_request(stream: &mut tokio::net::TcpStream) -> Value {
    let mut request = Vec::new();
    let mut chunk = [0_u8; 8 * 1024];
    loop {
        let read = stream.read(&mut chunk).await.expect("read model request");
        assert_ne!(read, 0, "model request ended before its body");
        request.extend_from_slice(&chunk[..read]);
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            })
            .expect("Responses request content length");
        let body_start = header_end + 4;
        if request.len() >= body_start + content_length {
            return serde_json::from_slice(&request[body_start..body_start + content_length])
                .expect("Responses request JSON");
        }
    }
}

async fn model_response_for(request: &Value, control: &RuntimeModelControl) -> String {
    control.requests.lock().await.push(request.clone());
    let request_text = request.to_string();
    if request_text.contains(FORM_ROOT_STEER) {
        return assistant_message(
            "runtime-gate-form-root-steered",
            "Root accepted the steer while its child was waiting for form input.",
        );
    }
    if request_text.contains(FORM_RESTART_CALL) {
        return assistant_message(
            "runtime-gate-form-child-unexpected-restart-result",
            "The restart request unexpectedly resolved before restart.",
        );
    }
    if request_text.contains(FORM_CANCEL_CALL) {
        return mcp_call(
            "runtime-gate-form-restart",
            FORM_RESTART_CALL,
            FORM_SERVER,
            FORM_TOOL,
            json!({ "scenario": "restart" }),
        );
    }
    if request_text.contains(FORM_DECLINE_CALL) {
        return mcp_call(
            "runtime-gate-form-cancel",
            FORM_CANCEL_CALL,
            FORM_SERVER,
            FORM_TOOL,
            json!({ "scenario": "cancel" }),
        );
    }
    if request_text.contains(FORM_ACCEPT_CALL) {
        return mcp_call(
            "runtime-gate-form-decline",
            FORM_DECLINE_CALL,
            FORM_SERVER,
            FORM_TOOL,
            json!({ "scenario": "decline" }),
        );
    }
    if request_text.contains(FORM_SPAWN_CALL) {
        control
            .release_form_root
            .clone()
            .acquire_owned()
            .await
            .expect("form Root release channel")
            .forget();
        return assistant_message(
            "runtime-gate-form-root-done",
            "Root remained active while the child requested form input.",
        );
    }
    if request_text.contains(FORM_CHILD_PROMPT) {
        control
            .release_form_child
            .clone()
            .acquire_owned()
            .await
            .expect("form child release channel")
            .forget();
        return mcp_call(
            "runtime-gate-form-accept",
            FORM_ACCEPT_CALL,
            FORM_SERVER,
            FORM_TOOL,
            json!({ "scenario": "accept" }),
        );
    }
    if request_text.contains(FORM_ROOT_PROMPT) {
        return spawn_agent_call(
            "runtime-gate-form-root-spawn",
            FORM_SPAWN_CALL,
            FORM_CHILD_PROMPT,
            "runtime_gate_form_data",
            "data_agent",
        );
    }
    if request_text.contains(HOT_DATA_SPAWN_CALL) {
        return assistant_message("runtime-gate-hot-root-done", "Hot Role Root completed.");
    }
    if request_text.contains(HOT_DATA_PROMPT) {
        return assistant_message("runtime-gate-hot-data-done", "Hot Data child completed.");
    }
    if request_text.contains(HOT_ROOT_PROMPT) {
        return spawn_agent_call(
            "runtime-gate-root-spawn-hot-data",
            HOT_DATA_SPAWN_CALL,
            HOT_DATA_PROMPT,
            "runtime_gate_hot_data",
            "data_agent",
        );
    }
    if request_text.contains(MALFORMED_SPAWN_CALL) {
        return assistant_message(
            "runtime-gate-malformed-root-done",
            "Root observed the unavailable Role and completed.",
        );
    }
    if request_text.contains(MALFORMED_ROOT_PROMPT) {
        return spawn_agent_call(
            "runtime-gate-root-spawn-malformed-role",
            MALFORMED_SPAWN_CALL,
            "this child must never start",
            "runtime_gate_malformed_role",
            "broken_agent",
        );
    }
    if request_text.contains(FOLLOWUP_WAIT_CALL) {
        return assistant_message("runtime-gate-root-done", "Root completed after mailbox.");
    }
    if request_text.contains(DATA_FOLLOWUP_CALL) {
        return collaboration_call(
            "runtime-gate-root-followup-wait",
            FOLLOWUP_WAIT_CALL,
            "wait_agent",
            json!({ "timeout_ms": 30000 }),
        );
    }
    if request_text.contains(INITIAL_WAIT_CALL) {
        return collaboration_call(
            "runtime-gate-root-data-followup",
            DATA_FOLLOWUP_CALL,
            "followup_task",
            json!({
                "target": "runtime_gate_data",
                "message": DATA_FOLLOWUP_PROMPT,
            }),
        );
    }
    if request_text.contains(NETWORK_SPAWN_CALL) {
        return collaboration_call(
            "runtime-gate-root-initial-wait",
            INITIAL_WAIT_CALL,
            "wait_agent",
            json!({ "timeout_ms": 30000 }),
        );
    }
    if request_text.contains(DATA_SPAWN_CALL) {
        return spawn_agent_call(
            "runtime-gate-root-spawn-network",
            NETWORK_SPAWN_CALL,
            NETWORK_CHILD_PROMPT,
            "runtime_gate_network",
            "network_agent",
        );
    }
    if request_text.contains(DATA_FOLLOWUP_PROMPT) {
        control
            .release_data_followup
            .clone()
            .acquire_owned()
            .await
            .expect("data follow-up release channel")
            .forget();
        return assistant_message(
            "runtime-gate-data-followup-done",
            "Data follow-up completed.",
        );
    }
    if request_text.contains(DATA_CHILD_PROMPT) {
        control
            .release_data_child
            .clone()
            .acquire_owned()
            .await
            .expect("data release channel")
            .forget();
        return assistant_message("runtime-gate-data-done", "Data child completed.");
    }
    if request_text.contains(NETWORK_CHILD_PROMPT) {
        control
            .release_network_child
            .clone()
            .acquire_owned()
            .await
            .expect("network release channel")
            .forget();
        return assistant_message("runtime-gate-network-done", "Network child completed.");
    }
    if request_text.contains(ROOT_PROMPT) {
        return spawn_agent_call(
            "runtime-gate-root-spawn-data",
            DATA_SPAWN_CALL,
            DATA_CHILD_PROMPT,
            "runtime_gate_data",
            "data_agent",
        );
    }
    assistant_message(
        "runtime-gate-unexpected",
        "Unexpected Runtime gate request.",
    )
}

async fn start_runtime_model_server() -> (String, RuntimeModelControl, tokio::task::JoinHandle<()>)
{
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind Runtime model server");
    let address = listener.local_addr().expect("Runtime model address");
    let control = RuntimeModelControl {
        requests: Arc::new(Mutex::new(Vec::new())),
        release_data_child: Arc::new(Semaphore::new(0)),
        release_data_followup: Arc::new(Semaphore::new(0)),
        release_network_child: Arc::new(Semaphore::new(0)),
        release_form_child: Arc::new(Semaphore::new(0)),
        release_form_root: Arc::new(Semaphore::new(0)),
    };
    let server_control = control.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let control = server_control.clone();
            tokio::spawn(async move {
                let request = read_responses_request(&mut stream).await;
                let body = model_response_for(&request, &control).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                    body.len(),
                    body,
                );
                stream
                    .write_all(response.as_bytes())
                    .await
                    .expect("write Runtime model response");
                stream.shutdown().await.expect("close model response");
            });
        }
    });
    (format!("http://{address}"), control, server)
}

fn event_thread_id(event: &Value) -> Option<&str> {
    event
        .pointer("/params/threadId")
        .or_else(|| event.pointer("/params/thread/id"))
        .and_then(Value::as_str)
}

async fn list_direct_child_thread_ids(
    host: &ProfileHost,
    parent_thread_id: &str,
    agent_role: &str,
    excluded_thread_ids: &[&str],
) -> Vec<String> {
    let response = host
        .request(
            "thread/list",
            json!({
                "parentThreadId": parent_thread_id,
                "sourceKinds": ["subAgentThreadSpawn"],
                "archived": false,
                "useStateDbOnly": true,
                "limit": 100,
            }),
        )
        .await
        .expect("official direct-child thread/list");
    assert!(
        response
            .get("nextCursor")
            .is_none_or(|cursor| cursor.is_null()),
        "official direct-child thread/list must fit in one page: {response}",
    );
    response["data"]
        .as_array()
        .expect("official direct-child thread/list data")
        .iter()
        .filter_map(|thread| {
            let id = thread.get("id").and_then(Value::as_str)?;
            let parent = thread.get("parentThreadId").and_then(Value::as_str)?;
            let role = thread.get("agentRole").and_then(Value::as_str)?;
            (parent == parent_thread_id && role == agent_role && !excluded_thread_ids.contains(&id))
                .then(|| id.to_string())
        })
        .collect()
}

async fn wait_for_direct_child_thread(
    host: &ProfileHost,
    parent_thread_id: &str,
    agent_role: &str,
    excluded_thread_ids: &[&str],
) -> String {
    timeout(Duration::from_secs(30), async {
        loop {
            let matching_ids = list_direct_child_thread_ids(
                host,
                parent_thread_id,
                agent_role,
                excluded_thread_ids,
            )
            .await;
            match matching_ids.as_slice() {
                [thread_id] => return thread_id.clone(),
                [] => sleep(Duration::from_millis(100)).await,
                _ => panic!(
                    "official direct-child thread/list found multiple `{agent_role}` children for parent `{parent_thread_id}`: {matching_ids:?}"
                ),
            }
        }
    })
    .await
    .expect("official direct-child thread/list found no matching child within 30s")
}

async fn wait_for_model_request(
    control: &RuntimeModelControl,
    marker: &str,
    excluded_marker: &str,
) -> Value {
    timeout(Duration::from_secs(30), async {
        loop {
            if let Some(request) = control
                .requests
                .lock()
                .await
                .iter()
                .find(|request| {
                    let request = request.to_string();
                    request.contains(marker) && !request.contains(excluded_marker)
                })
                .cloned()
            {
                return request;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("model request marker")
}

async fn wait_for_function_output(control: &RuntimeModelControl, call_id: &str) -> Value {
    timeout(Duration::from_secs(30), async {
        loop {
            if let Some(output) = control
                .requests
                .lock()
                .await
                .iter()
                .find_map(|request| request_function_output(request, call_id).cloned())
            {
                return output;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("collaboration function output")
}

fn assert_role_mcp_inventory(response: &Value, expected: &[(&str, &[&str])]) {
    let expected_names = expected
        .iter()
        .map(|(name, _)| (*name).to_string())
        .collect::<BTreeSet<_>>();
    let warehouse_names = response_mcp_server_names(response)
        .intersection(&BTreeSet::from(WAREHOUSE_MCP_SERVERS.map(str::to_string)))
        .cloned()
        .collect::<BTreeSet<_>>();
    assert_eq!(warehouse_names, expected_names);
    for (server_name, tools) in expected {
        let server = response["data"]
            .as_array()
            .expect("MCP status data")
            .iter()
            .find(|server| server["name"].as_str() == Some(server_name))
            .unwrap_or_else(|| panic!("Runtime omitted MCP Server {server_name}"));
        let actual = server["tools"]
            .as_object()
            .expect("MCP tool inventory")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(
            actual,
            tools.iter().map(|tool| (*tool).to_string()).collect(),
            "unexpected tool inventory for {server_name}",
        );
    }
}

fn assert_child_skill_policy(
    request: &Value,
    enabled_skills: &[(&str, &str)],
    catalog_skills: &[&str],
) {
    let developer_text = request_developer_text(request);
    assert!(
        developer_text.contains("You are the native child Role"),
        "native child request omitted Role developer instructions",
    );
    for (skill_name, enabled_marker) in enabled_skills {
        let entry_prefix = format!("- {skill_name}: ");
        let entry = developer_text
            .lines()
            .find(|line| line.starts_with(&entry_prefix))
            .unwrap_or_else(|| panic!("Runtime omitted enabled Role Skill `{skill_name}`"));
        assert!(
            entry.contains(enabled_marker)
                && entry.contains("(file: ")
                && entry.contains(&format!("{skill_name}/SKILL.md)")),
            "enabled Role Skill catalog entry is incomplete: {entry}",
        );
    }
    assert!(
        catalog_skills
            .iter()
            .all(|skill| developer_text.contains(&format!("- {skill}: "))),
        "native task Skill catalog must remain available to every Copilot Thread",
    );
}

fn request_developer_text(request: &Value) -> String {
    request["input"]
        .as_array()
        .expect("Responses input")
        .iter()
        .filter(|item| item["role"].as_str() == Some("developer"))
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|content| content["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn request_user_text(request: &Value) -> String {
    request["input"]
        .as_array()
        .expect("Responses input")
        .iter()
        .filter(|item| item["role"].as_str() == Some("user"))
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter_map(|content| content["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn request_tool_names(request: &Value) -> BTreeSet<String> {
    fn collect(value: &Value, names: &mut BTreeSet<String>) {
        if let Some(name) = value.get("name").and_then(Value::as_str) {
            names.insert(name.to_string());
        }
        if let Some(name) = value
            .get("function")
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        {
            names.insert(name.to_string());
        }
        if let Some(tools) = value.get("tools").and_then(Value::as_array) {
            for tool in tools {
                collect(tool, names);
            }
        }
    }

    let mut names = BTreeSet::new();
    if let Some(tools) = request.get("tools").and_then(Value::as_array) {
        for tool in tools {
            collect(tool, &mut names);
        }
    }
    names
}

fn assert_role_has_no_shell_tools(request: &Value, role: &str, require_tool_search: bool) {
    let names = request_tool_names(request);
    for forbidden in ["exec_command", "shell_command", "write_stdin"] {
        assert!(
            !names.contains(forbidden),
            "{role} must not expose {forbidden}: {names:?}"
        );
    }
    if require_tool_search {
        assert!(
            names.contains("tool_search"),
            "{role} must retain native tool_search when deferred MCP tools are available: {names:?}"
        );
    }
}

fn request_function_output<'a>(request: &'a Value, call_id: &str) -> Option<&'a Value> {
    request["input"].as_array()?.iter().find(|item| {
        item["type"].as_str() == Some("function_call_output")
            && item["call_id"].as_str() == Some(call_id)
    })
}

async fn wait_for_event_method(
    events: &mut broadcast::Receiver<ProfileHostEvent>,
    method: &str,
) -> Value {
    timeout(Duration::from_secs(30), async {
        loop {
            let event = events.recv().await.expect("Profile Host event").message;
            if event["method"].as_str() == Some(method) {
                return event;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("Runtime did not emit {method}"))
}

fn install_form_gate_server(profile_home: &Path, url: &str) {
    let role_path = profile_home.join("agents/data_agent.toml");
    let mut role = std::fs::read_to_string(&role_path)
        .expect("read Profile Data Role")
        .parse::<DocumentMut>()
        .expect("parse Profile Data Role");
    let mut server = Table::new();
    server["url"] = value(url);
    server["enabled"] = value(true);
    server["required"] = value(true);
    server["default_tools_approval_mode"] = value("approve");
    let mut tools = Array::new();
    tools.push(FORM_TOOL);
    server["enabled_tools"] = Item::Value(tools.into());
    role["mcp_servers"][FORM_SERVER] = Item::Table(server);
    std::fs::write(&role_path, role.to_string()).expect("add typed form MCP to Data Role");
}

async fn start_form_gate_server(
    initialize_requests: Arc<Mutex<Vec<Value>>>,
    elicitation_requests: Arc<Mutex<Vec<Value>>>,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind typed form MCP listener");
    let address = listener
        .local_addr()
        .expect("read typed form MCP listener address");
    let service = StreamableHttpService::new(
        move || {
            Ok(FormGateMcpServer {
                initialize_requests: Arc::clone(&initialize_requests),
                elicitation_requests: Arc::clone(&elicitation_requests),
            })
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = Router::new().nest_service("/mcp", service);
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    (format!("http://{address}/mcp"), server)
}

struct FormGateStore {
    pool: sqlx::PgPool,
    actor: ApprovalActor,
    profile_id: Uuid,
    workspace_id: Uuid,
    run_id: Uuid,
    runtime_key: String,
}

async fn seed_form_gate_store(
    database_url: &str,
    workspace_root: &Path,
    root_thread_id: &str,
) -> FormGateStore {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(database_url)
        .await
        .expect("connect disposable PostgreSQL database");
    open_web_codex_platform_store::migrate::run(&pool)
        .await
        .expect("migrate disposable PostgreSQL database");
    let user_id = Uuid::now_v7();
    let organization_id = Uuid::now_v7();
    let project_id = Uuid::now_v7();
    let profile_id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let task_id = Uuid::now_v7();
    let run_id = Uuid::now_v7();
    let suffix = user_id.simple();
    let runtime_key = format!("{FORM_RUNTIME_KEY}-{suffix}");
    sqlx::query(
        "INSERT INTO users (id, username, name, email, password_hash, role) \
         VALUES ($1, $2, 'Form Gate User', $3, 'unused', 'owner')",
    )
    .bind(user_id)
    .bind(format!("form_gate_{suffix}"))
    .bind(format!("form-gate-{suffix}@example.invalid"))
    .execute(&pool)
    .await
    .expect("insert form gate user");
    sqlx::query("INSERT INTO organizations (id, name, slug) VALUES ($1, 'Form Gate', $2)")
        .bind(organization_id)
        .bind(format!("form-gate-{suffix}"))
        .execute(&pool)
        .await
        .expect("insert form gate organization");
    sqlx::query(
        "INSERT INTO memberships (organization_id, user_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("insert form gate membership");
    sqlx::query(
        "INSERT INTO projects (id, organization_id, created_by, name, git_url) \
         VALUES ($1, $2, $3, 'Form Gate Project', 'https://example.invalid/form-gate.git')",
    )
    .bind(project_id)
    .bind(organization_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("insert form gate project");
    sqlx::query(
        "INSERT INTO profiles (id, organization_id, owner_user_id, runtime_key, name) \
         VALUES ($1, $2, $3, $4, 'Form Gate Profile')",
    )
    .bind(profile_id)
    .bind(organization_id)
    .bind(user_id)
    .bind(&runtime_key)
    .execute(&pool)
    .await
    .expect("insert form gate profile");
    sqlx::query(
        "INSERT INTO workspaces \
         (id, organization_id, project_id, profile_id, created_by, name, root_path, source_ref, state) \
         VALUES ($1, $2, $3, $4, $5, 'Form Gate Workspace', $6, 'main', 'ready')",
    )
    .bind(workspace_id)
    .bind(organization_id)
    .bind(project_id)
    .bind(profile_id)
    .bind(user_id)
    .bind(workspace_root.to_string_lossy().as_ref())
    .execute(&pool)
    .await
    .expect("insert form gate Workspace");
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
    .expect("insert form gate Workspace grant");
    sqlx::query(
        "INSERT INTO tasks \
         (id, organization_id, project_id, workspace_id, created_by, title, status) \
         VALUES ($1, $2, $3, $4, $5, 'Form Gate Task', 'running')",
    )
    .bind(task_id)
    .bind(organization_id)
    .bind(project_id)
    .bind(workspace_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("insert form gate Task");
    sqlx::query(
        "INSERT INTO runs \
         (id, organization_id, task_id, requested_by, requested_profile_id, workspace_id, \
          status, codex_thread_id) \
         VALUES ($1, $2, $3, $4, $5, $6, 'running', $7)",
    )
    .bind(run_id)
    .bind(organization_id)
    .bind(task_id)
    .bind(user_id)
    .bind(profile_id)
    .bind(workspace_id)
    .bind(root_thread_id)
    .execute(&pool)
    .await
    .expect("insert form gate Run");
    FormGateStore {
        pool,
        actor: ApprovalActor {
            user_id,
            organization_id,
        },
        profile_id,
        workspace_id,
        run_id,
        runtime_key,
    }
}

impl FormGateStore {
    async fn bind_child(&self, root_thread_id: &str, child_thread_id: &str) {
        sqlx::query(
            "INSERT INTO runtime_agent_projections \
             (organization_id, profile_id, workspace_id, root_run_id, thread_id, \
              parent_thread_id, source_kind, agent_nickname, agent_role) \
             VALUES ($1, $2, $3, $4, $5, $6, 'thread_spawn', 'Wanwan', 'data_agent')",
        )
        .bind(self.actor.organization_id)
        .bind(self.profile_id)
        .bind(self.workspace_id)
        .bind(self.run_id)
        .bind(child_thread_id)
        .bind(root_thread_id)
        .execute(&self.pool)
        .await
        .expect("bind native child Thread projection");
        sqlx::query(
            "INSERT INTO runtime_agent_execution_projections \
             (organization_id, profile_id, workspace_id, root_run_id, agent_thread_id, \
              ordinal, task, display_title, status, current_behavior, \
              first_observed_sequence, last_observed_sequence) \
             VALUES ($1, $2, $3, $4, $5, 1, 'Collect route options', \
                     'Agent · use this ResourceRef for the full assigned task', \
                     'waiting', 'Waiting for user form input', 1, 1)",
        )
        .bind(self.actor.organization_id)
        .bind(self.profile_id)
        .bind(self.workspace_id)
        .bind(self.run_id)
        .bind(child_thread_id)
        .execute(&self.pool)
        .await
        .expect("bind native child execution projection");
    }
}

async fn capture_next_form(
    events: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    approvals: &ApprovalService,
) -> Uuid {
    timeout(Duration::from_secs(30), async {
        loop {
            let frame = events.recv().await.expect("adapter event frame");
            if let Some(approval_id) = approvals
                .capture_event_frame(&frame)
                .await
                .expect("persist Runtime request before projection")
            {
                return approval_id;
            }
        }
    })
    .await
    .expect("native child MCP form request")
}

async fn capture_form_before_tool_completion(
    events: &mut mpsc::UnboundedReceiver<Vec<u8>>,
    approvals: &ApprovalService,
    control: &RuntimeModelControl,
    call_id: &str,
    mcp_initialize_requests: &Arc<Mutex<Vec<Value>>>,
    mcp_elicitation_requests: &Arc<Mutex<Vec<Value>>>,
) -> Uuid {
    tokio::select! {
        approval_id = capture_next_form(events, approvals) => approval_id,
        request = wait_for_model_request(control, call_id, "no-excluded-marker") => {
            let mcp_initialize_requests = mcp_initialize_requests.lock().await.clone();
            let mcp_elicitation_requests = mcp_elicitation_requests.lock().await.clone();
            panic!(
                "MCP tool completed before emitting a durable form request: {:?}; \
                 fixture-observed MCP initialize requests: {:?}; fixture-constructed elicitation/create request: {:?}",
                request_function_output(&request, call_id),
                mcp_initialize_requests,
                mcp_elicitation_requests,
            )
        }
    }
}

async fn respond_to_form(
    approvals: &ApprovalService,
    adapter: &RealCodexAdapter,
    actor: ApprovalActor,
    runtime_instance_id: Uuid,
    approval_id: Uuid,
    action: McpFormResponseAction,
    content: Option<BTreeMap<String, Value>>,
) {
    let dispatch = approvals
        .begin_mcp_form_response(
            actor,
            approval_id,
            runtime_instance_id,
            RespondMcpFormRequest {
                action,
                content,
                version: 0,
            },
        )
        .await
        .expect("lock and validate typed MCP form response");
    adapter
        .respond_to_server_request(
            dispatch.runtime_instance_id,
            dispatch.runtime_request_id.clone(),
            dispatch.response.clone(),
        )
        .await
        .expect("deliver response to the exact native child request");
    approvals
        .complete_decision(actor, &dispatch)
        .await
        .expect("complete durable MCP form response");
}

/// Real native discovery gate for the phase-one configured Copilot.
///
/// Run with:
///
/// `CODEX_BIN=/absolute/path/to/codex cargo test -p open-web-codex-server \
/// copilot_package_clean_profile_runtime_gate -- --ignored --exact`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a real Codex CLI and prepared warehouse MCP environments"]
async fn copilot_package_clean_profile_runtime_gate() {
    let codex_bin = required_path(
        Path::new(&std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set")),
        "Codex binary",
    );
    let repository = required_path(&repository_root(), "repository root");
    let assets = CopilotPackageAssets::resolve(
        &warehouse_copilot_package(&repository),
        &prepared_network_copilot_descriptor(&repository),
        &prepared_network_copilot_build_store(&repository),
    )
    .expect("resolve configured package assets");

    let test_root = runtime_probe_tempdir();
    let profile_home = test_root.path().join("profile");
    let runner_root = test_root.path().join("runner");
    let workspace_root = runner_root.join("workspace");
    std::fs::create_dir_all(&profile_home).expect("create Profile home");
    std::fs::create_dir_all(&workspace_root).expect("create authorized Workspace");
    let (model_uri, model_control, model_server) = start_runtime_model_server().await;
    let profile_config = format!(
        r#"model = "mock-model"
model_provider = "mock_provider"
approval_policy = "never"
sandbox_mode = "read-only"

[features]
multi_agent_v2 = true

[skills]
include_instructions = true

[model_providers.mock_provider]
name = "Built-in network Runtime gate provider"
base_url = "{model_uri}/v1"
wire_api = "responses"
supports_function_tools = true
request_max_retries = 0
stream_max_retries = 0
models = [{{ model_id = "mock-model", context_window = 25600, supports_search_tool = true }}]
"#
    );
    let config_path = profile_home.join("config.toml");
    std::fs::write(&config_path, &profile_config).expect("write mock Profile config");

    let host = ProfileHost::spawn(
        ProfileHostConfig::new("package-network-runtime-gate", &profile_home, &runner_root)
            .with_codex_bin(&codex_bin)
            .with_disabled_features(crate::copilot_package::disabled_codex_features())
            .with_enabled_features(crate::copilot_package::enabled_codex_features())
            .with_startup_files(
                assets
                    .startup_files(&profile_home)
                    .expect("render production Profile seeds"),
            ),
    )
    .await
    .expect("spawn real Profile Host");
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("read Profile config"),
        profile_config,
        "Profile startup seeds must not rewrite config.toml",
    );

    let adapter = RealCodexAdapter::from_host_with_root_executions(
        host.clone(),
        "package-network-runtime-runner",
        runner_root.clone(),
        vec![assets
            .root_execution_config(&profile_home)
            .expect("resolve managed Root execution config")],
        Some(assets.id().to_string()),
    )
    .expect("construct real Codex adapter");
    let workspace = AuthorizedWorkspace {
        id: "warehouse-workspace".to_string(),
        root: workspace_root.canonicalize().expect("canonical Workspace"),
    };

    let skills = adapter
        .query_profile(ProfileQuery::Skills {
            workspace: workspace.clone(),
            force_reload: true,
        })
        .await
        .expect("query native Skills");
    assert_eq!(
        response_skill_names(&skills, "user"),
        BTreeSet::from(WAREHOUSE_SKILLS.map(str::to_string)),
        "a clean Profile must not inherit user Skills outside Profile-owned seeds",
    );
    assert!(
        response_skill_names(&skills, "repo").is_empty(),
        "the clean Workspace must not contribute repository Skills: {skills}",
    );
    for skill_name in WAREHOUSE_SKILLS {
        let expected_path = profile_home
            .join("skills")
            .join(skill_name)
            .join("SKILL.md")
            .canonicalize()
            .expect("canonical Profile Skill path");
        let runtime_skill = response_skills(&skills)
            .find(|skill| skill["name"].as_str() == Some(skill_name))
            .unwrap_or_else(|| panic!("Runtime omitted {skill_name}"));
        assert_eq!(runtime_skill["scope"].as_str(), Some("user"));
        assert_eq!(
            runtime_skill["path"].as_str(),
            Some(expected_path.to_string_lossy().as_ref()),
            "{skill_name} must resolve from this Profile",
        );
    }
    for (skill_name, description) in [
        ("warehouse-supervisor", "仓网多 Agent Copilot 的常驻身份"),
        (
            "warehouse-data",
            "仅供仓网 Supervisor 原生创建的 data_agent 使用",
        ),
        (
            "warehouse-network-planning",
            "仅供 network_agent 使用。准备路线与成本",
        ),
        (
            "warehouse-map-delivery",
            "仅供 network_agent 使用。将精确仓网结果",
        ),
    ] {
        let runtime_skill = response_skills(&skills)
            .find(|skill| skill["name"].as_str() == Some(skill_name))
            .unwrap_or_else(|| panic!("Runtime omitted {skill_name}"));
        assert!(
            runtime_skill["description"]
                .as_str()
                .is_some_and(|actual| actual.contains(description)),
            "{skill_name} must expose its native catalog description",
        );
    }
    assert!(
        skills["data"]
            .as_array()
            .expect("skills/list data")
            .iter()
            .all(|entry| entry["errors"].as_array().is_some_and(Vec::is_empty)),
        "native Skill discovery must not report parse errors: {skills}",
    );
    assert!(
        !workspace.root.join("skills").exists()
            && !workspace.root.join("agents").exists()
            && !workspace.root.join(".agents").exists()
            && !workspace.root.join(".codex").exists(),
        "Profile startup must not write capability files into the Workspace",
    );

    let root = adapter
        .start_thread(&workspace, None)
        .await
        .expect("start Standard Root Thread");
    let root_mcp = adapter
        .query_profile(ProfileQuery::McpServers {
            cursor: None,
            limit: None,
            thread_id: Some(root.thread_id.clone()),
        })
        .await
        .expect("query Root MCP inventory");
    let root_servers = response_mcp_server_names(&root_mcp);
    assert!(
        WAREHOUSE_MCP_SERVERS
            .iter()
            .all(|server| !root_servers.contains(*server)),
        "Standard Root must not inherit Role-local warehouse MCP Servers: {root_servers:?}",
    );

    let mut terminal_events = host.subscribe();
    adapter
        .send_user_message(
            &workspace,
            &root.thread_id,
            ROOT_PROMPT,
            &TurnOptions {
                access_mode: Some("read-only".to_string()),
                ..TurnOptions::default()
            },
        )
        .await
        .expect("start native Root collaboration Turn");

    let root_request = wait_for_model_request(&model_control, ROOT_PROMPT, DATA_SPAWN_CALL).await;
    let first_request = model_control
        .requests
        .lock()
        .await
        .first()
        .cloned()
        .expect("the Root model request is recorded");
    assert_eq!(
        first_request, root_request,
        "the Root skill projection must apply to the first model request",
    );
    let root_developer_text = request_developer_text(&root_request);
    let root_user_text = request_user_text(&root_request);
    assert_role_has_no_shell_tools(&root_request, "warehouse Root", false);
    assert!(
        request_tool_names(&root_request)
            .iter()
            .any(|name| name.contains("multi_agent_v1") || name == "spawn_agent"),
        "warehouse Root must retain native collaboration tools",
    );
    assert!(
        root_user_text.contains("<name>warehouse-supervisor</name>")
            && root_user_text.contains("Root 只负责理解目标、协调 child"),
        "the first Root request omitted the selected Supervisor Skill body: {root_user_text}",
    );
    assert!(
        root_developer_text.contains("### Available skills")
            && root_developer_text.contains("- warehouse-supervisor: ")
            && ["warehouse-data", "warehouse-network-planning", "warehouse-map-delivery"]
            .iter()
            .all(|skill| !root_developer_text.contains(&format!("- {skill}: "))),
        "the multi-Agent Root exposed task Skills outside its declared scope: {root_developer_text}",
    );
    assert!(
        !root_user_text.contains("<name>warehouse-data</name>")
            && !root_user_text.contains("<name>warehouse-network-planning</name>"),
        "unselected task Skills leaked their bodies into Root context: {root_user_text}",
    );

    let data_spawn_output = wait_for_function_output(&model_control, DATA_SPAWN_CALL).await;
    assert!(
        data_spawn_output.to_string().contains("runtime_gate_data"),
        "native Data child spawn failed: {data_spawn_output}",
    );

    let data_thread_id =
        wait_for_direct_child_thread(&host, &root.thread_id, "data_agent", &[]).await;
    let data_request =
        wait_for_model_request(&model_control, DATA_CHILD_PROMPT, DATA_SPAWN_CALL).await;
    assert_role_has_no_shell_tools(&data_request, "data_agent", true);
    assert!(
        request_tool_names(&data_request)
            .iter()
            .any(|name| name.contains("supply_chain_data") || name == "discover_workspace_sources"),
        "data_agent must retain its Data MCP surface",
    );
    assert_child_skill_policy(
        &data_request,
        &[(
            "warehouse-data",
            "仅供仓网 Supervisor 原生创建的 data_agent 使用",
        )],
        &["warehouse-data"],
    );
    let data_mcp = adapter
        .query_profile(ProfileQuery::McpServers {
            cursor: None,
            limit: None,
            thread_id: Some(data_thread_id.clone()),
        })
        .await
        .expect("query Data child MCP inventory");
    assert_role_mcp_inventory(&data_mcp, &[("supply_chain_data", &DATA_TOOLS[..])]);

    let network_thread_id = wait_for_direct_child_thread(
        &host,
        &root.thread_id,
        "network_agent",
        &[data_thread_id.as_str()],
    )
    .await;
    let network_request =
        wait_for_model_request(&model_control, NETWORK_CHILD_PROMPT, NETWORK_SPAWN_CALL).await;
    assert_role_has_no_shell_tools(&network_request, "network_agent", true);
    assert!(
        request_tool_names(&network_request)
            .iter()
            .any(|name| name.contains("supply_chain") || name == "prepare_route_matrix"),
        "network_agent must retain its Network MCP surface",
    );
    let network_instructions = network_request["instructions"]
        .as_str()
        .expect("Network Role base instructions");
    assert!(network_instructions.contains("focused warehouse-network analysis Agent"));
    assert!(!network_instructions.contains("You are a coding agent"));
    let network_developer_text = request_developer_text(&network_request);
    assert!(network_developer_text.contains("warehouse-network"));
    assert!(!network_developer_text.contains("imagegen:"));
    assert!(!network_developer_text.contains("<permissions instructions>"));
    assert_child_skill_policy(
        &network_request,
        &[
            ("warehouse-network-planning", "仅供 network_agent 使用"),
            ("warehouse-map-delivery", "仅供 network_agent 使用"),
        ],
        &["warehouse-network-planning", "warehouse-map-delivery"],
    );
    let network_mcp = adapter
        .query_profile(ProfileQuery::McpServers {
            cursor: None,
            limit: None,
            thread_id: Some(network_thread_id.clone()),
        })
        .await
        .expect("query Network child MCP inventory");
    assert_role_mcp_inventory(
        &network_mcp,
        &[
            ("supply_chain", &NETWORK_TOOLS[..]),
            ("map_utils", &MAP_TOOLS[..]),
        ],
    );

    // The official reload queues the latest effective Thread configuration.
    // This Role-local projection has no delta: Role files are re-read only by
    // the next spawn, while this existing child's thread_config stays fixed.
    // Therefore the safe-boundary reload is a native no-op and correctly emits
    // no fabricated startup transition. The follow-up inventory below is the
    // authoritative observation that the live projection remains available.
    host.request("config/mcpServer/reload", Value::Null)
        .await
        .expect("queue native MCP configuration reload");
    model_control.release_data_child.add_permits(1);
    let data_followup_request =
        wait_for_model_request(&model_control, DATA_FOLLOWUP_PROMPT, DATA_FOLLOWUP_CALL).await;
    assert_child_skill_policy(
        &data_followup_request,
        &[(
            "warehouse-data",
            "仅供仓网 Supervisor 原生创建的 data_agent 使用",
        )],
        &["warehouse-data"],
    );
    let reloaded_data_mcp = adapter
        .query_profile(ProfileQuery::McpServers {
            cursor: None,
            limit: None,
            thread_id: Some(data_thread_id.clone()),
        })
        .await
        .expect("query reloaded Data child MCP inventory");
    assert_role_mcp_inventory(
        &reloaded_data_mcp,
        &[("supply_chain_data", &DATA_TOOLS[..])],
    );
    model_control.release_data_followup.add_permits(1);
    model_control.release_network_child.add_permits(1);

    timeout(Duration::from_secs(30), async {
        let expected = BTreeMap::from([
            (root.thread_id.clone(), 1_usize),
            (data_thread_id.clone(), 2_usize),
            (network_thread_id.clone(), 1_usize),
        ]);
        let mut completed = BTreeMap::<String, usize>::new();
        while completed != expected {
            let event = terminal_events
                .recv()
                .await
                .expect("terminal Runtime event")
                .message;
            if event["method"] == "turn/completed"
                && event["params"]["turn"]["status"].as_str() == Some("completed")
            {
                if let Some(thread_id) = event_thread_id(&event) {
                    if expected.contains_key(thread_id) {
                        *completed.entry(thread_id.to_string()).or_default() += 1;
                    }
                }
            }
        }
    })
    .await
    .expect("Root, both native children, and the Data follow-up Turn complete");

    // The checked-in Skill is only the clean-Profile seed. Runtime edits happen
    // on this Profile-owned copy and are observed by Codex's native watcher.
    let data_skill_path = profile_home.join("skills/warehouse-data/SKILL.md");
    let data_skill = std::fs::read_to_string(&data_skill_path).expect("read Profile Data Skill");
    let hot_data_skill = data_skill.replacen(
        "description: ",
        &format!("description: {SKILL_HOT_MARKER} "),
        1,
    );
    assert_ne!(hot_data_skill, data_skill, "Data Skill has frontmatter");
    let mut skill_events = host.subscribe();
    std::fs::write(&data_skill_path, hot_data_skill).expect("update Profile Data Skill");
    wait_for_event_method(&mut skill_events, "skills/changed").await;
    let refreshed_skills = adapter
        .query_profile(ProfileQuery::Skills {
            workspace: workspace.clone(),
            force_reload: true,
        })
        .await
        .expect("force reload native Skills after Profile edit");
    let refreshed_data_skill = response_skills(&refreshed_skills)
        .find(|skill| skill["name"].as_str() == Some("warehouse-data"))
        .expect("refreshed warehouse-data Skill");
    assert!(
        refreshed_data_skill["description"]
            .as_str()
            .is_some_and(|description| description.contains(SKILL_HOT_MARKER)),
        "forceReload did not observe the Profile Skill edit: {refreshed_data_skill}",
    );
    assert_eq!(
        response_skill_names(&refreshed_skills, "user"),
        BTreeSet::from(WAREHOUSE_SKILLS.map(str::to_string)),
    );

    // Role contents are re-read at native spawn time. Editing the Profile Role
    // must affect the next child without mutating the already-captured child.
    let data_role_path = profile_home.join("agents/data_agent.toml");
    let mut data_role = std::fs::read_to_string(&data_role_path)
        .expect("read Profile Data Role")
        .parse::<DocumentMut>()
        .expect("parse Profile Data Role");
    let original_instructions = data_role["developer_instructions"]
        .as_str()
        .expect("Data Role developer instructions");
    data_role["developer_instructions"] =
        value(format!("{original_instructions}\n{ROLE_HOT_MARKER}"));
    std::fs::write(&data_role_path, data_role.to_string()).expect("update Profile Data Role");
    assert!(
        !request_developer_text(&data_request).contains(ROLE_HOT_MARKER),
        "an existing child request cannot be retroactively changed",
    );

    let mut hot_terminal_events = host.subscribe();
    adapter
        .send_user_message(
            &workspace,
            &root.thread_id,
            HOT_ROOT_PROMPT,
            &TurnOptions {
                access_mode: Some("read-only".to_string()),
                ..TurnOptions::default()
            },
        )
        .await
        .expect("start Root Turn for next Role spawn");
    let hot_data_thread_id = wait_for_direct_child_thread(
        &host,
        &root.thread_id,
        "data_agent",
        &[data_thread_id.as_str(), network_thread_id.as_str()],
    )
    .await;
    let hot_data_request =
        wait_for_model_request(&model_control, HOT_DATA_PROMPT, HOT_DATA_SPAWN_CALL).await;
    assert_child_skill_policy(
        &hot_data_request,
        &[("warehouse-data", SKILL_HOT_MARKER)],
        &["warehouse-data"],
    );
    assert!(
        request_developer_text(&hot_data_request).contains(ROLE_HOT_MARKER),
        "the next native Role spawn did not re-read the Profile Role",
    );

    timeout(Duration::from_secs(30), async {
        let expected = BTreeSet::from([root.thread_id.clone(), hot_data_thread_id.clone()]);
        let mut completed = BTreeSet::new();
        while completed != expected {
            let event = hot_terminal_events
                .recv()
                .await
                .expect("hot Role terminal event")
                .message;
            if event["method"] == "turn/completed"
                && event["params"]["turn"]["status"].as_str() == Some("completed")
            {
                if let Some(thread_id) = event_thread_id(&event) {
                    if expected.contains(thread_id) {
                        completed.insert(thread_id.to_string());
                    }
                }
            }
        }
    })
    .await
    .expect("Root and next native Data child complete");

    host.shutdown().await.expect("shutdown Profile Host");
    model_server.abort();
}

/// Real Platform bridge gate for a native child MCP form elicitation.
///
/// Run with:
///
/// `CODEX_BIN=/absolute/path/to/codex TEST_DATABASE_URL=postgres://... cargo test \
/// -p open-web-codex-server copilot_package_child_mcp_form_bridge_gate \
/// -- --ignored --exact`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires a real Codex CLI, prepared Copilot descriptor, and disposable PostgreSQL database"]
async fn copilot_package_child_mcp_form_bridge_gate() {
    let codex_bin = required_path(
        Path::new(&std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set")),
        "Codex binary",
    );
    let database_url =
        std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is set for a disposable DB");
    let repository = required_path(&repository_root(), "repository root");
    let assets = CopilotPackageAssets::resolve(
        &warehouse_copilot_package(&repository),
        &prepared_network_copilot_descriptor(&repository),
        &prepared_network_copilot_build_store(&repository),
    )
    .expect("resolve configured package assets");

    let test_root = runtime_probe_tempdir();
    let profile_home = test_root.path().join("profile");
    let runner_root = test_root.path().join("runner");
    let workspace_root = runner_root.join("workspace");
    std::fs::create_dir_all(&profile_home).expect("create Profile home");
    std::fs::create_dir_all(&workspace_root).expect("create authorized Workspace");
    let (model_uri, model_control, model_server) = start_runtime_model_server().await;
    std::fs::write(
        profile_home.join("config.toml"),
        format!(
            r#"model = "mock-model"
model_provider = "mock_provider"
approval_policy = "on-request"
sandbox_mode = "read-only"

[features]
multi_agent_v2 = true
auth_elicitation = true

[skills]
include_instructions = true

[model_providers.mock_provider]
name = "Child MCP form Runtime gate provider"
base_url = "{model_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
models = [{{ model_id = "mock-model", context_window = 25600 }}]
"#,
        ),
    )
    .expect("write mock Profile config");
    let host_config = ProfileHostConfig::new(
        "package-network-child-form-runtime-gate",
        &profile_home,
        &runner_root,
    )
    .with_codex_bin(&codex_bin)
    .with_startup_files(
        assets
            .startup_files(&profile_home)
            .expect("render production Profile seeds"),
    );
    let host = ProfileHost::spawn(host_config.clone())
        .await
        .expect("spawn real Profile Host");
    let mcp_initialize_requests = Arc::new(Mutex::new(Vec::new()));
    let mcp_elicitation_requests = Arc::new(Mutex::new(Vec::new()));
    let (form_server_url, form_server) = start_form_gate_server(
        Arc::clone(&mcp_initialize_requests),
        Arc::clone(&mcp_elicitation_requests),
    )
    .await;
    install_form_gate_server(&profile_home, &form_server_url);

    let adapter = Arc::new(
        RealCodexAdapter::from_host(
            host.clone(),
            "package-network-child-form-runner",
            runner_root.clone(),
        )
        .expect("construct real Codex adapter"),
    );
    let workspace = AuthorizedWorkspace {
        id: "warehouse-form-workspace".to_string(),
        root: workspace_root.canonicalize().expect("canonical Workspace"),
    };
    let root = adapter
        .start_thread(&workspace, None)
        .await
        .expect("start Standard Root Thread");
    let store = seed_form_gate_store(&database_url, &workspace.root, &root.thread_id).await;
    let approvals = ApprovalService::new(store.pool.clone(), &store.runtime_key);
    let runtime_instance_id = adapter.runtime_instance_id().await;
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();
    let subscription_adapter = Arc::clone(&adapter);
    let subscription = tokio::spawn(async move {
        subscription_adapter
            .subscribe_events(event_tx)
            .await
            .expect("adapter event subscription")
    });
    let root_turn = adapter
        .send_user_message(
            &workspace,
            &root.thread_id,
            FORM_ROOT_PROMPT,
            &TurnOptions {
                access_mode: Some("read-only".to_string()),
                ..TurnOptions::default()
            },
        )
        .await
        .expect("start Root collaboration Turn");
    let root_turn_id = root_turn["turnId"]
        .as_str()
        .expect("Root Turn id")
        .to_string();
    let spawn_result_request =
        wait_for_model_request(&model_control, FORM_SPAWN_CALL, "no-excluded-marker").await;
    assert!(
        !spawn_result_request.to_string().contains("failed")
            && !spawn_result_request
                .to_string()
                .contains("unknown agent_type"),
        "native Data Role spawn failed before form elicitation: {spawn_result_request}",
    );
    let child_thread_id =
        wait_for_direct_child_thread(&host, &root.thread_id, "data_agent", &[]).await;
    wait_for_model_request(&model_control, FORM_CHILD_PROMPT, FORM_SPAWN_CALL).await;
    store.bind_child(&root.thread_id, &child_thread_id).await;

    // The official app-server route must exercise the existing native child
    // MCP manager before the model is released to make its own tool call. This
    // keeps a failure localized: a direct failure belongs to the Role-local
    // manager/client, while a later model-only failure belongs to its Turn
    // snapshot or scheduling path. It is not a Platform production bypass.
    let direct_host = host.clone();
    let direct_child_thread_id = child_thread_id.clone();
    let mut direct_tool_call = tokio::spawn(async move {
        direct_host
            .request_long_running(
                "mcpServer/tool/call",
                json!({
                    "threadId": direct_child_thread_id,
                    "server": FORM_SERVER,
                    "tool": FORM_TOOL,
                    "arguments": { "scenario": "accept" },
                }),
            )
            .await
    });
    let direct_form_id = tokio::select! {
        approval_id = capture_next_form(&mut event_rx, &approvals) => approval_id,
        direct_result = &mut direct_tool_call => {
            panic!(
                "official direct child mcpServer/tool/call finished before a durable form request: {direct_result:?}"
            )
        }
    };
    respond_to_form(
        &approvals,
        &adapter,
        store.actor,
        runtime_instance_id,
        direct_form_id,
        McpFormResponseAction::Accept,
        Some(BTreeMap::from([
            ("method".to_string(), json!("curve")),
            ("factor".to_string(), json!(1.0)),
        ])),
    )
    .await;
    let direct_tool_result = timeout(Duration::from_secs(30), &mut direct_tool_call)
        .await
        .expect("official direct child MCP tool response")
        .expect("official direct child MCP tool task")
        .expect("official direct child MCP tool result");
    assert!(
        direct_tool_result.to_string().contains("accepted"),
        "official direct child MCP tool did not receive the typed form response: {direct_tool_result}"
    );

    model_control.release_form_child.add_permits(1);

    let accept_id = capture_form_before_tool_completion(
        &mut event_rx,
        &approvals,
        &model_control,
        FORM_ACCEPT_CALL,
        &mcp_initialize_requests,
        &mcp_elicitation_requests,
    )
    .await;
    let refreshed = approvals
        .list_pending_mcp_forms(store.actor, runtime_instance_id, store.run_id)
        .await
        .expect("read typed pending form after refresh");
    let refreshed_again = approvals
        .list_pending_mcp_forms(store.actor, runtime_instance_id, store.run_id)
        .await
        .expect("restore typed pending form on a second refresh");
    assert_eq!(refreshed, refreshed_again);
    assert_eq!(refreshed.len(), 1);
    assert_eq!(refreshed[0].id, accept_id);
    assert_eq!(refreshed[0].server_name, FORM_SERVER);
    assert_eq!(refreshed[0].fields.len(), 2);
    assert!(matches!(
        &refreshed[0].source,
        McpFormRequestSource::Agent { display_title, .. } if display_title == "Wanwan"
    ));
    assert!(
        approvals
            .list_pending(store.actor, runtime_instance_id)
            .await
            .expect("generic approvals remain available")
            .is_empty(),
        "typed MCP forms must not leak into the generic approval contract",
    );

    adapter
        .steer_turn(
            &workspace,
            None,
            &root.thread_id,
            &root_turn_id,
            FORM_ROOT_STEER,
            &[],
        )
        .await
        .expect("Root accepts steer while native child waits for MCP form input");
    model_control.release_form_root.add_permits(1);
    wait_for_model_request(&model_control, FORM_ROOT_STEER, "no-excluded-marker").await;

    respond_to_form(
        &approvals,
        &adapter,
        store.actor,
        runtime_instance_id,
        accept_id,
        McpFormResponseAction::Accept,
        Some(BTreeMap::from([
            ("method".to_string(), json!("navigation")),
            ("factor".to_string(), json!(1.35)),
        ])),
    )
    .await;
    let accepted_request =
        wait_for_model_request(&model_control, FORM_ACCEPT_CALL, "no-excluded-marker").await;
    assert!(
        accepted_request.to_string().contains("navigation")
            && accepted_request.to_string().contains("1.35")
            && accepted_request.to_string().contains("accept"),
        "the same child did not receive the user's non-default values: {accepted_request}",
    );

    let decline_id = capture_next_form(&mut event_rx, &approvals).await;
    respond_to_form(
        &approvals,
        &adapter,
        store.actor,
        runtime_instance_id,
        decline_id,
        McpFormResponseAction::Decline,
        None,
    )
    .await;
    let declined_request =
        wait_for_model_request(&model_control, FORM_DECLINE_CALL, "no-excluded-marker").await;
    assert!(declined_request.to_string().contains("decline"));

    let cancel_id = capture_next_form(&mut event_rx, &approvals).await;
    respond_to_form(
        &approvals,
        &adapter,
        store.actor,
        runtime_instance_id,
        cancel_id,
        McpFormResponseAction::Cancel,
        None,
    )
    .await;
    let cancelled_request =
        wait_for_model_request(&model_control, FORM_CANCEL_CALL, "no-excluded-marker").await;
    assert!(cancelled_request.to_string().contains("cancel"));

    let restart_pending_id = capture_next_form(&mut event_rx, &approvals).await;
    assert_eq!(
        approvals
            .list_pending_mcp_forms(store.actor, runtime_instance_id, store.run_id)
            .await
            .expect("read pending form before Runtime restart")[0]
            .id,
        restart_pending_id,
    );
    let safe_restart = host
        .restart(host_config.clone())
        .await
        .expect_err("safe restart must retain a native child waiting on form input");
    assert!(matches!(safe_restart, ProfileHostError::RuntimeBusy));
    assert_eq!(host.runtime_instance_id().await, runtime_instance_id);
    assert_eq!(
        approvals
            .list_pending_mcp_forms(store.actor, runtime_instance_id, store.run_id)
            .await
            .expect("safe restart refusal preserves the pending native form")[0]
            .id,
        restart_pending_id,
    );

    host.shutdown()
        .await
        .expect("terminate Profile Host with a pending native child request");
    host.restart(host_config)
        .await
        .expect("start a new Profile Runtime after process termination");
    let restarted_runtime_instance_id = host.runtime_instance_id().await;
    assert_ne!(restarted_runtime_instance_id, runtime_instance_id);
    assert!(approvals
        .list_pending_mcp_forms(store.actor, restarted_runtime_instance_id, store.run_id)
        .await
        .expect("reconcile stale pending form after Runtime restart")
        .is_empty());
    let restart_state: String = sqlx::query_scalar("SELECT state FROM approvals WHERE id = $1")
        .bind(restart_pending_id)
        .fetch_one(&store.pool)
        .await
        .expect("read restarted pending approval state");
    assert_eq!(restart_state, "cancelled");

    subscription.abort();
    host.shutdown().await.expect("shutdown Profile Host");
    form_server.abort();
    model_server.abort();
}

/// Real unavailable gate for a malformed Profile-owned native Role.
///
/// Run with:
///
/// `CODEX_BIN=/absolute/path/to/codex cargo test -p open-web-codex-server \
/// copilot_package_malformed_role_is_unavailable -- --ignored --exact`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires a real Codex CLI and prepared warehouse MCP environments"]
async fn copilot_package_malformed_role_is_unavailable() {
    let codex_bin = required_path(
        Path::new(&std::env::var_os("CODEX_BIN").expect("CODEX_BIN is set")),
        "Codex binary",
    );
    let repository = required_path(&repository_root(), "repository root");
    let assets = CopilotPackageAssets::resolve(
        &warehouse_copilot_package(&repository),
        &prepared_network_copilot_descriptor(&repository),
        &prepared_network_copilot_build_store(&repository),
    )
    .expect("resolve configured package assets");

    let test_root = runtime_probe_tempdir();
    let profile_home = test_root.path().join("profile");
    let runner_root = test_root.path().join("runner");
    let workspace_root = runner_root.join("workspace");
    std::fs::create_dir_all(&profile_home).expect("create Profile home");
    std::fs::create_dir_all(&workspace_root).expect("create authorized Workspace");
    let (model_uri, model_control, model_server) = start_runtime_model_server().await;
    std::fs::write(
        profile_home.join("config.toml"),
        format!(
            r#"model = "mock-model"
model_provider = "mock_provider"
approval_policy = "never"
sandbox_mode = "read-only"

[features]
multi_agent_v2 = true

[model_providers.mock_provider]
name = "Malformed Role Runtime gate provider"
base_url = "{model_uri}/v1"
wire_api = "responses"
request_max_retries = 0
stream_max_retries = 0
models = [{{ model_id = "mock-model", context_window = 25600 }}]
"#
        ),
    )
    .expect("write mock Profile config");

    let host_config = ProfileHostConfig::new(
        "package-network-malformed-role-gate",
        &profile_home,
        &runner_root,
    )
    .with_codex_bin(&codex_bin)
    .with_startup_files(
        assets
            .startup_files(&profile_home)
            .expect("render production Profile seeds"),
    );
    let host = ProfileHost::spawn(host_config.clone())
        .await
        .expect("spawn clean Profile Host");

    let malformed_role_path = profile_home.join("agents/broken_agent.toml");
    std::fs::write(
        &malformed_role_path,
        "name = \"broken_agent\"\ndescription = \"invalid role\"\ndeveloper_instructions = [\n",
    )
    .expect("write malformed Profile Role");
    let mut warning_events = host.subscribe();
    host.restart(host_config)
        .await
        .expect("restart Profile Host with malformed Role");
    let warning = wait_for_event_method(&mut warning_events, "configWarning").await;
    assert!(
        warning["params"]["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("Ignoring malformed agent role definition")),
        "Runtime omitted the malformed Role configWarning: {warning}",
    );

    let adapter = RealCodexAdapter::from_host(
        host.clone(),
        "package-network-malformed-role-runner",
        runner_root,
    )
    .expect("construct real Codex adapter");
    let workspace = AuthorizedWorkspace {
        id: "malformed-role-workspace".to_string(),
        root: workspace_root.canonicalize().expect("canonical Workspace"),
    };
    let root = adapter
        .start_thread(&workspace, None)
        .await
        .expect("start Standard Root Thread");
    let mut runtime_events = host.subscribe();
    adapter
        .send_user_message(
            &workspace,
            &root.thread_id,
            MALFORMED_ROOT_PROMPT,
            &TurnOptions {
                access_mode: Some("read-only".to_string()),
                ..TurnOptions::default()
            },
        )
        .await
        .expect("ask Root to spawn malformed Role");

    timeout(Duration::from_secs(30), async {
        loop {
            let event = runtime_events
                .recv()
                .await
                .expect("malformed Role Runtime event")
                .message;
            if event["method"] == "turn/completed"
                && event_thread_id(&event) == Some(root.thread_id.as_str())
            {
                assert_eq!(
                    event["params"]["turn"]["status"].as_str(),
                    Some("completed"),
                    "Root must receive the tool failure and terminate normally: {event}",
                );
                return;
            }
        }
    })
    .await
    .expect("Root handles malformed Role failure");

    let malformed_children =
        list_direct_child_thread_ids(&host, &root.thread_id, "broken_agent", &[]).await;
    assert!(
        malformed_children.is_empty(),
        "a malformed unavailable Role must not create a child: {malformed_children:?}",
    );

    let failed_spawn_request = model_control
        .requests
        .lock()
        .await
        .iter()
        .find(|request| request.to_string().contains(MALFORMED_SPAWN_CALL))
        .cloned()
        .expect("Root receives malformed Role tool result");
    assert!(
        failed_spawn_request
            .to_string()
            .contains("unknown agent_type 'broken_agent'"),
        "Root did not receive the explicit native spawn failure: {failed_spawn_request}",
    );

    host.shutdown().await.expect("shutdown Profile Host");
    model_server.abort();
}
