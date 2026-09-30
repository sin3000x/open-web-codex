use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use open_web_codex_adapter::real::{RootExecutionConfig, ThreadSkillConfig};
use open_web_codex_profile_host::{CodexFeature, ProfileStartupFile};
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use toml_edit::{value, Array, DocumentMut, Item, Table};

use crate::delivery_contracts::{
    ContentVerifier, DeliveryContract, DeliveryKind, DeliveryRegistry,
};

const ROLE_MCP_SERVER_POLICY_KEYS: [&str; 5] = [
    "enabled",
    "required",
    "omit_tools_from",
    "default_tools_approval_mode",
    "tools",
];
const ALLOWED_HOST_ENVIRONMENT_NAMES: [&str; 8] = [
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];
const MAX_PREPARED_DESCRIPTOR_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DELIVERIES: usize = 32;
const MAX_DELIVERY_TEXT_BYTES: usize = 256;

/// Keep a local Copilot development Profile focused on the package selected
/// by the operator. Codex still owns discovery and prompt construction; these
/// are official feature switches, not a local tool/plugin filter.
pub(crate) const fn disabled_codex_features() -> [CodexFeature; 4] {
    [
        CodexFeature::Plugins,
        CodexFeature::RemotePlugin,
        CodexFeature::Apps,
        CodexFeature::ToolSuggest,
    ]
}

/// Use Codex's native structured user-input lifecycle while the selected
/// supervisor is executing. Platform and Web only persist, authorize, and
/// render the official request; they do not infer questions from assistant text.
pub(crate) const fn enabled_codex_features() -> [CodexFeature; 1] {
    [CodexFeature::DefaultModeRequestUserInput]
}

#[derive(Clone, Debug)]
pub(crate) struct CopilotPackageAssets {
    id: String,
    display_name: String,
    source_revision: String,
    root_skill: String,
    root_task_skill_access: RootTaskSkillAccess,
    root_agent: Option<String>,
    skills: Vec<PackageSkill>,
    roles: Vec<PackageRole>,
    capability_roots: BTreeMap<String, PreparedCapabilityRoot>,
    deliveries: DeliveryRegistry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootTaskSkillAccess {
    None,
    All,
}

#[derive(Clone, Debug)]
struct PackageSkill {
    id: String,
    contents: Vec<u8>,
}

#[derive(Clone, Debug)]
struct PackageRole {
    id: String,
    template: String,
}

#[derive(Debug, Clone)]
struct McpTransport {
    command: PathBuf,
    args: Vec<String>,
    startup_timeout_sec: Option<i64>,
    tool_timeout_sec: Option<i64>,
    env_bindings: Vec<PreparedEnvironmentBinding>,
}

#[derive(Debug, Clone)]
struct PreparedCapabilityRoot {
    servers: BTreeMap<String, McpTransport>,
}

#[derive(Debug, Clone)]
enum PreparedEnvironmentBinding {
    ProfileHome { name: String },
    ToolStateRoot { name: String },
    DependencyRoot { name: String, root: PathBuf },
    Host { name: String },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedDescriptorDocument {
    schema_version: u32,
    composition_descriptor_sha256: String,
    capability_roots: Vec<PreparedDescriptorCapabilityRoot>,
    deliveries: Vec<PreparedDescriptorDelivery>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedDescriptorDelivery {
    id: String,
    server: String,
    tool: String,
    kind: String,
    schema: String,
    mime_type: String,
    display_name: String,
    content_verifier: Option<PreparedContentVerifier>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedContentVerifier {
    kind: String,
    value: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedDescriptorCapabilityRoot {
    id: String,
    servers: Vec<PreparedDescriptorServer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedDescriptorServer {
    id: String,
    transport: String,
    command: PathBuf,
    args: Vec<String>,
    env_bindings: Vec<PreparedDescriptorEnvironmentBinding>,
    startup_timeout_sec: Option<i64>,
    tool_timeout_sec: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreparedDescriptorEnvironmentBinding {
    name: String,
    source: String,
    dependency: Option<String>,
    resolved_root: Option<PathBuf>,
}

impl CopilotPackageAssets {
    #[cfg(test)]
    pub(crate) fn lifecycle_test_assets(
        id: &str,
        source_revision: &str,
        deliveries: DeliveryRegistry,
    ) -> Self {
        Self {
            id: id.to_string(),
            display_name: "Lifecycle test Copilot".to_string(),
            source_revision: source_revision.to_string(),
            root_skill: "lifecycle-root".to_string(),
            root_task_skill_access: RootTaskSkillAccess::None,
            root_agent: None,
            skills: vec![PackageSkill {
                id: "lifecycle-root".to_string(),
                contents: b"---\nname: lifecycle-root\ndescription: lifecycle test\n---\n".to_vec(),
            }],
            roles: Vec::new(),
            capability_roots: BTreeMap::new(),
            deliveries,
        }
    }

    pub(crate) fn manifest_id(package_root: &Path) -> Result<String, CopilotPackageError> {
        Ok(load_copilot_package(package_root)?.id)
    }

    pub(crate) fn resolve(
        package_root: &Path,
        descriptor_path: &Path,
        build_store_root: &Path,
    ) -> Result<Self, CopilotPackageError> {
        let package = load_copilot_package(package_root)?;
        let (source_revision, capability_roots, deliveries) =
            load_prepared_descriptor(descriptor_path, build_store_root)?;
        let prepared_ids = capability_roots.keys().cloned().collect::<BTreeSet<_>>();
        if prepared_ids != package.tool_ids {
            return Err(unavailable_message(
                "prepared Copilot descriptor",
                format!(
                    "declared Tool ids {:?} do not match prepared capability roots {:?}",
                    package.tool_ids, prepared_ids
                ),
            ));
        }
        Ok(Self {
            id: package.id,
            display_name: package.display_name,
            source_revision,
            root_skill: package.root_skill,
            root_task_skill_access: package.root_task_skill_access,
            root_agent: package.root_agent,
            skills: package.skills,
            roles: package.roles,
            capability_roots,
            deliveries,
        })
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn source_revision(&self) -> &str {
        &self.source_revision
    }

    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }

    pub(crate) fn skill_ids(&self) -> Vec<String> {
        self.skills.iter().map(|skill| skill.id.clone()).collect()
    }

    pub(crate) fn agent_role_ids(&self) -> Vec<String> {
        self.roles
            .iter()
            .filter(|role| Some(role.id.as_str()) != self.root_agent.as_deref())
            .map(|role| role.id.clone())
            .collect()
    }

    pub(crate) fn deliveries(&self) -> DeliveryRegistry {
        self.deliveries.clone()
    }

    pub(crate) fn root_skill_config(&self, profile_home: &Path) -> Vec<ThreadSkillConfig> {
        self.skills
            .iter()
            .map(|skill| ThreadSkillConfig {
                name: skill.id.clone(),
                enabled: skill.id == self.root_skill
                    || self.root_task_skill_access == RootTaskSkillAccess::All,
                main_prompt: (skill.id == self.root_skill)
                    .then(|| profile_home.join("skills").join(&skill.id).join("SKILL.md")),
            })
            .collect()
    }

    pub(crate) fn root_execution_config(
        &self,
        profile_home: &Path,
    ) -> Result<RootExecutionConfig, CopilotPackageError> {
        let mut runtime_config = if let Some(agent) = self.root_agent.as_deref() {
            let rendered = self.render_role(agent, profile_home)?;
            let mut role = parse_role_template(agent, &rendered)?;
            role.remove("name");
            role.remove("description");
            role.remove("nickname_candidates");
            let config = toml_edit::de::from_str::<serde_json::Value>(&role.to_string()).map_err(
                |error| {
                    invalid_role(
                        agent,
                        format!("Root Agent config could not be encoded: {error}"),
                    )
                },
            )?;
            Self::flatten_runtime_config(config).map_err(|message| invalid_role(agent, message))?
        } else {
            serde_json::json!({})
        };
        let runtime_config_object = runtime_config.as_object_mut().ok_or_else(|| {
            invalid_role(
                self.root_agent.as_deref().unwrap_or("root"),
                "Root Agent config must be an object",
            )
        })?;
        for role_id in self.agent_role_ids() {
            let role_config_file = profile_home.join("agents").join(format!("{role_id}.toml"));
            runtime_config_object.insert(
                format!("agents.{role_id}.config_file"),
                Value::String(role_config_file.to_string_lossy().into_owned()),
            );
            runtime_config_object.insert(
                format!("agents.{role_id}.runtime_mcp_projection"),
                Value::Bool(true),
            );
        }
        Ok(RootExecutionConfig {
            id: self.id.clone(),
            skill_config: self.root_skill_config(profile_home),
            runtime_config,
        })
    }

    fn flatten_runtime_config(value: Value) -> Result<Value, String> {
        fn visit(
            prefix: Option<&str>,
            value: Value,
            output: &mut serde_json::Map<String, Value>,
        ) -> Result<(), String> {
            match value {
                Value::Object(object) => {
                    for (key, child) in object {
                        let path = match prefix {
                            Some(prefix) => format!("{prefix}.{key}"),
                            None => key,
                        };
                        visit(Some(&path), child, output)?;
                    }
                    Ok(())
                }
                leaf => {
                    let Some(path) = prefix else {
                        return Err("Root Agent config must be a TOML table".to_string());
                    };
                    if output.insert(path.to_string(), leaf).is_some() {
                        return Err(format!("Root Agent config contains duplicate path {path}"));
                    }
                    Ok(())
                }
            }
        }

        let mut output = serde_json::Map::new();
        visit(None, value, &mut output)?;
        Ok(Value::Object(output))
    }

    pub(crate) fn startup_files(
        &self,
        profile_home: &Path,
    ) -> Result<Vec<ProfileStartupFile>, CopilotPackageError> {
        let profile_home = require_absolute_path("Profile CODEX_HOME", profile_home)?;
        let mut files = Vec::with_capacity(self.skills.len() + self.roles.len());
        for skill in &self.skills {
            files.push(ProfileStartupFile::package_skill(
                skill.id.clone(),
                skill.contents.clone(),
            )?);
        }
        for source in &self.roles {
            if Some(source.id.as_str()) == self.root_agent.as_deref() {
                continue;
            }
            files.push(ProfileStartupFile::package_agent_role(
                source.id.clone(),
                self.render_role(&source.id, &profile_home)?.into_bytes(),
            )?);
        }
        Ok(files)
    }

    fn render_role(
        &self,
        role_id: &str,
        profile_home: &Path,
    ) -> Result<String, CopilotPackageError> {
        let source = self
            .roles
            .iter()
            .find(|source| source.id == role_id)
            .ok_or_else(|| invalid_role(role_id, "Role is not declared by the Copilot package"))?;
        let mut role = parse_role_template(&source.id, &source.template)?;
        project_role_mcp_servers(&mut role, &source.id, &self.capability_roots, profile_home)?;
        Ok(finish_role_template(role))
    }
}

#[derive(Debug, Error)]
pub(crate) enum CopilotPackageError {
    #[error("configured Copilot package is unavailable: {component}: {message}")]
    Unavailable {
        component: &'static str,
        message: String,
    },
    #[error("invalid configured Copilot package {role} Role template: {message}")]
    InvalidRoleTemplate { role: String, message: String },
    #[error(transparent)]
    StartupFile(#[from] open_web_codex_profile_host::ProfileStartupFileError),
}

fn require_absolute_path(
    component: &'static str,
    path: &Path,
) -> Result<PathBuf, CopilotPackageError> {
    if !path.is_absolute() {
        return Err(unavailable_message(
            component,
            "configured path must be absolute".to_string(),
        ));
    }
    Ok(path.to_path_buf())
}

fn unavailable(component: &'static str, error: impl std::fmt::Display) -> CopilotPackageError {
    unavailable_message(component, error.to_string())
}

fn unavailable_message(component: &'static str, message: String) -> CopilotPackageError {
    CopilotPackageError::Unavailable { component, message }
}

fn parse_role_template(role: &str, template: &str) -> Result<DocumentMut, CopilotPackageError> {
    template
        .parse::<DocumentMut>()
        .map_err(|error| CopilotPackageError::InvalidRoleTemplate {
            role: role.to_string(),
            message: error.to_string(),
        })
}

struct LoadedCopilotPackage {
    id: String,
    display_name: String,
    root_skill: String,
    root_task_skill_access: RootTaskSkillAccess,
    root_agent: Option<String>,
    skills: Vec<PackageSkill>,
    roles: Vec<PackageRole>,
    tool_ids: BTreeSet<String>,
}

fn load_copilot_package(package_root: &Path) -> Result<LoadedCopilotPackage, CopilotPackageError> {
    const COMPONENT: &str = "Copilot package";
    let root_metadata =
        fs::symlink_metadata(package_root).map_err(|error| unavailable(COMPONENT, error))?;
    if root_metadata.file_type().is_symlink() {
        return Err(unavailable_message(
            COMPONENT,
            "package root must not be a symlink".to_string(),
        ));
    }
    let package_root = canonical_directory(COMPONENT, package_root)?;
    let manifest_path = package_regular_file(&package_root, Path::new("copilot.toml"), COMPONENT)?;
    let raw = fs::read_to_string(manifest_path).map_err(|error| unavailable(COMPONENT, error))?;
    let manifest = raw
        .parse::<DocumentMut>()
        .map_err(|error| unavailable(COMPONENT, error))?;
    reject_unknown_keys(
        COMPONENT,
        "copilot.toml",
        manifest.as_table(),
        &[
            "schema_version",
            "id",
            "display_name",
            "root",
            "skills",
            "agents",
            "tools",
            "tests",
            "deliveries",
        ],
    )?;
    if manifest.get("schema_version").and_then(Item::as_integer) != Some(1) {
        return Err(unavailable_message(
            COMPONENT,
            "schema_version must equal 1".to_string(),
        ));
    }
    let id = required_string(COMPONENT, manifest.as_table(), "id")?.to_string();
    require_stable_identifier(COMPONENT, "Copilot id", &id)?;
    let display_name = required_string(COMPONENT, manifest.as_table(), "display_name")?.to_string();
    let root_config = required_table(COMPONENT, manifest.as_table(), "root")?;
    reject_unknown_keys(
        COMPONENT,
        "root",
        root_config,
        &["skill", "agent", "task_skills"],
    )?;
    let root_skill = required_string(COMPONENT, root_config, "skill")?.to_string();
    let root_task_skill_access = match required_string(COMPONENT, root_config, "task_skills")? {
        "none" => RootTaskSkillAccess::None,
        "all" => RootTaskSkillAccess::All,
        _ => {
            return Err(unavailable_message(
                COMPONENT,
                "root.task_skills must equal `none` or `all`".to_string(),
            ));
        }
    };

    let skill_entries = manifest
        .get("skills")
        .and_then(Item::as_array_of_tables)
        .ok_or_else(|| {
            unavailable_message(COMPONENT, "skills must be an array of tables".into())
        })?;
    if skill_entries.is_empty() {
        return Err(unavailable_message(
            COMPONENT,
            "skills must not be empty".to_string(),
        ));
    }
    let mut skill_ids = BTreeSet::new();
    let mut skills = Vec::new();
    for entry in skill_entries {
        reject_unknown_keys(COMPONENT, "skills[]", entry, &["id", "path"])?;
        let skill_id = required_string(COMPONENT, entry, "id")?.to_string();
        require_stable_identifier(COMPONENT, "Skill id", &skill_id)?;
        if !skill_ids.insert(skill_id.clone()) {
            return Err(unavailable_message(
                COMPONENT,
                format!("duplicate Skill id {skill_id}"),
            ));
        }
        let relative = Path::new(required_string(COMPONENT, entry, "path")?).join("SKILL.md");
        let path = package_regular_file(&package_root, &relative, COMPONENT)?;
        let contents = fs::read(path).map_err(|error| unavailable(COMPONENT, error))?;
        skills.push(PackageSkill {
            id: skill_id,
            contents,
        });
    }
    if !skill_ids.contains(&root_skill) {
        return Err(unavailable_message(
            COMPONENT,
            format!("Root Skill {root_skill} is not declared"),
        ));
    }

    let agent_entries = manifest
        .get("agents")
        .and_then(Item::as_array_of_tables)
        .ok_or_else(|| {
            unavailable_message(COMPONENT, "agents must be an array of tables".into())
        })?;
    if agent_entries.is_empty() {
        return Err(unavailable_message(
            COMPONENT,
            "agents must not be empty".to_string(),
        ));
    }
    let mut role_ids = BTreeSet::new();
    let mut roles = Vec::new();
    for entry in agent_entries {
        reject_unknown_keys(COMPONENT, "agents[]", entry, &["id", "role"])?;
        let role_id = required_string(COMPONENT, entry, "id")?.to_string();
        require_stable_identifier(COMPONENT, "Agent Role id", &role_id)?;
        if !role_ids.insert(role_id.clone()) {
            return Err(unavailable_message(
                COMPONENT,
                format!("duplicate Agent Role id {role_id}"),
            ));
        }
        let relative = Path::new(required_string(COMPONENT, entry, "role")?);
        let path = package_regular_file(&package_root, relative, COMPONENT)?;
        let template = fs::read_to_string(path).map_err(|error| unavailable(COMPONENT, error))?;
        let parsed = parse_role_template(&role_id, &template)?;
        if parsed.get("name").and_then(Item::as_str) != Some(role_id.as_str()) {
            return Err(invalid_role(
                &role_id,
                "top-level name must equal the manifest Agent id",
            ));
        }
        roles.push(PackageRole {
            id: role_id,
            template,
        });
    }

    let root_agent = root_config
        .get("agent")
        .map(|item| {
            item.as_str().map(str::to_string).ok_or_else(|| {
                unavailable_message(COMPONENT, "root.agent must be a string".to_string())
            })
        })
        .transpose()?;
    if let Some(agent) = root_agent.as_deref() {
        if !role_ids.contains(agent) {
            return Err(unavailable_message(
                COMPONENT,
                format!("Root references undeclared Agent Role {agent}"),
            ));
        }
    }

    let tool_entries = manifest
        .get("tools")
        .and_then(Item::as_array_of_tables)
        .ok_or_else(|| unavailable_message(COMPONENT, "tools must be an array of tables".into()))?;
    if tool_entries.is_empty() {
        return Err(unavailable_message(
            COMPONENT,
            "tools must not be empty".to_string(),
        ));
    }
    let mut tool_ids = BTreeSet::new();
    for entry in tool_entries {
        reject_unknown_keys(
            COMPONENT,
            "tools[]",
            entry,
            &["id", "root", "runtime", "package"],
        )?;
        let tool_id = required_string(COMPONENT, entry, "id")?.to_string();
        require_stable_identifier(COMPONENT, "Tool id", &tool_id)?;
        if !tool_ids.insert(tool_id.clone()) {
            return Err(unavailable_message(
                COMPONENT,
                format!("duplicate Tool id {tool_id}"),
            ));
        }
        match (
            entry.get("root"),
            entry.get("runtime"),
            entry.get("package"),
        ) {
            (Some(_), Some(_), None) => {
                let root = package_directory(
                    &package_root,
                    Path::new(required_string(COMPONENT, entry, "root")?),
                    COMPONENT,
                )?;
                let runtime = Path::new(required_string(COMPONENT, entry, "runtime")?);
                let runtime_file = package_regular_file(&package_root, runtime, COMPONENT)?;
                if !runtime_file.starts_with(&root) {
                    return Err(unavailable_message(
                        COMPONENT,
                        format!("Tool {tool_id} runtime must be inside its Tool root"),
                    ));
                }
            }
            (None, None, Some(_)) => {
                require_stable_identifier(
                    COMPONENT,
                    "shared Tool package id",
                    required_string(COMPONENT, entry, "package")?,
                )?;
            }
            _ => {
                return Err(unavailable_message(
                    COMPONENT,
                    format!("Tool {tool_id} must declare either root/runtime or package"),
                ));
            }
        }
    }

    Ok(LoadedCopilotPackage {
        id,
        display_name,
        root_skill,
        root_task_skill_access,
        root_agent,
        skills,
        roles,
        tool_ids,
    })
}

fn required_string<'a>(
    component: &'static str,
    table: &'a Table,
    key: &str,
) -> Result<&'a str, CopilotPackageError> {
    table
        .get(key)
        .and_then(Item::as_str)
        .ok_or_else(|| unavailable_message(component, format!("{key} must be a string")))
}

fn required_table<'a>(
    component: &'static str,
    table: &'a Table,
    key: &str,
) -> Result<&'a Table, CopilotPackageError> {
    table
        .get(key)
        .and_then(Item::as_table)
        .ok_or_else(|| unavailable_message(component, format!("{key} must be a table")))
}

fn reject_unknown_keys(
    component: &'static str,
    owner: &str,
    table: &Table,
    allowed: &[&str],
) -> Result<(), CopilotPackageError> {
    for (key, _) in table {
        if !allowed.contains(&key) {
            return Err(unavailable_message(
                component,
                format!("{owner}.{key} is not part of the current contract"),
            ));
        }
    }
    Ok(())
}

fn safe_package_relative_path(
    component: &'static str,
    relative: &Path,
) -> Result<(), CopilotPackageError> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(unavailable_message(
            component,
            format!("unsafe package-relative path {}", relative.display()),
        ));
    }
    Ok(())
}

fn package_path(
    root: &Path,
    relative: &Path,
    component: &'static str,
) -> Result<PathBuf, CopilotPackageError> {
    safe_package_relative_path(component, relative)?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        let Component::Normal(part) = part else {
            unreachable!("validated package path component")
        };
        current.push(part);
        let metadata =
            fs::symlink_metadata(&current).map_err(|error| unavailable(component, error))?;
        if metadata.file_type().is_symlink() {
            return Err(unavailable_message(
                component,
                format!(
                    "package path must not contain a symlink: {}",
                    relative.display()
                ),
            ));
        }
    }
    Ok(current)
}

fn package_regular_file(
    root: &Path,
    relative: &Path,
    component: &'static str,
) -> Result<PathBuf, CopilotPackageError> {
    let path = package_path(root, relative, component)?;
    if !path.is_file() {
        return Err(unavailable_message(
            component,
            format!("expected a regular file at {}", relative.display()),
        ));
    }
    Ok(path)
}

fn package_directory(
    root: &Path,
    relative: &Path,
    component: &'static str,
) -> Result<PathBuf, CopilotPackageError> {
    let path = package_path(root, relative, component)?;
    if !path.is_dir() {
        return Err(unavailable_message(
            component,
            format!("expected a directory at {}", relative.display()),
        ));
    }
    Ok(path)
}

fn load_prepared_descriptor(
    descriptor_path: &Path,
    build_store_root: &Path,
) -> Result<
    (
        String,
        BTreeMap<String, PreparedCapabilityRoot>,
        DeliveryRegistry,
    ),
    CopilotPackageError,
> {
    const COMPONENT: &str = "prepared Copilot descriptor";
    let descriptor_path = canonical_regular_file(COMPONENT, descriptor_path, false)?;
    let build_store_root = canonical_directory(COMPONENT, build_store_root)?;
    if fs::metadata(&descriptor_path)
        .map_err(|error| unavailable(COMPONENT, error))?
        .len()
        > MAX_PREPARED_DESCRIPTOR_BYTES
    {
        return Err(unavailable_message(
            COMPONENT,
            "descriptor exceeds 8 MiB".into(),
        ));
    }
    let raw =
        fs::read_to_string(&descriptor_path).map_err(|error| unavailable(COMPONENT, error))?;
    let document: PreparedDescriptorDocument =
        serde_json::from_str(&raw).map_err(|error| unavailable(COMPONENT, error))?;
    if document.schema_version != 1 {
        return Err(unavailable_message(
            COMPONENT,
            format!("unsupported schemaVersion {}", document.schema_version),
        ));
    }
    if document.composition_descriptor_sha256.len() != 64
        || !document
            .composition_descriptor_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(unavailable_message(
            COMPONENT,
            "compositionDescriptorSha256 must be a lowercase SHA-256 digest".to_string(),
        ));
    }
    let source_revision = document.composition_descriptor_sha256.clone();
    if document.capability_roots.is_empty() {
        return Err(unavailable_message(
            COMPONENT,
            "capabilityRoots must not be empty".to_string(),
        ));
    }

    if document.deliveries.len() > MAX_DELIVERIES {
        return Err(unavailable_message(
            COMPONENT,
            "deliveries exceed 32 entries".into(),
        ));
    }
    let mut capability_roots = BTreeMap::new();
    let mut all_server_ids = BTreeSet::new();
    let mut build_fingerprint: Option<String> = None;
    for root in document.capability_roots {
        require_stable_identifier(COMPONENT, "capability root", &root.id)?;
        if root.servers.is_empty() {
            return Err(unavailable_message(
                COMPONENT,
                format!("capability root {} declares no servers", root.id),
            ));
        }
        let mut servers = BTreeMap::new();
        for server in root.servers {
            require_stable_identifier(COMPONENT, "MCP server", &server.id)?;
            if !all_server_ids.insert(server.id.clone()) {
                return Err(unavailable_message(
                    COMPONENT,
                    format!(
                        "MCP server {} is owned by more than one capability root",
                        server.id
                    ),
                ));
            }
            if server.transport != "stdio" {
                return Err(unavailable_message(
                    COMPONENT,
                    format!(
                        "capability root {} server {} transport must be stdio",
                        root.id, server.id
                    ),
                ));
            }
            let command = prepared_command(
                COMPONENT,
                &server.command,
                &build_store_root,
                &mut build_fingerprint,
            )?;
            let startup_timeout_sec = positive_timeout(
                COMPONENT,
                &root.id,
                &server.id,
                "startupTimeoutSec",
                server.startup_timeout_sec,
            )?;
            let tool_timeout_sec = positive_timeout(
                COMPONENT,
                &root.id,
                &server.id,
                "toolTimeoutSec",
                server.tool_timeout_sec,
            )?;
            let mut environment_names = BTreeMap::new();
            let mut env_bindings = Vec::new();
            for binding in server.env_bindings {
                require_environment_name(COMPONENT, &binding.name)?;
                if environment_names.insert(binding.name.clone(), ()).is_some() {
                    return Err(unavailable_message(
                        COMPONENT,
                        format!(
                            "capability root {} server {} duplicates environment binding {}",
                            root.id, server.id, binding.name
                        ),
                    ));
                }
                let binding = match binding.source.as_str() {
                    "profile_home" => {
                        reject_binding_metadata(COMPONENT, &root.id, &server.id, &binding)?;
                        PreparedEnvironmentBinding::ProfileHome { name: binding.name }
                    }
                    "tool_state_root" => {
                        reject_binding_metadata(COMPONENT, &root.id, &server.id, &binding)?;
                        PreparedEnvironmentBinding::ToolStateRoot { name: binding.name }
                    }
                    "dependency_root" => {
                        let dependency = binding.dependency.as_deref().ok_or_else(|| {
                            unavailable_message(
                                COMPONENT,
                                format!(
                                    "capability root {} server {} dependency_root binding {} omits dependency",
                                    root.id, server.id, binding.name
                                ),
                            )
                        })?;
                        require_stable_identifier(COMPONENT, "dependency", dependency)?;
                        let resolved_root = binding.resolved_root.as_deref().ok_or_else(|| {
                            unavailable_message(
                                COMPONENT,
                                format!(
                                    "capability root {} server {} dependency_root binding {} omits resolvedRoot",
                                    root.id, server.id, binding.name
                                ),
                            )
                        })?;
                        PreparedEnvironmentBinding::DependencyRoot {
                            name: binding.name,
                            root: prepared_dependency_root(
                                COMPONENT,
                                resolved_root,
                                &build_store_root,
                                &mut build_fingerprint,
                            )?,
                        }
                    }
                    "host" => {
                        reject_binding_metadata(COMPONENT, &root.id, &server.id, &binding)?;
                        if !ALLOWED_HOST_ENVIRONMENT_NAMES.contains(&binding.name.as_str()) {
                            return Err(unavailable_message(
                                COMPONENT,
                                format!(
                                    "capability root {} server {} host binding {} is not an allowed proxy environment variable",
                                    root.id, server.id, binding.name
                                ),
                            ));
                        }
                        PreparedEnvironmentBinding::Host { name: binding.name }
                    }
                    source => {
                        return Err(unavailable_message(
                            COMPONENT,
                            format!(
                                "capability root {} server {} binding {} has unsupported source {source}",
                                root.id, server.id, binding.name
                            ),
                        ));
                    }
                };
                env_bindings.push(binding);
            }
            let server_id = server.id;
            if servers
                .insert(
                    server_id.clone(),
                    McpTransport {
                        command,
                        args: server.args,
                        startup_timeout_sec,
                        tool_timeout_sec,
                        env_bindings,
                    },
                )
                .is_some()
            {
                return Err(unavailable_message(
                    COMPONENT,
                    format!(
                        "capability root {} duplicates MCP server {server_id}",
                        root.id
                    ),
                ));
            }
        }
        let root_id = root.id;
        if capability_roots
            .insert(root_id.clone(), PreparedCapabilityRoot { servers })
            .is_some()
        {
            return Err(unavailable_message(
                COMPONENT,
                format!("duplicate capability root {root_id}"),
            ));
        }
    }
    let mut deliveries = Vec::new();
    for delivery in document.deliveries {
        for (field, value) in [
            ("id", delivery.id.as_str()),
            ("server", delivery.server.as_str()),
            ("tool", delivery.tool.as_str()),
            ("schema", delivery.schema.as_str()),
            ("mimeType", delivery.mime_type.as_str()),
            ("displayName", delivery.display_name.as_str()),
        ] {
            if value.is_empty() || value.len() > MAX_DELIVERY_TEXT_BYTES {
                return Err(unavailable_message(
                    COMPONENT,
                    format!("delivery {field} is empty or exceeds 256 bytes"),
                ));
            }
        }
        require_stable_identifier(COMPONENT, "delivery", &delivery.id)?;
        require_stable_identifier(COMPONENT, "delivery MCP server", &delivery.server)?;
        require_stable_identifier(COMPONENT, "delivery MCP tool", &delivery.tool)?;
        if !capability_roots
            .values()
            .any(|root| root.servers.contains_key(&delivery.server))
        {
            return Err(unavailable_message(
                COMPONENT,
                format!(
                    "delivery {} references undeclared MCP server {}",
                    delivery.id, delivery.server
                ),
            ));
        }
        let kind = match delivery.kind.as_str() {
            "workspace_artifact" => {
                let verifier = delivery.content_verifier.ok_or_else(|| {
                    unavailable_message(
                        COMPONENT,
                        format!("delivery {} omits contentVerifier", delivery.id),
                    )
                })?;
                let verifier = match verifier.kind.as_str() {
                    "json_schema" => ContentVerifier::JsonSchema {
                        document: {
                            let mut nodes = 0;
                            validate_bounded_descriptor_json(&verifier.value, 0, &mut nodes)?;
                            verifier.value
                        },
                    },
                    "markdown_marker" => ContentVerifier::MarkdownMarker {
                        marker: verifier
                            .value
                            .as_str()
                            .ok_or_else(|| {
                                unavailable_message(
                                    COMPONENT,
                                    format!(
                                        "delivery {} Markdown marker must be a string",
                                        delivery.id
                                    ),
                                )
                            })?
                            .to_string(),
                    },
                    other => {
                        return Err(unavailable_message(
                            COMPONENT,
                            format!("delivery {} has unsupported verifier {other}", delivery.id),
                        ))
                    }
                };
                DeliveryKind::WorkspaceArtifact { verifier }
            }
            "inline_geojson_map_card" => {
                if delivery.content_verifier.is_some() {
                    return Err(unavailable_message(
                        COMPONENT,
                        format!(
                            "delivery {} inline map card must not declare contentVerifier",
                            delivery.id
                        ),
                    ));
                }
                DeliveryKind::InlineGeoJsonMapCard
            }
            other => {
                return Err(unavailable_message(
                    COMPONENT,
                    format!("delivery {} has unsupported kind {other}", delivery.id),
                ))
            }
        };
        deliveries.push(DeliveryContract {
            id: delivery.id,
            server: delivery.server,
            tool: delivery.tool,
            kind,
            schema: delivery.schema,
            mime_type: delivery.mime_type,
            display_name: delivery.display_name,
        });
    }
    let deliveries = DeliveryRegistry::new(deliveries)
        .and_then(|registry| {
            registry.validate()?;
            Ok(registry)
        })
        .map_err(|error| unavailable_message(COMPONENT, error))?;
    Ok((source_revision, capability_roots, deliveries))
}

fn validate_bounded_descriptor_json(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
) -> Result<(), CopilotPackageError> {
    const COMPONENT: &str = "prepared Copilot descriptor";
    *nodes = nodes.saturating_add(1);
    if depth > 32 || *nodes > 20_000 {
        return Err(unavailable_message(
            COMPONENT,
            "delivery verifier exceeds structural limits".into(),
        ));
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key.len() > 2_000 {
                    return Err(unavailable_message(
                        COMPONENT,
                        "delivery verifier key is oversized".into(),
                    ));
                }
                validate_bounded_descriptor_json(child, depth + 1, nodes)?;
            }
        }
        Value::Array(values) => {
            for child in values {
                validate_bounded_descriptor_json(child, depth + 1, nodes)?;
            }
        }
        Value::String(text) if text.len() > 64 * 1024 => {
            return Err(unavailable_message(
                COMPONENT,
                "delivery verifier string is oversized".into(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn canonical_regular_file(
    component: &'static str,
    path: &Path,
    executable: bool,
) -> Result<PathBuf, CopilotPackageError> {
    let path = require_absolute_path(component, path)?;
    let path = path
        .canonicalize()
        .map_err(|error| unavailable(component, error))?;
    let metadata = fs::metadata(&path).map_err(|error| unavailable(component, error))?;
    if !metadata.is_file() || (executable && !is_executable(&metadata)) {
        let expected = if executable {
            "an executable regular file"
        } else {
            "a regular file"
        };
        return Err(unavailable_message(
            component,
            format!("expected {expected} at {}", path.display()),
        ));
    }
    Ok(path)
}

/// Validate a launcher emitted by the trusted shared build store without
/// replacing it with its resolved executable target. Python virtual
/// environments intentionally use a final `bin/python` symlink; launching the
/// resolved system interpreter would discard the virtual-environment prefix
/// and its installed dependencies.
fn prepared_command(
    component: &'static str,
    path: &Path,
    build_store_root: &Path,
    build_fingerprint: &mut Option<String>,
) -> Result<PathBuf, CopilotPackageError> {
    let path = require_absolute_path(component, path)?;
    let build_store_root = build_store_root
        .canonicalize()
        .map_err(|error| unavailable(component, error))?;
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err(unavailable_message(
            component,
            format!(
                "prepared MCP command must be lexically normalized: {}",
                path.display()
            ),
        ));
    }
    let (build_root, fingerprint) = shared_build_root(component, &path, &build_store_root)?;
    require_matching_build_fingerprint(component, build_fingerprint, &fingerprint)?;
    let parent = path.parent().ok_or_else(|| {
        unavailable_message(
            component,
            format!("prepared MCP command has no parent: {}", path.display()),
        )
    })?;
    let parent = parent
        .canonicalize()
        .map_err(|error| unavailable(component, error))?;
    if !parent.starts_with(&build_root) {
        return Err(unavailable_message(
            component,
            "prepared command escapes shared build".into(),
        ));
    }
    let entry = fs::symlink_metadata(&path).map_err(|error| unavailable(component, error))?;
    if !entry.is_file() && !entry.file_type().is_symlink() {
        return Err(unavailable_message(
            component,
            format!(
                "expected an executable regular file or launcher symlink at {}",
                path.display()
            ),
        ));
    }
    let target = fs::metadata(&path).map_err(|error| unavailable(component, error))?;
    if !target.is_file() || !is_executable(&target) {
        return Err(unavailable_message(
            component,
            format!(
                "expected launcher target to be an executable regular file at {}",
                path.display()
            ),
        ));
    }
    let target_path = path
        .canonicalize()
        .map_err(|error| unavailable(component, error))?;
    if !target_path.starts_with(&build_root) {
        return Err(unavailable_message(
            component,
            "prepared command symlink escapes shared build".into(),
        ));
    }
    Ok(path)
}

fn prepared_dependency_root(
    component: &'static str,
    path: &Path,
    build_store_root: &Path,
    build_fingerprint: &mut Option<String>,
) -> Result<PathBuf, CopilotPackageError> {
    let path = require_absolute_path(component, path)?;
    let (build_root, fingerprint) = shared_build_root(component, &path, build_store_root)?;
    require_matching_build_fingerprint(component, build_fingerprint, &fingerprint)?;
    let canonical = path
        .canonicalize()
        .map_err(|error| unavailable(component, error))?;
    if !canonical.starts_with(&build_root) || !canonical.is_dir() {
        return Err(unavailable_message(
            component,
            "dependency root escapes shared build".into(),
        ));
    }
    Ok(canonical)
}

fn shared_build_root(
    component: &'static str,
    path: &Path,
    build_store_root: &Path,
) -> Result<(PathBuf, String), CopilotPackageError> {
    let builds_root = build_store_root.join("builds");
    let canonical_path = path
        .canonicalize()
        .map_err(|error| unavailable(component, error))?;
    let relative = canonical_path.strip_prefix(&builds_root).map_err(|_| {
        unavailable_message(
            component,
            "prepared path is outside the shared build store".into(),
        )
    })?;
    let mut components = relative.components();
    let fingerprint = components
        .next()
        .and_then(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .ok_or_else(|| {
            unavailable_message(component, "prepared path has no build fingerprint".into())
        })?;
    if fingerprint.len() != 64
        || !fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(unavailable_message(
            component,
            "build fingerprint is invalid".into(),
        ));
    }
    let build_root = builds_root.join(fingerprint);
    let metadata =
        fs::symlink_metadata(&build_root).map_err(|error| unavailable(component, error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unavailable_message(
            component,
            "build fingerprint root is unsafe".into(),
        ));
    }
    let canonical = build_root
        .canonicalize()
        .map_err(|error| unavailable(component, error))?;
    if canonical != build_root {
        return Err(unavailable_message(
            component,
            "build fingerprint root is not canonical".into(),
        ));
    }
    let marker_path = canonical.join(".copilot-tool-build.v1.json");
    let marker_metadata =
        fs::symlink_metadata(&marker_path).map_err(|error| unavailable(component, error))?;
    if marker_metadata.file_type().is_symlink() || !marker_metadata.is_file() {
        return Err(unavailable_message(
            component,
            "shared build marker is unsafe".into(),
        ));
    }
    let marker: serde_json::Value = serde_json::from_slice(
        &fs::read(&marker_path).map_err(|error| unavailable(component, error))?,
    )
    .map_err(|error| unavailable(component, error))?;
    if marker
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        != Some(1)
        || marker
            .get("preparationFingerprint")
            .and_then(serde_json::Value::as_str)
            != Some(fingerprint)
    {
        return Err(unavailable_message(
            component,
            "shared build marker does not match fingerprint".into(),
        ));
    }
    Ok((canonical, fingerprint.to_string()))
}

fn require_matching_build_fingerprint(
    component: &'static str,
    current: &mut Option<String>,
    fingerprint: &str,
) -> Result<(), CopilotPackageError> {
    if let Some(existing) = current {
        if existing != fingerprint {
            return Err(unavailable_message(
                component,
                "descriptor mixes shared build fingerprints".into(),
            ));
        }
    } else {
        *current = Some(fingerprint.to_string());
    }
    Ok(())
}

fn canonical_directory(
    component: &'static str,
    path: &Path,
) -> Result<PathBuf, CopilotPackageError> {
    let path = require_absolute_path(component, path)?;
    let path = path
        .canonicalize()
        .map_err(|error| unavailable(component, error))?;
    if !path.is_dir() {
        return Err(unavailable_message(
            component,
            format!("expected a directory at {}", path.display()),
        ));
    }
    Ok(path)
}

fn require_stable_identifier(
    component: &'static str,
    field: &str,
    value: &str,
) -> Result<(), CopilotPackageError> {
    let mut characters = value.chars();
    let valid = matches!(characters.next(), Some(first) if first.is_ascii_alphanumeric())
        && value.len() <= 128
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-')
        });
    if !valid {
        return Err(unavailable_message(
            component,
            format!("{field} {value:?} is not a stable identifier"),
        ));
    }
    Ok(())
}

fn require_environment_name(
    component: &'static str,
    value: &str,
) -> Result<(), CopilotPackageError> {
    let mut characters = value.chars();
    let valid = matches!(characters.next(), Some(first) if first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric());
    if !valid {
        return Err(unavailable_message(
            component,
            format!("invalid environment variable name {value:?}"),
        ));
    }
    Ok(())
}

fn reject_binding_metadata(
    component: &'static str,
    root_id: &str,
    server_id: &str,
    binding: &PreparedDescriptorEnvironmentBinding,
) -> Result<(), CopilotPackageError> {
    if binding.dependency.is_some() || binding.resolved_root.is_some() {
        return Err(unavailable_message(
            component,
            format!(
                "capability root {root_id} server {server_id} binding {} permits dependency and resolvedRoot only for dependency_root",
                binding.name
            ),
        ));
    }
    Ok(())
}

fn positive_timeout(
    component: &'static str,
    root_id: &str,
    server_id: &str,
    field: &str,
    value: Option<i64>,
) -> Result<Option<i64>, CopilotPackageError> {
    if matches!(value, Some(value) if value <= 0) {
        return Err(unavailable_message(
            component,
            format!("capability root {root_id} server {server_id} {field} must be positive"),
        ));
    }
    Ok(value)
}

fn project_role_mcp_servers(
    role: &mut DocumentMut,
    role_name: &str,
    capability_roots: &BTreeMap<String, PreparedCapabilityRoot>,
    profile_home: &Path,
) -> Result<(), CopilotPackageError> {
    let plugins = role
        .get("plugins")
        .and_then(Item::as_table)
        .ok_or_else(|| invalid_role(role_name, "Role omits plugins policy"))?;
    let mut runtime_servers = Table::new();
    for (capability_root_id, plugin) in plugins {
        let plugin = plugin.as_table().ok_or_else(|| {
            invalid_role(
                role_name,
                format!("plugins.{capability_root_id} is not a table"),
            )
        })?;
        let policies = plugin
            .get("mcp_servers")
            .and_then(Item::as_table)
            .ok_or_else(|| {
                invalid_role(
                    role_name,
                    format!("plugins.{capability_root_id} omits mcp_servers"),
                )
            })?;
        let declared = capability_roots.get(capability_root_id).ok_or_else(|| {
            invalid_role(
                role_name,
                format!("plugins.{capability_root_id} has no prepared capability root"),
            )
        })?;
        for (server_name, policy) in policies {
            if runtime_servers.contains_key(server_name) {
                return Err(invalid_role(
                    role_name,
                    format!("MCP server {server_name} has multiple transports"),
                ));
            }
            let transport = declared.servers.get(server_name).ok_or_else(|| {
                invalid_role(
                    role_name,
                    format!(
                        "plugins.{capability_root_id} references unprepared server {server_name}"
                    ),
                )
            })?;
            let policy = policy.as_table().ok_or_else(|| {
                invalid_role(role_name, format!("server {server_name} policy is invalid"))
            })?;
            for (key, item) in policy {
                if !ROLE_MCP_SERVER_POLICY_KEYS.contains(&key) {
                    return Err(invalid_role(
                        role_name,
                        format!(
                            "plugins.{capability_root_id}.mcp_servers.{server_name}.{key} is not part of Role MCP server policy"
                        ),
                    ));
                }
                if matches!(key, "enabled" | "required") && item.as_bool().is_none() {
                    return Err(invalid_role(
                        role_name,
                        format!(
                            "plugins.{capability_root_id}.mcp_servers.{server_name}.{key} must be a boolean"
                        ),
                    ));
                }
                if key == "omit_tools_from" {
                    validate_omitted_tool_surfaces(
                        role_name,
                        capability_root_id,
                        server_name,
                        item,
                    )?;
                }
            }
            let mut runtime = Table::new();
            for key in ROLE_MCP_SERVER_POLICY_KEYS {
                if let Some(item) = policy.get(key) {
                    runtime[key] = item.clone();
                }
            }
            runtime["command"] = value(path_text("prepared MCP command", &transport.command)?);
            let mut args = Array::new();
            for argument in &transport.args {
                args.push(argument.as_str());
            }
            runtime["args"] = value(args);
            if let Some(timeout) = transport.startup_timeout_sec {
                runtime["startup_timeout_sec"] = value(timeout);
            }
            if let Some(timeout) = transport.tool_timeout_sec {
                runtime["tool_timeout_sec"] = value(timeout);
            }
            let mut env = Table::new();
            let mut inherited_env_vars = Array::new();
            let tool_state_root = profile_home
                .join(".open-web-codex")
                .join("mcp-state")
                .join(capability_root_id);
            for binding in &transport.env_bindings {
                match binding {
                    PreparedEnvironmentBinding::ProfileHome { name } => {
                        env[name] = value(path_text("Profile home binding", profile_home)?);
                    }
                    PreparedEnvironmentBinding::ToolStateRoot { name } => {
                        env[name] = value(path_text("Tool state binding", &tool_state_root)?);
                    }
                    PreparedEnvironmentBinding::DependencyRoot { name, root } => {
                        env[name] = value(path_text("dependency root binding", root)?);
                    }
                    PreparedEnvironmentBinding::Host { name } => {
                        inherited_env_vars.push(name.as_str());
                    }
                }
            }
            runtime["env"] = Item::Table(env);
            runtime["env_vars"] = value(inherited_env_vars);
            runtime_servers[server_name] = Item::Table(runtime);
        }
    }
    role.remove("plugins");
    role["mcp_servers"] = Item::Table(runtime_servers);
    Ok(())
}

fn validate_omitted_tool_surfaces(
    role_name: &str,
    capability_root_id: &str,
    server_name: &str,
    item: &Item,
) -> Result<(), CopilotPackageError> {
    let values = item.as_array().ok_or_else(|| {
        invalid_role(
            role_name,
            format!(
                "plugins.{capability_root_id}.mcp_servers.{server_name}.omit_tools_from must be an array"
            ),
        )
    })?;
    if values.len() != 1 {
        return Err(invalid_role(
            role_name,
            format!(
                "plugins.{capability_root_id}.mcp_servers.{server_name}.omit_tools_from must be [\"direct\"]"
            ),
        ));
    }
    if values.iter().next().and_then(|value| value.as_str()) != Some("direct") {
        return Err(invalid_role(
            role_name,
            format!(
                "plugins.{capability_root_id}.mcp_servers.{server_name}.omit_tools_from must be [\"direct\"]"
            ),
        ));
    }
    Ok(())
}

fn invalid_role(role: &str, message: impl Into<String>) -> CopilotPackageError {
    CopilotPackageError::InvalidRoleTemplate {
        role: role.to_string(),
        message: message.into(),
    }
}

fn finish_role_template(role: DocumentMut) -> String {
    role.to_string()
}

fn path_text<'a>(component: &'static str, path: &'a Path) -> Result<&'a str, CopilotPackageError> {
    path.to_str().ok_or_else(|| {
        unavailable_message(
            component,
            format!("path is not valid UTF-8: {}", path.display()),
        )
    })
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUPERVISOR_SKILL: &str =
        include_str!("../../../../copilots/warehouse-network/skills/warehouse-supervisor/SKILL.md");
    const DATA_SKILL: &str =
        include_str!("../../../../copilots/warehouse-network/skills/warehouse-data/SKILL.md");
    const PLANNING_SKILL: &str = include_str!(
        "../../../../copilots/warehouse-network/skills/warehouse-network-planning/SKILL.md"
    );
    const MAP_DELIVERY_SKILL: &str = include_str!(
        "../../../../copilots/warehouse-network/skills/warehouse-map-delivery/SKILL.md"
    );
    const SINGLE_AGENT_MAP_DELIVERY_SKILL: &str = include_str!(
        "../../../../copilots/warehouse-network-single-agent/skills/warehouse-single-map-delivery/SKILL.md"
    );
    const SINGLE_AGENT_DATA_SKILL: &str = include_str!(
        "../../../../copilots/warehouse-network-single-agent/skills/warehouse-data-preparation/SKILL.md"
    );
    const SINGLE_AGENT_PLANNING_SKILL: &str = include_str!(
        "../../../../copilots/warehouse-network-single-agent/skills/warehouse-single-network-planning/SKILL.md"
    );
    const SINGLE_AGENT_ROOT_SKILL: &str = include_str!(
        "../../../../copilots/warehouse-network-single-agent/skills/warehouse-single-agent/SKILL.md"
    );
    const DATA_ROLE: &str =
        include_str!("../../../../copilots/warehouse-network/agents/data_agent.toml");
    const NETWORK_ROLE: &str =
        include_str!("../../../../copilots/warehouse-network/agents/network_agent.toml");
    const ROOT_ROLE: &str = include_str!(
        "../../../../copilots/warehouse-network/agents/warehouse_supervisor_root.toml"
    );
    const SINGLE_AGENT_ROOT_ROLE: &str = include_str!(
        "../../../../copilots/warehouse-network-single-agent/agents/warehouse_single_agent.toml"
    );

    #[test]
    fn root_agent_toml_becomes_flat_app_server_config_overrides() {
        let config = CopilotPackageAssets::flatten_runtime_config(serde_json::json!({
            "features": {"multi_agent": false},
            "plugins": {"supply_chain": {"enabled": true}},
            "skills": {"config": [{"name": "warehouse-single-agent", "enabled": true}]},
        }))
        .expect("flatten Root Agent config");

        assert_eq!(config["features.multi_agent"], false);
        assert_eq!(config["plugins.supply_chain.enabled"], true);
        assert_eq!(config["skills.config"][0]["name"], "warehouse-single-agent");
        assert!(config.get("features").is_none());
        assert!(config.get("plugins").is_none());
    }

    #[test]
    fn single_agent_root_keeps_sandboxed_calculation_without_multi_agent() {
        let role = parse_role_template("warehouse_single_agent", SINGLE_AGENT_ROOT_ROLE)
            .expect("parse single-Agent Root Role");

        assert_eq!(role["features"]["shell_tool"].as_bool(), Some(true));
        assert_eq!(role["features"]["multi_agent"].as_bool(), Some(false));
        let instructions = role["developer_instructions"]
            .as_str()
            .expect("single-Agent developer instructions");
        assert!(instructions.contains("prepared_input_relative_path"));
        assert!(instructions.contains("outputs/warehouse-network/calculations/"));
        assert!(instructions.contains("只有用户明确要求脚本核算时"));
        assert!(instructions.contains("同时保存可复用的 `.py` 脚本和 `.json` 结果"));
        assert!(instructions.contains("`create_network_map_card` 成功后立即回复并结束"));
        assert!(
            role["plugins"]["map_utils"]["mcp_servers"]["map_utils"]["tools"]
                .get("publish_workspace_geojson")
                .is_none(),
            "single-Agent final GeoJSON export must inherit the server default prompt",
        );
    }

    #[test]
    fn warehouse_roles_separate_base_identity_from_developer_constraints() {
        for (role_name, contents, expected_base) in [
            ("warehouse_supervisor_root", ROOT_ROLE, "你是仓网协调助手"),
            ("data_agent", DATA_ROLE, "你是仓网数据准备专家"),
            ("network_agent", NETWORK_ROLE, "你是仓网规划专家"),
        ] {
            let role = parse_role_template(role_name, contents).expect("parse warehouse Role");
            let instructions = role["instructions"]
                .as_str()
                .expect("base Role instructions");
            let developer_instructions = role["developer_instructions"]
                .as_str()
                .expect("Role developer instructions");
            assert!(instructions.contains(expected_base));
            assert!(!developer_instructions.contains(expected_base));
        }
    }

    #[test]
    fn warehouse_map_delivery_skill_preserves_default_map_visual_hierarchy() {
        for skill in [MAP_DELIVERY_SKILL, SINGLE_AGENT_MAP_DELIVERY_SKILL] {
            let has_hex_color = skill.as_bytes().windows(7).any(|window| {
                window[0] == b'#'
                    && window[1..]
                        .iter()
                        .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(byte))
            });
            assert!(!has_hex_color);
            assert!(!skill.contains("半径"));
            assert!(skill.contains("用户没有指定样式"));
            assert!(skill.contains("中心仓"));
            assert!(skill.contains("越库仓（XD）"));
            assert!(skill.contains("候选仓"));
            assert!(skill.contains("启用/关闭状态"));
            assert!(skill.contains("时效结果"));
        }
    }

    #[test]
    fn warehouse_supervisor_skill_does_not_retry_terminal_child_failures() {
        assert!(SUPERVISOR_SKILL.contains("权限拒绝、身份不一致、取消、超时、外部失败或能力不可用"));
        assert!(SUPERVISOR_SKILL.contains("不自动重试或另建同角色 child"));
        assert!(SUPERVISOR_SKILL.contains("::codex-inline-vis{artifact=\"...\"}"));
        assert!(SUPERVISOR_SKILL.contains("逐字复制 child 返回"));
        assert!(SUPERVISOR_SKILL.contains("不得声称地图已经展示"));
        assert!(PLANNING_SKILL.contains("在当前候选和假设下无法达到目标"));
    }

    #[test]
    fn warehouse_supervisor_skill_selects_the_native_child_continuation_mode() {
        assert!(SUPERVISOR_SKILL.contains("先 `resume_agent(id)`"));
        assert!(SUPERVISOR_SKILL.contains("`fork_turns=\"none\"`"));
        assert!(!SUPERVISOR_SKILL.contains("`list_agents`"));
        assert!(!SUPERVISOR_SKILL.contains("`followup_task`"));
        assert!(SUPERVISOR_SKILL.contains("`$warehouse-data`"));
        assert!(SUPERVISOR_SKILL.contains("`$warehouse-network-planning`"));
        assert!(SUPERVISOR_SKILL.contains("`$warehouse-map-delivery`"));
        assert!(SUPERVISOR_SKILL.contains("`agent_type=\"data_agent\"`"));
        assert!(SUPERVISOR_SKILL.contains("`agent_type=\"network_agent\"`"));
        assert!(SUPERVISOR_SKILL.contains("`spawn_agent.items`"));
        assert!(SUPERVISOR_SKILL.contains("完整的用户业务目标"));
        assert!(SUPERVISOR_SKILL.contains("一次纳入候选仓来源"));
        assert!(SUPERVISOR_SKILL.contains("用 `send_input.items` 发送本次所需 Skill"));
        assert!(SUPERVISOR_SKILL.contains("同一任务中的同一角色默认复用原 child"));
        assert!(SUPERVISOR_SKILL.contains("普通完成后不调用 `close_agent`"));
        assert!(SUPERVISOR_SKILL
            .contains("只有找不到原 child、原 child 无法恢复、角色不同或 Workspace 不同时才新建"));
        assert!(SUPERVISOR_SKILL.contains("只有本次确实缺少所需工具时才执行 `tool_search`"));
        assert!(SUPERVISOR_SKILL.contains("恢复 child 或开始新一轮对话本身不是重新搜索的理由"));
        assert!(SUPERVISOR_SKILL.contains("`list_mcp_resources`"));
        assert!(SUPERVISOR_SKILL.contains("不调用 goal/plan 工具"));
        assert!(!SUPERVISOR_SKILL.contains("canonical Thread history"));
        assert!(!SUPERVISOR_SKILL.contains("Chat adapter"));
        assert!(!SUPERVISOR_SKILL.contains("ToolRouter"));
    }

    #[test]
    fn warehouse_roles_keep_minimal_tool_reuse_and_task_boundaries() {
        for role in [DATA_ROLE, NETWORK_ROLE, SINGLE_AGENT_ROOT_ROLE] {
            assert!(role.contains("需要的工具已经可用时直接调用"));
            assert!(role.contains("新一轮对话或恢复本身不是重复搜索的理由"));
            assert!(!role.contains("canonical Thread history"));
            assert!(!role.contains("ToolSearchOutput"));
            assert!(!role.contains("projected into the current request"));
            assert!(!role.contains("Chat adapter"));
            assert!(!role.contains("ToolRouter"));
        }
        assert!(DATA_ROLE.contains("$warehouse-data"));
        assert!(NETWORK_ROLE.contains("仓网规划与地图 Skill"));
        assert!(SINGLE_AGENT_ROOT_ROLE.contains("数据准备、仓网规划和地图 Skill"));
    }

    #[test]
    fn warehouse_data_skill_stops_after_prepared_input_terminal_result() {
        assert!(DATA_SKILL.contains("已有数据可复用或准备完成"));
        assert!(DATA_SKILL.contains("返回 `needs_input`"));
        assert!(DATA_SKILL.contains("返回 `source_changed` 时重新检查一次"));
        assert!(DATA_SKILL.contains("时效、达标率或成本分析"));
        assert!(DATA_SKILL.contains("路线或报价"));
        assert!(DATA_SKILL.contains("首次准备就同时纳入候选仓"));
        assert!(SINGLE_AGENT_DATA_SKILL.contains("时效、达标率或成本分析"));
        assert!(SINGLE_AGENT_DATA_SKILL.contains("路线或报价"));
    }

    #[test]
    fn warehouse_data_skills_stop_and_ask_on_typed_missing_input() {
        assert!(DATA_SKILL.contains("只交接本次所选数据的真实缺口"));
        assert!(SUPERVISOR_SKILL.contains("Data 或 Network 返回 `needs_input`"));
        assert!(SUPERVISOR_SKILL.contains("`request_user_input`"));
        assert!(SUPERVISOR_SKILL.contains("`id`、标题、问题和选项原样传给"));
        assert!(SUPERVISOR_SKILL.contains("不得用普通文字假装已经询问"));
        assert!(SINGLE_AGENT_DATA_SKILL.contains("用一次输入卡片询问"));
        assert!(SINGLE_AGENT_ROOT_SKILL.contains("`request_user_input`"));
        assert!(SINGLE_AGENT_ROOT_SKILL.contains("`id`、标题、问题和选项原样传给"));
        assert!(SINGLE_AGENT_ROOT_SKILL.contains("不得用普通文字假装已经询问"));
    }

    #[test]
    fn warehouse_map_delivery_skills_require_an_explicit_map_spec_selection() {
        for skill in [MAP_DELIVERY_SKILL, SINGLE_AGENT_MAP_DELIVERY_SKILL] {
            assert!(skill.contains("`map_spec_ref`"));
            assert!(skill.contains("请用户重新选择"));
        }
        assert!(SUPERVISOR_SKILL.contains("只使用用户通过“基于此图修改”选中的 `map_spec_ref`"));
    }

    #[test]
    fn warehouse_map_delivery_skills_use_the_domain_map_builder() {
        for skill in [MAP_DELIVERY_SKILL, SINGLE_AGENT_MAP_DELIVERY_SKILL] {
            assert!(skill.contains("`create_network_map_card`"));
            assert!(skill.contains("`prepare_network_coverage_map`"));
            assert!(skill.contains("用户要求的服务时限"));
            assert!(skill.contains("地图只展示规划工具已经确认的结果"));
            assert!(!skill.contains("`create_map_card`"));
            assert!(skill.contains("不自行拼接 GeoJSON 或图层"));
            assert!(skill.contains("`publish_workspace_geojson`"));
        }
        let multi_terminal = "`create_network_map_card` 或 `present_map_card` 成功后立即返回";
        let single_terminal = "`create_network_map_card` 成功后立即返回";
        assert!(MAP_DELIVERY_SKILL.contains(multi_terminal));
        assert!(SINGLE_AGENT_MAP_DELIVERY_SKILL.contains(single_terminal));
        assert!(DATA_SKILL.contains("候选仓信息不得因基线只计算现有仓而从准备结果中删除"));
        assert!(SUPERVISOR_SKILL.contains("`prepared_input_relative_path`"));
        assert!(SUPERVISOR_SKILL.contains("`input_identity`"));
    }

    #[test]
    fn warehouse_roots_require_business_language_and_evidence_boundaries() {
        for skill in [SUPERVISOR_SKILL, SINGLE_AGENT_ROOT_SKILL] {
            assert!(skill.contains("最终回答面向仓网业务人员"));
            assert!(skill.contains("先用一句话给出业务结论"));
            assert!(skill.contains("不超过四个短要点"));
            assert!(skill.contains("开场句、标题和正文都用中文"));
            assert!(skill.contains("不要把内部系统名"));
            assert!(skill.contains("明确区分“结果已确认”“根据结果推断”和“现有结果无法确认”"));
            assert!(skill.contains("地图上的仓库归属只能证明最终归属"));
            assert!(skill.contains("当前结果不足以判断具体原因"));
            assert!(skill.contains("不要生造“成本制分配”"));
        }
        assert!(ROOT_ROLE.contains("只使用业务语言"));
        assert!(ROOT_ROLE.contains("不描述内部协作、角色状态或工具过程"));
        assert!(!ROOT_ROLE.contains("$warehouse-supervisor"));
        assert!(SINGLE_AGENT_ROOT_ROLE.contains("面向用户的回答"));
        assert!(SINGLE_AGENT_ROOT_SKILL.contains("`$warehouse-data-preparation`"));
        assert!(SINGLE_AGENT_ROOT_SKILL.contains("`$warehouse-single-network-planning`"));
        assert!(SINGLE_AGENT_ROOT_SKILL.contains("`$warehouse-single-map-delivery`"));
        assert!(SINGLE_AGENT_ROOT_SKILL.contains("`list_mcp_resources`"));
        for skill in [PLANNING_SKILL, SINGLE_AGENT_PLANNING_SKILL] {
            assert!(skill.contains("`route_method=\"provided\"`"));
            assert!(skill.contains("不得自行填写绕路系数、平均速度或导航费用"));
            assert!(skill.contains("首次返回 `needs_input`"));
            assert!(skill.contains("不得重调同一工具"));
        }
        assert!(SINGLE_AGENT_PLANNING_SKILL.contains("`warehouse_quote_mean_calculation.v1`"));
        assert!(SINGLE_AGENT_PLANNING_SKILL.contains("`plan_cost_matrix`"));
    }

    fn write_file(path: &Path, contents: &str, executable: bool) {
        fs::create_dir_all(path.parent().expect("parent")).expect("create parent");
        fs::write(path, contents).expect("write fixture");
        #[cfg(unix)]
        if executable {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).expect("make executable");
        }
        let _ = executable;
    }

    fn fixture() -> (tempfile::TempDir, CopilotPackageAssets, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().expect("temp dir");
        let prepared = temp.path().join("prepared 运行态");
        let build_store = temp.path().join("tool-builds");
        let build_root = build_store
            .join("builds/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let supply_python = build_root.join("tool-environments/supply_chain/venv/bin/python");
        let maps_python = build_root.join("tool-environments/map_utils/venv/bin/python");
        let system_python = build_root.join("system-python");
        let style_spec = build_root.join("tool-environments/map_utils/dependencies/style-spec");
        let profile = temp.path().join("profile 用户");
        let package = temp.path().join("copilot package");
        for directory in [&profile, &style_spec] {
            fs::create_dir_all(directory).expect("fixture directory");
        }
        write_file(
            &package.join("copilot.toml"),
            r#"schema_version = 1
id = "warehouse-network"
display_name = "Warehouse network"

[root]
skill = "warehouse-supervisor"
task_skills = "none"
agent = "warehouse_supervisor_root"

[[skills]]
id = "warehouse-supervisor"
path = "skills/warehouse-supervisor"

[[skills]]
id = "warehouse-data"
path = "skills/warehouse-data"

[[skills]]
id = "warehouse-network-planning"
path = "skills/warehouse-network-planning"

[[skills]]
id = "warehouse-map-delivery"
path = "skills/warehouse-map-delivery"

[[agents]]
id = "warehouse_supervisor_root"
role = "agents/warehouse_supervisor_root.toml"

[[agents]]
id = "data_agent"
role = "agents/data_agent.toml"

[[agents]]
id = "network_agent"
role = "agents/network_agent.toml"

[[tools]]
id = "supply_chain"
root = "tools/planner"
runtime = "tools/planner/runtime.toml"

[[tools]]
id = "map_utils"
root = "tools/maps"
runtime = "tools/maps/runtime.toml"
"#,
            false,
        );
        write_file(
            &package.join("skills/warehouse-supervisor/SKILL.md"),
            SUPERVISOR_SKILL,
            false,
        );
        write_file(
            &package.join("skills/warehouse-data/SKILL.md"),
            DATA_SKILL,
            false,
        );
        write_file(
            &package.join("skills/warehouse-network-planning/SKILL.md"),
            PLANNING_SKILL,
            false,
        );
        write_file(
            &package.join("skills/warehouse-map-delivery/SKILL.md"),
            MAP_DELIVERY_SKILL,
            false,
        );
        write_file(&package.join("agents/data_agent.toml"), DATA_ROLE, false);
        write_file(
            &package.join("agents/warehouse_supervisor_root.toml"),
            ROOT_ROLE,
            false,
        );
        write_file(
            &package.join("agents/network_agent.toml"),
            NETWORK_ROLE,
            false,
        );
        write_file(
            &package.join("tools/planner/runtime.toml"),
            "schema_version = 1\n",
            false,
        );
        write_file(
            &package.join("tools/maps/runtime.toml"),
            "schema_version = 1\n",
            false,
        );
        write_file(&system_python, "#!/bin/sh\n", true);
        #[cfg(unix)]
        {
            fs::create_dir_all(supply_python.parent().expect("supply Python parent"))
                .expect("create managed virtual-environment bin");
            std::os::unix::fs::symlink(&system_python, &supply_python)
                .expect("link managed virtual-environment Python");
        }
        #[cfg(not(unix))]
        write_file(&supply_python, "#!/bin/sh\n", true);
        write_file(&maps_python, "#!/bin/sh\n", true);
        write_file(
            &build_root.join(".copilot-tool-build.v1.json"),
            &serde_json::to_string_pretty(&serde_json::json!({
                "schemaVersion": 1,
                "preparationFingerprint": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "capabilityRoots": []
            }))
            .expect("serialize build marker"),
            false,
        );
        let descriptor = prepared.join("copilot-sdk/prepared-tools.v1.json");
        write_file(
            &descriptor,
            &serde_json::to_string_pretty(&serde_json::json!({
                "schemaVersion": 1,
                "compositionDescriptorSha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "deliveries": [
                    {
                        "id": "map-card",
                        "server": "map_utils",
                        "tool": "create_network_map_card",
                        "kind": "inline_geojson_map_card",
                        "schema": "map.v3",
                        "mimeType": "application/vnd.open-web-codex.map-card+json",
                        "displayName": "Warehouse network map"
                    },
                    {
                        "id": "map-card-custom",
                        "server": "map_utils",
                        "tool": "create_map_card",
                        "kind": "inline_geojson_map_card",
                        "schema": "map.v3",
                        "mimeType": "application/vnd.open-web-codex.map-card+json",
                        "displayName": "Custom warehouse map"
                    },
                    {
                        "id": "map-card-revision",
                        "server": "map_utils",
                        "tool": "revise_map_card",
                        "kind": "inline_geojson_map_card",
                        "schema": "map.v3",
                        "mimeType": "application/vnd.open-web-codex.map-card+json",
                        "displayName": "Revised interactive map"
                    }
                ],
                "capabilityRoots": [
                    {
                        "id": "supply_chain",
                        "servers": [
                            {
                                "id": "supply_chain_data",
                                "transport": "stdio",
                                "command": supply_python,
                                "args": ["-m", "prepared.data"],
                                "envBindings": [
                                    {"name": "CODEX_HOME", "source": "profile_home"},
                                    {"name": "HTTP_PROXY", "source": "host"}
                                ]
                            },
                            {
                                "id": "supply_chain",
                                "transport": "stdio",
                                "command": supply_python,
                                "args": ["-m", "prepared.network"],
                                "envBindings": [
                                    {"name": "CODEX_HOME", "source": "profile_home"},
                                    {"name": "HTTP_PROXY", "source": "host"}
                                ]
                            }
                        ]
                    },
                    {
                        "id": "map_utils",
                        "servers": [{
                            "id": "map_utils",
                            "transport": "stdio",
                            "command": maps_python,
                            "args": ["-m", "prepared.maps"],
                            "envBindings": [
                                {"name": "TOOL_STATE", "source": "tool_state_root"},
                                {
                                    "name": "DEPENDENCY_ASSETS",
                                    "source": "dependency_root",
                                    "dependency": "style-spec",
                                    "resolvedRoot": style_spec
                                },
                                {"name": "HTTP_PROXY", "source": "host"}
                            ],
                            "startupTimeoutSec": 60,
                            "toolTimeoutSec": 90
                        }]
                    }
                ]
            }))
            .expect("serialize descriptor"),
            false,
        );

        let assets = CopilotPackageAssets::resolve(&package, &descriptor, &build_store)
            .expect("resolve assets");
        (temp, assets, profile, descriptor)
    }

    #[test]
    fn renders_native_profile_seeds_with_role_local_mcp_only() {
        let (_temp, assets, profile, _descriptor) = fixture();
        assert_eq!(assets.id(), "warehouse-network");
        assert!(matches!(
            assets
                .deliveries()
                .for_item(
                    serde_json::json!({"server":"map_utils","tool":"create_network_map_card"})
                        .as_object()
                        .expect("item")
                )
                .map(|contract| &contract.kind),
            Some(DeliveryKind::InlineGeoJsonMapCard)
        ));
        assert!(matches!(
            assets
                .deliveries()
                .for_item(
                    serde_json::json!({"server":"map_utils","tool":"revise_map_card"})
                        .as_object()
                        .expect("item")
                )
                .map(|contract| &contract.kind),
            Some(DeliveryKind::InlineGeoJsonMapCard)
        ));
        assert_eq!(
            assets.root_skill_config(&profile),
            vec![
                ThreadSkillConfig {
                    name: "warehouse-supervisor".to_string(),
                    enabled: true,
                    main_prompt: Some(profile.join("skills/warehouse-supervisor/SKILL.md"),),
                },
                ThreadSkillConfig {
                    name: "warehouse-data".to_string(),
                    enabled: false,
                    main_prompt: None,
                },
                ThreadSkillConfig {
                    name: "warehouse-network-planning".to_string(),
                    enabled: false,
                    main_prompt: None,
                },
                ThreadSkillConfig {
                    name: "warehouse-map-delivery".to_string(),
                    enabled: false,
                    main_prompt: None,
                },
            ]
        );
        let mut direct_root_assets = assets.clone();
        direct_root_assets.root_task_skill_access = RootTaskSkillAccess::All;
        assert!(
            direct_root_assets
                .root_skill_config(&profile)
                .iter()
                .all(|skill| skill.enabled),
            "a direct-execution Root must retain its declared task Skill catalog"
        );
        let data = assets
            .render_role("data_agent", &profile)
            .expect("data role");
        let network = assets
            .render_role("network_agent", &profile)
            .expect("network role");

        let data = data.parse::<DocumentMut>().expect("parse data role");
        let network = network.parse::<DocumentMut>().expect("parse network role");
        assert!(
            network["mcp_servers"].get("supply_chain_data").is_none(),
            "Network Role must consume the Data Agent reference through its planning Tools, not configure the Data MCP server",
        );
        assert!(
            data.get("__codex_runtime_mcp_projection").is_none(),
            "managed Role MCP provenance must not be embedded in Role file content",
        );
        let root_execution = assets
            .root_execution_config(&profile)
            .expect("root execution config");
        assert_eq!(
            root_execution.runtime_config["agents.data_agent.runtime_mcp_projection"],
            Value::Bool(true),
            "managed Role MCP projection must be carried by typed Runtime config",
        );
        assert_eq!(
            root_execution.runtime_config["features.shell_tool"],
            Value::Bool(false),
            "warehouse Root must disable shell tools through native config",
        );
        assert_eq!(
            root_execution.runtime_config["features.multi_agent_v2"],
            Value::Bool(false),
            "warehouse Root must use the one established collaboration contract",
        );
        let root = assets
            .render_role("warehouse_supervisor_root", &profile)
            .expect("Root Role");
        let root = root.parse::<DocumentMut>().expect("parse Root Role");
        for role in [&data, &network, &root] {
            assert_eq!(
                role["features"]["shell_tool"].as_bool(),
                Some(false),
                "warehouse Root and child Roles must disable shell tools through native config",
            );
            assert_eq!(
                role["features"]["multi_agent_v2"].as_bool(),
                Some(false),
                "warehouse Root and child Roles must use one established collaboration contract",
            );
        }
        let supply_python = _temp.path().join(
            "tool-builds/builds/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/tool-environments/supply_chain/venv/bin/python",
        );
        assert_eq!(
            data["mcp_servers"]["supply_chain_data"]["command"].as_str(),
            supply_python.to_str(),
            "Role projection must preserve the managed virtual-environment launcher",
        );
        #[cfg(unix)]
        assert_ne!(
            supply_python
                .canonicalize()
                .expect("resolve launcher")
                .to_str(),
            data["mcp_servers"]["supply_chain_data"]["command"].as_str(),
            "Role projection must not replace the launcher with its system executable target",
        );
        assert_eq!(
            data["nickname_candidates"]
                .as_array()
                .expect("data nicknames")
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>(),
            vec!["Wanwan"]
        );
        assert!(data["mcp_servers"]["supply_chain_data"]
            .get("cwd")
            .is_none());
        assert_eq!(
            data["mcp_servers"]["supply_chain_data"]["required"].as_bool(),
            Some(true)
        );
        assert_eq!(
            data["mcp_servers"]["supply_chain_data"]["args"]
                .as_array()
                .expect("data args")
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>(),
            vec!["-m", "prepared.data"]
        );
        assert_eq!(
            data["mcp_servers"]["supply_chain_data"]["default_tools_approval_mode"].as_str(),
            Some("approve")
        );
        assert_eq!(
            data["mcp_servers"]["supply_chain_data"]["omit_tools_from"]
                .as_array()
                .expect("deferred data-tool exposure")
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>(),
            vec!["direct"]
        );
        assert!(data["mcp_servers"]["supply_chain_data"]
            .get("enabled_tools")
            .is_none());
        assert_eq!(
            data["mcp_servers"]["supply_chain_data"]["env"]["CODEX_HOME"].as_str(),
            profile.to_str()
        );
        assert_eq!(
            data["mcp_servers"]["supply_chain_data"]["env_vars"]
                .as_array()
                .and_then(|values| values.get(0))
                .and_then(|value| value.as_str()),
            Some("HTTP_PROXY")
        );
        let data_instructions = data["developer_instructions"]
            .as_str()
            .expect("data instructions");
        assert!(data_instructions.contains("$warehouse-data"));
        assert!(data_instructions.contains("不得使用 shell、Git、临时代码或资源枚举"));
        assert!(data["instructions"]
            .as_str()
            .expect("Data Role base instructions")
            .contains("你是仓网数据准备专家"));
        assert!(network["mcp_servers"]["supply_chain"].get("cwd").is_none());
        assert_eq!(
            network["mcp_servers"]["supply_chain"]["required"].as_bool(),
            Some(true)
        );
        assert_eq!(
            network["mcp_servers"]["supply_chain"]["omit_tools_from"]
                .as_array()
                .expect("deferred supply-chain exposure")
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>(),
            vec!["direct"]
        );
        assert_eq!(
            network["mcp_servers"]["supply_chain"]["args"]
                .as_array()
                .expect("network args")
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>(),
            vec!["-m", "prepared.network"]
        );
        assert!(network["mcp_servers"]["supply_chain"]
            .get("enabled_tools")
            .is_none());
        assert_eq!(
            network["mcp_servers"]["supply_chain"]["default_tools_approval_mode"].as_str(),
            Some("prompt")
        );
        for tool in [
            "prepare_route_matrix",
            "create_navigation_matrix_request",
            "import_navigation_matrix",
            "plan_cost_matrix",
            "prepare_network_distribution_map",
            "prepare_network_comparison_map",
            "prepare_network_coverage_map",
            "evaluate_network_baseline",
            "assess_facility_change",
            "solve_p_median",
            "compare_network_scenarios",
        ] {
            assert_eq!(
                network["mcp_servers"]["supply_chain"]["tools"][tool]["approval_mode"].as_str(),
                Some("approve"),
                "safe Network Tool {tool} must be preapproved",
            );
        }
        assert_eq!(
            network["mcp_servers"]["supply_chain"]["tools"]["publish_network_planning_report"]
                ["approval_mode"]
                .as_str(),
            Some("approve"),
            "final report publishing must be preapproved",
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["command"].as_str(),
            Some(
                path_text(
                    "maps command",
                    &assets.capability_roots["map_utils"].servers["map_utils"].command
                )
                .expect("path")
            )
        );
        assert!(network["mcp_servers"]["map_utils"].get("cwd").is_none());
        assert_eq!(
            network["mcp_servers"]["map_utils"]["args"]
                .as_array()
                .expect("map args")
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>(),
            vec!["-m", "prepared.maps"]
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["env"]["TOOL_STATE"].as_str(),
            profile.join(".open-web-codex/mcp-state/map_utils").to_str()
        );
        assert!(
            network["mcp_servers"]["map_utils"]["env"]["DEPENDENCY_ASSETS"]
                .as_str()
                .is_some()
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["startup_timeout_sec"].as_integer(),
            Some(60)
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["tool_timeout_sec"].as_integer(),
            Some(90)
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["required"].as_bool(),
            Some(false),
            "map presentation must not block route, cost, analysis, or optimization Tools",
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["omit_tools_from"]
                .as_array()
                .expect("deferred map-tool exposure")
                .iter()
                .filter_map(|item| item.as_str())
                .collect::<Vec<_>>(),
            vec!["direct"]
        );
        assert!(network["mcp_servers"]["map_utils"]
            .get("enabled_tools")
            .is_none());
        assert_eq!(
            network["mcp_servers"]["map_utils"]["default_tools_approval_mode"].as_str(),
            Some("prompt")
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["tools"]["create_map_card"]["approval_mode"]
                .as_str(),
            Some("approve")
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["tools"]["create_network_map_card"]
                ["approval_mode"]
                .as_str(),
            Some("approve")
        );
        assert!(
            network["mcp_servers"]["map_utils"]["tools"]
                .get("publish_workspace_geojson")
                .is_none(),
            "final Workspace GeoJSON export must inherit the server default prompt",
        );
        assert_eq!(
            network["mcp_servers"]["map_utils"]["tools"]["revise_map_card"]["approval_mode"]
                .as_str(),
            Some("approve")
        );
        for tool in ["get_route", "distance_matrix", "execute_navigation_matrix"] {
            assert!(
                network["mcp_servers"]["map_utils"]["tools"]
                    .get(tool)
                    .is_none(),
                "external Map Tool {tool} must inherit prompt",
            );
        }
        let network_developer_instructions = network["developer_instructions"]
            .as_str()
            .expect("network instructions");
        assert!(network_developer_instructions.contains("仓网规划与地图 Skill"));
        assert!(network_developer_instructions.contains("prepared_input_relative_path"));
        assert!(network_developer_instructions.contains("不读取原始文件"));
        assert!(network_developer_instructions.contains("不使用 shell、Git"));
        for operation in [
            "list_mcp_resources",
            "list_mcp_resource_templates",
            "read_mcp_resource",
        ] {
            assert!(network_developer_instructions.contains(operation));
        }
        let network_instructions = network["instructions"]
            .as_str()
            .expect("network Role instructions");
        assert!(network_instructions.contains("仓网规划专家"));
        assert!(network_instructions.contains("基于已准备的数据完成仓网分析和地图交付"));
        assert_eq!(
            network["include_permissions_instructions"].as_bool(),
            Some(false)
        );
        assert_eq!(network["include_apps_instructions"].as_bool(), Some(false));
        assert_eq!(
            network["include_collaboration_mode_instructions"].as_bool(),
            Some(false)
        );
        assert_eq!(
            network["skills"]["bundled"]["enabled"].as_bool(),
            Some(false)
        );
        let network_skill_names = network["skills"]["config"]
            .as_array_of_tables()
            .expect("network Skill policy")
            .iter()
            .filter(|skill| skill["enabled"].as_bool() == Some(true))
            .filter_map(|skill| skill["name"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            network_skill_names,
            vec!["warehouse-network-planning", "warehouse-map-delivery",]
        );
        assert!(!data.to_string().contains("__OPEN_WEB_CODEX_"));
        assert!(!network.to_string().contains("__OPEN_WEB_CODEX_"));
        assert!(data.get("plugins").is_none());
        assert!(network.get("plugins").is_none());
        assert_eq!(
            assets.startup_files(&profile).expect("startup files").len(),
            6
        );
        assert!(!profile.join("config.toml").exists());
    }

    #[test]
    fn root_agent_config_is_not_installed_as_a_child_role() {
        let (_temp, mut assets, profile, _descriptor) = fixture();
        assets.root_agent = Some("data_agent".to_string());

        assert_eq!(
            assets.agent_role_ids(),
            vec!["warehouse_supervisor_root", "network_agent"]
        );
        assert_eq!(
            assets.startup_files(&profile).expect("startup files").len(),
            6
        );
        assert!(assets
            .root_execution_config(&profile)
            .expect("Root execution config")
            .runtime_config
            .get("mcp_servers.supply_chain_data.command")
            .is_some());
    }

    #[test]
    fn rejects_transport_cwd_in_author_role_mcp_server_policy() {
        let (_temp, assets, profile, _descriptor) = fixture();
        let mut role = parse_role_template("data", DATA_ROLE).expect("parse data Role");
        role["plugins"]["supply_chain"]["mcp_servers"]["supply_chain_data"]["cwd"] =
            value("/private/tool-source");

        let error = project_role_mcp_servers(&mut role, "data", &assets.capability_roots, &profile)
            .expect_err("author Role transport cwd must be rejected");

        assert!(matches!(
            error,
            CopilotPackageError::InvalidRoleTemplate { message, .. }
                if message.contains("plugins.supply_chain.mcp_servers.supply_chain_data.cwd")
        ));

        let mut role = parse_role_template("data", DATA_ROLE).expect("parse data Role");
        role["plugins"]["supply_chain"]["mcp_servers"]["supply_chain_data"]["required"] =
            value("yes");
        let error = project_role_mcp_servers(&mut role, "data", &assets.capability_roots, &profile)
            .expect_err("non-boolean required policy must be rejected");
        assert!(matches!(
            error,
            CopilotPackageError::InvalidRoleTemplate { message, .. }
                if message.contains("plugins.supply_chain.mcp_servers.supply_chain_data.required must be a boolean")
        ));

        let mut role = parse_role_template("data", DATA_ROLE).expect("parse data Role");
        let mut tool_names = Array::new();
        tool_names.push("discover_workspace_sources");
        role["plugins"]["supply_chain"]["mcp_servers"]["supply_chain_data"]["enabled_tools"] =
            value(tool_names);
        let error = project_role_mcp_servers(&mut role, "data", &assets.capability_roots, &profile)
            .expect_err("Role tool-name allowlists must be rejected");
        assert!(matches!(
            error,
            CopilotPackageError::InvalidRoleTemplate { message, .. }
                if message.contains("plugins.supply_chain.mcp_servers.supply_chain_data.enabled_tools")
        ));
    }

    #[test]
    fn acceptance_profile_disables_only_remote_model_visible_surfaces() {
        assert_eq!(
            disabled_codex_features(),
            [
                CodexFeature::Plugins,
                CodexFeature::RemotePlugin,
                CodexFeature::Apps,
                CodexFeature::ToolSuggest,
            ]
        );
        assert_eq!(
            enabled_codex_features(),
            [CodexFeature::DefaultModeRequestUserInput]
        );
    }

    #[test]
    fn rejects_missing_or_relative_prepared_descriptor_as_unavailable() {
        let temp = tempfile::tempdir().expect("temp dir");
        assert!(matches!(
            load_prepared_descriptor(Path::new("relative"), &temp.path().join("tool-builds")),
            Err(CopilotPackageError::Unavailable { .. })
        ));
        assert!(matches!(
            load_prepared_descriptor(
                &temp.path().join("missing"),
                &temp.path().join("tool-builds"),
            ),
            Err(CopilotPackageError::Unavailable { .. })
        ));
    }

    #[test]
    fn rejects_unknown_fields_and_duplicate_prepared_capability_roots() {
        let (temp, _assets, _profile, descriptor) = fixture();
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&descriptor).expect("read descriptor"))
                .expect("parse descriptor fixture");
        document["unexpected"] = serde_json::json!(true);
        fs::write(
            &descriptor,
            serde_json::to_vec(&document).expect("serialize descriptor"),
        )
        .expect("write unknown field fixture");
        assert!(matches!(
            load_prepared_descriptor(&descriptor, &temp.path().join("tool-builds")),
            Err(CopilotPackageError::Unavailable { .. })
        ));

        document
            .as_object_mut()
            .expect("descriptor object")
            .remove("unexpected");
        document["capabilityRoots"][0]["servers"][0]["cwd"] =
            serde_json::json!("/obsolete/runtime-cwd");
        fs::write(
            &descriptor,
            serde_json::to_vec(&document).expect("serialize descriptor"),
        )
        .expect("write obsolete cwd fixture");
        assert!(matches!(
            load_prepared_descriptor(&descriptor, &temp.path().join("tool-builds")),
            Err(CopilotPackageError::Unavailable { .. })
        ));

        document["capabilityRoots"][0]["servers"][0]
            .as_object_mut()
            .expect("prepared server object")
            .remove("cwd");
        let duplicate = document["capabilityRoots"][0].clone();
        document["capabilityRoots"]
            .as_array_mut()
            .expect("capability roots")
            .push(duplicate);
        fs::write(
            &descriptor,
            serde_json::to_vec(&document).expect("serialize descriptor"),
        )
        .expect("write duplicate root fixture");
        assert!(matches!(
            load_prepared_descriptor(&descriptor, &temp.path().join("tool-builds")),
            Err(CopilotPackageError::Unavailable { .. })
        ));
    }

    #[test]
    fn rejects_prepared_host_binding_outside_proxy_capability() {
        let (temp, _assets, _profile, descriptor) = fixture();
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(&descriptor).expect("read descriptor"))
                .expect("parse descriptor fixture");
        document["capabilityRoots"][0]["servers"][0]["envBindings"][1]["name"] =
            serde_json::json!("OPEN_WEB_CODEX_MASTER_KEY");
        fs::write(
            &descriptor,
            serde_json::to_vec(&document).expect("serialize descriptor"),
        )
        .expect("write host binding fixture");

        assert!(matches!(
            load_prepared_descriptor(&descriptor, &temp.path().join("tool-builds")),
            Err(CopilotPackageError::Unavailable { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn prepared_command_preserves_final_symlink_and_rejects_unsafe_launchers() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("temp dir");
        let build_store = temp.path().join("tool-builds");
        let build = build_store.join(
            "builds/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/tool/venv/bin",
        );
        let marker = build_store.join(
            "builds/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/.copilot-tool-build.v1.json",
        );
        write_file(
            &marker,
            r#"{"schemaVersion":1,"preparationFingerprint":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","capabilityRoots":[]}"#,
            false,
        );
        let bin = build;
        fs::create_dir_all(&bin).expect("prepared bin");
        let system_python = build_store.join(
            "builds/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/system-python",
        );
        write_file(&system_python, "#!/bin/sh\n", true);
        let launcher = bin.join("python");
        symlink(&system_python, &launcher).expect("virtual-environment launcher");
        let mut valid_fingerprint = None;
        assert_eq!(
            prepared_command(
                "prepared command",
                &launcher,
                &build_store,
                &mut valid_fingerprint,
            )
            .expect("valid virtual-environment launcher"),
            launcher,
            "validation must preserve the final launcher symlink path",
        );

        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside).expect("outside directory");
        let outside_command = outside.join("command");
        write_file(&outside_command, "#!/bin/sh\n", true);
        let escaped_parent = bin.join("escaped");
        symlink(&outside, &escaped_parent).expect("escaping parent symlink");

        let broken = bin.join("broken");
        symlink(temp.path().join("missing"), &broken).expect("broken launcher");
        let non_executable = bin.join("non-executable");
        write_file(&non_executable, "#!/bin/sh\n", false);
        let dot_dot = bin.join("../bin/python");
        let mut fingerprint = None;

        for rejected in [
            outside_command,
            escaped_parent.join("command"),
            broken,
            non_executable,
            dot_dot,
        ] {
            assert!(
                matches!(
                    prepared_command(
                        "prepared command",
                        &rejected,
                        &build_store,
                        &mut fingerprint
                    ),
                    Err(CopilotPackageError::Unavailable { .. })
                ),
                "unsafe prepared command was accepted: {}",
                rejected.display(),
            );
        }
    }
}
