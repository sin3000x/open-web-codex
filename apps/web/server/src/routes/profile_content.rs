use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxumPath, Query, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use open_web_codex_adapter::{CodexAdapter, ProfileMutation, ProfileQuery};
use open_web_codex_platform_contracts::error::PlatformError;
use open_web_codex_platform_contracts::{
    AgentSummary, AgentsSettings, CreateAgentRequest, CreatePromptRequest, DeleteAgentQuery,
    DeletePromptRequest, MovePromptRequest, ProfileTextFile, PromptEntry, PromptListQuery,
    RememberApprovalRuleRequest, SetAgentsCoreRequest, SetExperimentalFeatureRequest,
    UpdateAgentRequest, UpdatePromptRequest, WriteProfileTextFileRequest,
};
use open_web_codex_platform_store::AppState;
use serde_json::{json, Value};
use sqlx::Row;
use toml_edit::{value, DocumentMut};
use uuid::Uuid;

use crate::middleware::auth::{require_runtime_profile, AuthenticatedUser};
use crate::routes::RuntimeProfileBinding;

type ApiError = (StatusCode, Json<PlatformError>);
type ApiResult<T> = Result<Json<T>, ApiError>;

const MAX_PROFILE_TEXT_BYTES: usize = 2 * 1024 * 1024;
const DEFAULT_MAX_THREADS: u32 = 6;
const DEFAULT_MAX_DEPTH: u32 = 1;

static RULES_WRITE_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

pub async fn set_experimental_feature(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(name): AxumPath<String>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<SetExperimentalFeatureRequest>,
) -> ApiResult<Value> {
    authorize_profile(&state, &auth, &profile).await?;
    let name = validate_identifier(&name, "feature")?;
    adapter
        .mutate_profile(ProfileMutation::SetExperimentalFeature {
            name,
            enabled: request.enabled,
        })
        .await
        .map_err(runtime_write_error)?;
    Ok(Json(json!({ "status": "ok" })))
}

pub async fn read_profile_file(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(kind): AxumPath<String>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<ProfileTextFile> {
    authorize_profile(&state, &auth, &profile).await?;
    let root = profile_root(&profile)?;
    let path = profile_file_path(root, &kind)?;
    Ok(Json(read_text_file(&path).await?))
}

pub async fn write_profile_file(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(kind): AxumPath<String>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<WriteProfileTextFileRequest>,
) -> ApiResult<Value> {
    authorize_profile(&state, &auth, &profile).await?;
    validate_text_size(&request.content)?;
    let root = profile_root(&profile)?;
    let path = profile_file_path(root, &kind)?;
    if kind == "config" {
        request
            .content
            .parse::<DocumentMut>()
            .map_err(|_| bad_request("Codex configuration is not valid TOML"))?;
    }
    let previous = tokio::fs::read(&path).await.ok();
    atomic_write(&path, request.content.as_bytes()).await?;
    if kind == "config" && adapter.query_profile(ProfileQuery::Config).await.is_err() {
        match previous {
            Some(previous) => {
                let _ = atomic_write(&path, &previous).await;
            }
            None => {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
        return Err(bad_request("Codex rejected the updated configuration"));
    }
    Ok(Json(json!({ "status": "ok" })))
}

pub async fn get_agents(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<AgentsSettings> {
    authorize_profile(&state, &auth, &profile).await?;
    agents_settings(adapter.as_ref(), profile_root(&profile)?)
        .await
        .map(Json)
}

pub async fn get_config_model(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<Value> {
    authorize_profile(&state, &auth, &profile).await?;
    let value = adapter
        .query_profile(ProfileQuery::Config)
        .await
        .map_err(|_| runtime_read_error())?;
    let model = value
        .pointer("/config/model")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(Json(json!({ "model": model })))
}

pub async fn set_agents_core(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<SetAgentsCoreRequest>,
) -> ApiResult<AgentsSettings> {
    authorize_profile(&state, &auth, &profile).await?;
    if !(1..=12).contains(&request.max_threads) || !(1..=4).contains(&request.max_depth) {
        return Err(bad_request(
            "Agent concurrency settings are outside supported limits",
        ));
    }
    adapter
        .mutate_profile(ProfileMutation::SetAgentCore {
            multi_agent_enabled: request.multi_agent_enabled,
            max_threads: request.max_threads,
            max_depth: request.max_depth,
        })
        .await
        .map_err(runtime_write_error)?;
    agents_settings(adapter.as_ref(), profile_root(&profile)?)
        .await
        .map(Json)
}

pub async fn create_agent(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<CreateAgentRequest>,
) -> ApiResult<AgentsSettings> {
    authorize_profile(&state, &auth, &profile).await?;
    let root = profile_root(&profile)?;
    let name = validate_identifier(&request.name, "agent")?;
    let current = agents_settings(adapter.as_ref(), root).await?;
    if current.agents.iter().any(|agent| agent.name == name) {
        return Err(conflict("Agent already exists"));
    }
    let path = managed_agent_path(root, &name);
    if tokio::fs::try_exists(&path).await.map_err(io_error)? {
        return Err(conflict("Managed Agent configuration already exists"));
    }
    let contents = new_agent_document(&request)?;
    atomic_write(&path, contents.as_bytes()).await?;
    let mutation = ProfileMutation::SetAgentDefinition {
        original_name: None,
        name: name.clone(),
        description: normalized(request.description),
        config_file: managed_agent_config_file(&name),
    };
    if adapter.mutate_profile(mutation).await.is_err() {
        let _ = tokio::fs::remove_file(&path).await;
        return Err(runtime_write_error(
            open_web_codex_adapter::AdapterError::Rpc(
                "Agent configuration update failed".to_string(),
            ),
        ));
    }
    agents_settings(adapter.as_ref(), root).await.map(Json)
}

pub async fn update_agent(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(original_name): AxumPath<String>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<UpdateAgentRequest>,
) -> ApiResult<AgentsSettings> {
    authorize_profile(&state, &auth, &profile).await?;
    let root = profile_root(&profile)?;
    let original_name = validate_identifier(&original_name, "agent")?;
    let name = validate_identifier(&request.name, "agent")?;
    let current = agents_settings(adapter.as_ref(), root).await?;
    let existing = current
        .agents
        .iter()
        .find(|agent| agent.name == original_name)
        .cloned()
        .ok_or_else(|| not_found("Agent was not found"))?;
    if name != original_name && current.agents.iter().any(|agent| agent.name == name) {
        return Err(conflict("Agent already exists"));
    }

    let original_path = managed_agent_path_from_config(root, &existing.config_file);
    if request.developer_instructions.is_some() && original_path.is_none() {
        return Err(bad_request(
            "External Agent configuration cannot be edited through the browser",
        ));
    }
    let rename_file = request.rename_managed_file.unwrap_or(true)
        && name != original_name
        && original_path.is_some();
    let next_path = if rename_file {
        Some(managed_agent_path(root, &name))
    } else {
        original_path.clone()
    };
    let previous = match original_path.as_ref() {
        Some(path) => tokio::fs::read(path).await.unwrap_or_default(),
        None => Vec::new(),
    };
    if let Some(next_path) = next_path.as_ref() {
        let mut document = String::from_utf8(previous.clone())
            .unwrap_or_default()
            .parse::<DocumentMut>()
            .unwrap_or_default();
        if let Some(instructions) = normalized(request.developer_instructions.clone()) {
            document["developer_instructions"] = value(instructions);
        }
        atomic_write(next_path, document.to_string().as_bytes()).await?;
    }
    let config_file = if rename_file {
        managed_agent_config_file(&name)
    } else {
        existing.config_file.clone()
    };
    let mutation = ProfileMutation::SetAgentDefinition {
        original_name: Some(original_name.clone()),
        name: name.clone(),
        description: normalized(request.description),
        config_file,
    };
    if adapter.mutate_profile(mutation).await.is_err() {
        if let Some(next_path) = next_path.as_ref() {
            if original_path.as_ref() != Some(next_path) {
                let _ = tokio::fs::remove_file(next_path).await;
            } else {
                let _ = atomic_write(next_path, &previous).await;
            }
        }
        return Err(runtime_write_error(
            open_web_codex_adapter::AdapterError::Rpc(
                "Agent configuration update failed".to_string(),
            ),
        ));
    }
    if rename_file {
        if let Some(original_path) = original_path {
            let _ = tokio::fs::remove_file(original_path).await;
        }
    }
    agents_settings(adapter.as_ref(), root).await.map(Json)
}

pub async fn delete_agent(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(name): AxumPath<String>,
    Query(query): Query<DeleteAgentQuery>,
    Extension(adapter): Extension<Arc<dyn CodexAdapter>>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<AgentsSettings> {
    authorize_profile(&state, &auth, &profile).await?;
    let root = profile_root(&profile)?;
    let name = validate_identifier(&name, "agent")?;
    let current = agents_settings(adapter.as_ref(), root).await?;
    let existing = current
        .agents
        .iter()
        .find(|agent| agent.name == name)
        .ok_or_else(|| not_found("Agent was not found"))?;
    let managed_path = managed_agent_path_from_config(root, &existing.config_file);
    if query.delete_managed_file.unwrap_or(false) && managed_path.is_none() {
        return Err(bad_request(
            "External Agent configuration cannot be deleted through the browser",
        ));
    }
    adapter
        .mutate_profile(ProfileMutation::RemoveAgentDefinition { name: name.clone() })
        .await
        .map_err(runtime_write_error)?;
    if query.delete_managed_file.unwrap_or(false) {
        let path = managed_path.expect("managed path was validated above");
        match tokio::fs::remove_file(path).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
    }
    agents_settings(adapter.as_ref(), root).await.map(Json)
}

pub async fn read_agent_config(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(name): AxumPath<String>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<String> {
    authorize_profile(&state, &auth, &profile).await?;
    let name = validate_identifier(&name, "agent")?;
    let path = managed_agent_path(profile_root(&profile)?, &name);
    let content = match tokio::fs::read_to_string(path).await {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(io_error(error)),
    };
    Ok(Json(content))
}

pub async fn write_agent_config(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    AxumPath(name): AxumPath<String>,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<WriteProfileTextFileRequest>,
) -> ApiResult<Value> {
    authorize_profile(&state, &auth, &profile).await?;
    validate_text_size(&request.content)?;
    let name = validate_identifier(&name, "agent")?;
    request
        .content
        .parse::<DocumentMut>()
        .map_err(|_| bad_request("Agent configuration is not valid TOML"))?;
    atomic_write(
        &managed_agent_path(profile_root(&profile)?, &name),
        request.content.as_bytes(),
    )
    .await?;
    Ok(Json(json!({ "status": "ok" })))
}

pub async fn list_prompts(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Query(query): Query<PromptListQuery>,
    Extension(profile): Extension<RuntimeProfileBinding>,
) -> ApiResult<Vec<PromptEntry>> {
    let project_id = authorize_run_project(&state, &auth, query.run_id).await?;
    let root = profile_root(&profile)?;
    let mut prompts =
        discover_prompts(&prompt_dir(root, project_id, "workspace")?, "workspace").await?;
    prompts.extend(discover_prompts(&prompt_dir(root, project_id, "global")?, "global").await?);
    prompts.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then(left.scope.cmp(&right.scope))
    });
    Ok(Json(prompts))
}

pub async fn create_prompt(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<CreatePromptRequest>,
) -> ApiResult<PromptEntry> {
    let project_id = authorize_run_project(&state, &auth, request.run_id).await?;
    let name = validate_identifier(&request.name, "prompt")?;
    let scope = validate_scope(&request.scope)?;
    let directory = prompt_dir(profile_root(&profile)?, project_id, scope)?;
    let path = directory.join(format!("{name}.md"));
    if tokio::fs::try_exists(&path).await.map_err(io_error)? {
        return Err(conflict("Prompt already exists"));
    }
    let file = build_prompt_file(
        &request.description,
        &request.argument_hint,
        &request.content,
    )?;
    atomic_write(&path, file.as_bytes()).await?;
    Ok(Json(prompt_entry(
        name,
        scope,
        request.description,
        request.argument_hint,
        request.content,
    )))
}

pub async fn update_prompt(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<UpdatePromptRequest>,
) -> ApiResult<PromptEntry> {
    let project_id = authorize_run_project(&state, &auth, request.run_id).await?;
    let (scope, old_name) = parse_prompt_id(&request.path)?;
    let name = validate_identifier(&request.name, "prompt")?;
    let directory = prompt_dir(profile_root(&profile)?, project_id, scope)?;
    let old_path = directory.join(format!("{old_name}.md"));
    if !tokio::fs::try_exists(&old_path).await.map_err(io_error)? {
        return Err(not_found("Prompt was not found"));
    }
    let next_path = directory.join(format!("{name}.md"));
    if next_path != old_path && tokio::fs::try_exists(&next_path).await.map_err(io_error)? {
        return Err(conflict("Prompt already exists"));
    }
    let file = build_prompt_file(
        &request.description,
        &request.argument_hint,
        &request.content,
    )?;
    atomic_write(&next_path, file.as_bytes()).await?;
    if next_path != old_path {
        tokio::fs::remove_file(old_path).await.map_err(io_error)?;
    }
    Ok(Json(prompt_entry(
        name,
        scope,
        request.description,
        request.argument_hint,
        request.content,
    )))
}

pub async fn delete_prompt(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<DeletePromptRequest>,
) -> ApiResult<Value> {
    let project_id = authorize_run_project(&state, &auth, request.run_id).await?;
    let (scope, name) = parse_prompt_id(&request.path)?;
    let path = prompt_dir(profile_root(&profile)?, project_id, scope)?.join(format!("{name}.md"));
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_error(error)),
    }
    Ok(Json(json!({ "status": "ok" })))
}

pub async fn move_prompt(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<MovePromptRequest>,
) -> ApiResult<PromptEntry> {
    let project_id = authorize_run_project(&state, &auth, request.run_id).await?;
    let (old_scope, name) = parse_prompt_id(&request.path)?;
    let scope = validate_scope(&request.scope)?;
    if old_scope == scope {
        return Err(conflict("Prompt is already in that scope"));
    }
    let root = profile_root(&profile)?;
    let old_path = prompt_dir(root, project_id, old_scope)?.join(format!("{name}.md"));
    let next_path = prompt_dir(root, project_id, scope)?.join(format!("{name}.md"));
    if tokio::fs::try_exists(&next_path).await.map_err(io_error)? {
        return Err(conflict("Prompt already exists in the target scope"));
    }
    let content = tokio::fs::read_to_string(&old_path)
        .await
        .map_err(io_error)?;
    atomic_write(&next_path, content.as_bytes()).await?;
    tokio::fs::remove_file(old_path).await.map_err(io_error)?;
    let (description, argument_hint, body) = parse_frontmatter(&content);
    Ok(Json(prompt_entry(
        name,
        scope,
        description,
        argument_hint,
        body,
    )))
}

pub async fn remember_approval_rule(
    State(state): State<AppState>,
    auth: AuthenticatedUser,
    Extension(profile): Extension<RuntimeProfileBinding>,
    Json(request): Json<RememberApprovalRuleRequest>,
) -> ApiResult<Value> {
    authorize_run_project(&state, &auth, request.run_id).await?;
    let command = request
        .command
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if command.is_empty()
        || command.len() > 32
        || command.iter().any(|value| value.len() > 512)
        || command.iter().map(String::len).sum::<usize>() > 4096
    {
        return Err(bad_request("Invalid approval command prefix"));
    }
    let rule = format!(
        "prefix_rule(\n    pattern = [{}],\n    decision = \"allow\",\n)\n",
        command
            .iter()
            .map(|value| serde_json::to_string(value).expect("string serialization"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let path = profile_root(&profile)?.join("rules").join("default.rules");
    let lock = RULES_WRITE_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
    let _guard = lock.lock().await;
    let mut current = tokio::fs::read_to_string(&path).await.unwrap_or_default();
    if !current.contains(&rule) {
        if !current.is_empty() && !current.ends_with('\n') {
            current.push('\n');
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(&rule);
        atomic_write(&path, current.as_bytes()).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn agents_settings(
    adapter: &dyn CodexAdapter,
    root: &Path,
) -> Result<AgentsSettings, ApiError> {
    let value = adapter
        .query_profile(ProfileQuery::Config)
        .await
        .map_err(|_| runtime_read_error())?;
    let config = value.get("config").and_then(Value::as_object);
    let features = config
        .and_then(|value| value.get("features"))
        .and_then(Value::as_object);
    let agents = config
        .and_then(|value| value.get("agents"))
        .and_then(Value::as_object);
    let mut summaries = Vec::new();
    if let Some(agents) = agents {
        for (name, definition) in agents {
            if matches!(name.as_str(), "max_threads" | "max_depth")
                || validate_identifier(name, "agent").is_err()
            {
                continue;
            }
            let definition = definition.as_object();
            let description = definition
                .and_then(|value| value.get("description"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let config_file = definition
                .and_then(|value| value.get("config_file"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let managed_path = managed_agent_path_from_config(root, &config_file);
            let content = match managed_path.as_ref() {
                Some(path) => tokio::fs::read_to_string(path).await.ok(),
                None => None,
            };
            let developer_instructions = content
                .as_deref()
                .and_then(|content| content.parse::<DocumentMut>().ok())
                .and_then(|document| {
                    document
                        .get("developer_instructions")
                        .and_then(|item| item.as_str())
                        .map(str::to_string)
                });
            summaries.push(AgentSummary {
                name: name.clone(),
                description,
                developer_instructions,
                config_file,
                resolved_path: if managed_path.is_some() {
                    format!("profile://agents/{name}.toml")
                } else {
                    "profile://external-agent-config".to_string()
                },
                managed_by_app: managed_path.is_some(),
                file_exists: content.is_some(),
            });
        }
    }
    summaries.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(AgentsSettings {
        config_path: "profile://config.toml".to_string(),
        multi_agent_enabled: features
            .and_then(|value| value.get("multi_agent"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        max_threads: agents
            .and_then(|value| value.get("max_threads"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(DEFAULT_MAX_THREADS),
        max_depth: agents
            .and_then(|value| value.get("max_depth"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(DEFAULT_MAX_DEPTH),
        agents: summaries,
    })
}

async fn authorize_profile(
    state: &AppState,
    auth: &AuthenticatedUser,
    profile: &RuntimeProfileBinding,
) -> Result<(), ApiError> {
    require_runtime_profile(&state.db, auth, &profile.runtime_key).await
}

async fn authorize_run_project(
    state: &AppState,
    auth: &AuthenticatedUser,
    run_id: Uuid,
) -> Result<Uuid, ApiError> {
    let row = sqlx::query(
        "SELECT project_id, requested_by FROM runs WHERE id = $1 AND organization_id = $2",
    )
    .bind(run_id)
    .bind(auth.organization_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|_| database_error())?
    .ok_or_else(|| not_found("Run was not found"))?;
    let requested_by: Option<Uuid> = row.get("requested_by");
    if requested_by != Some(auth.user_id)
        && !matches!(auth.organization_role.as_str(), "owner" | "admin")
    {
        return Err(not_found("Run was not found"));
    }
    Ok(row.get("project_id"))
}

fn profile_root(profile: &RuntimeProfileBinding) -> Result<&Path, ApiError> {
    profile
        .codex_home
        .as_deref()
        .map(PathBuf::as_path)
        .ok_or_else(|| unavailable("Profile file storage is not configured"))
}

fn profile_file_path<'a>(root: &'a Path, kind: &str) -> Result<PathBuf, ApiError> {
    match kind {
        "agents" => Ok(root.join("AGENTS.md")),
        "config" => Ok(root.join("config.toml")),
        _ => Err(not_found("Profile file was not found")),
    }
}

fn managed_agent_path(root: &Path, name: &str) -> PathBuf {
    root.join("agents").join(format!("{name}.toml"))
}

fn managed_agent_config_file(name: &str) -> String {
    format!("agents/{name}.toml")
}

fn managed_agent_path_from_config(root: &Path, config_file: &str) -> Option<PathBuf> {
    let path = Path::new(config_file);
    let mut components = path.components();
    let directory = components.next()?.as_os_str().to_str()?;
    let file = components.next()?.as_os_str().to_str()?;
    if directory != "agents" || components.next().is_some() || !file.ends_with(".toml") {
        return None;
    }
    let name = file.strip_suffix(".toml")?;
    if validate_identifier(name, "agent").is_err() {
        return None;
    }
    Some(root.join(path))
}

fn new_agent_document(request: &CreateAgentRequest) -> Result<String, ApiError> {
    let mut document = DocumentMut::new();
    if request
        .template
        .as_deref()
        .is_some_and(|value| value != "blank")
    {
        return Err(bad_request("Unknown Agent template"));
    }
    if let Some(model) = normalized(request.model.clone()) {
        document["model"] = value(model);
    }
    if let Some(effort) = normalized(request.reasoning_effort.clone()) {
        document["model_reasoning_effort"] = value(effort);
    }
    if let Some(instructions) = normalized(request.developer_instructions.clone()) {
        document["developer_instructions"] = value(instructions);
    }
    Ok(document.to_string())
}

async fn read_text_file(path: &Path) -> Result<ProfileTextFile, ApiError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ProfileTextFile {
                exists: false,
                content: String::new(),
                truncated: false,
            });
        }
        Err(error) => return Err(io_error(error)),
    };
    let truncated = bytes.len() > MAX_PROFILE_TEXT_BYTES;
    let content =
        String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_PROFILE_TEXT_BYTES)]).to_string();
    Ok(ProfileTextFile {
        exists: true,
        content,
        truncated,
    })
}

async fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    if bytes.len() > MAX_PROFILE_TEXT_BYTES {
        return Err(bad_request("Profile text file is too large"));
    }
    let parent = path
        .parent()
        .ok_or_else(|| bad_request("Invalid Profile file"))?;
    tokio::fs::create_dir_all(parent).await.map_err(io_error)?;
    let temporary = parent.join(format!(".open-web-codex-{}.tmp", Uuid::now_v7()));
    tokio::fs::write(&temporary, bytes)
        .await
        .map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .await
            .map_err(io_error)?;
    }
    if let Err(error) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(io_error(error));
    }
    Ok(())
}

fn prompt_dir(root: &Path, project_id: Uuid, scope: &str) -> Result<PathBuf, ApiError> {
    match validate_scope(scope)? {
        "workspace" => Ok(root.join("web-prompts").join(project_id.to_string())),
        "global" => Ok(root.join("prompts")),
        _ => unreachable!(),
    }
}

async fn discover_prompts(directory: &Path, scope: &str) -> Result<Vec<PromptEntry>, ApiError> {
    tokio::fs::create_dir_all(directory)
        .await
        .map_err(io_error)?;
    let mut entries = tokio::fs::read_dir(directory).await.map_err(io_error)?;
    let mut prompts = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("md") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|value| value.to_str()) else {
            continue;
        };
        if validate_identifier(name, "prompt").is_err() {
            continue;
        }
        let Ok(content) = tokio::fs::read_to_string(&path).await else {
            continue;
        };
        if content.len() > MAX_PROFILE_TEXT_BYTES {
            continue;
        }
        let (description, argument_hint, body) = parse_frontmatter(&content);
        prompts.push(prompt_entry(
            name.to_string(),
            scope,
            description,
            argument_hint,
            body,
        ));
    }
    Ok(prompts)
}

fn prompt_entry(
    name: String,
    scope: &str,
    description: Option<String>,
    argument_hint: Option<String>,
    content: String,
) -> PromptEntry {
    PromptEntry {
        path: format!("{scope}:{name}"),
        name,
        description,
        argument_hint,
        content,
        scope: scope.to_string(),
    }
}

fn build_prompt_file(
    description: &Option<String>,
    argument_hint: &Option<String>,
    content: &str,
) -> Result<String, ApiError> {
    validate_text_size(content)?;
    let description = normalized(description.clone());
    let argument_hint = normalized(argument_hint.clone());
    if description.is_none() && argument_hint.is_none() {
        return Ok(content.to_string());
    }
    let mut output = String::from("---\n");
    if let Some(description) = description {
        output.push_str(&format!("description: {}\n", json!(description)));
    }
    if let Some(argument_hint) = argument_hint {
        output.push_str(&format!("argument-hint: {}\n", json!(argument_hint)));
    }
    output.push_str("---\n");
    output.push_str(content);
    validate_text_size(&output)?;
    Ok(output)
}

fn parse_frontmatter(content: &str) -> (Option<String>, Option<String>, String) {
    let Some(rest) = content.strip_prefix("---\n") else {
        return (None, None, content.to_string());
    };
    let Some((header, body)) = rest.split_once("\n---\n") else {
        return (None, None, content.to_string());
    };
    let mut description = None;
    let mut argument_hint = None;
    for line in header.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let parsed = serde_json::from_str::<String>(value.trim())
            .unwrap_or_else(|_| value.trim().trim_matches(['\'', '"']).to_string());
        match key.trim() {
            "description" => description = Some(parsed),
            "argument-hint" | "argument_hint" => argument_hint = Some(parsed),
            _ => {}
        }
    }
    (description, argument_hint, body.to_string())
}

fn parse_prompt_id(value: &str) -> Result<(&str, String), ApiError> {
    let (scope, name) = value
        .split_once(':')
        .ok_or_else(|| bad_request("Invalid prompt identifier"))?;
    let scope = validate_scope(scope)?;
    Ok((scope, validate_identifier(name, "prompt")?))
}

fn validate_scope(scope: &str) -> Result<&str, ApiError> {
    match scope {
        "workspace" | "global" => Ok(scope),
        _ => Err(bad_request("Invalid prompt scope")),
    }
}

fn validate_identifier(value: &str, label: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(bad_request(&format!("Invalid {label} name")));
    }
    if label == "agent" && matches!(value, "max_threads" | "max_depth") {
        return Err(bad_request("Agent name is reserved"));
    }
    Ok(value.to_string())
}

fn normalized(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn validate_text_size(value: &str) -> Result<(), ApiError> {
    if value.len() > MAX_PROFILE_TEXT_BYTES {
        return Err(bad_request("Profile text file is too large"));
    }
    Ok(())
}

fn bad_request(message: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(PlatformError::bad_request(message)),
    )
}

fn conflict(message: &str) -> ApiError {
    (
        StatusCode::CONFLICT,
        Json(PlatformError::bad_request(message)),
    )
}

fn not_found(message: &str) -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(PlatformError::not_found(message)),
    )
}

fn unavailable(message: &str) -> ApiError {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(PlatformError::internal(message)),
    )
}

fn runtime_read_error() -> ApiError {
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal(
            "Codex Profile configuration read failed",
        )),
    )
}

fn runtime_write_error(_error: open_web_codex_adapter::AdapterError) -> ApiError {
    (
        StatusCode::BAD_GATEWAY,
        Json(PlatformError::internal(
            "Codex Profile configuration update failed",
        )),
    )
}

fn database_error() -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("database operation failed")),
    )
}

fn io_error(_error: std::io::Error) -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(PlatformError::internal("Profile storage operation failed")),
    )
}
