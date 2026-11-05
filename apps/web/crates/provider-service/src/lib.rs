//! Platform-owned Provider CRUD orchestration.
//!
//! Codex remains responsible for Provider metadata, model discovery/cache and
//! transport semantics. This service validates authorized platform inputs and
//! composes the typed app-server Provider/config methods.

pub mod secured;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use async_trait::async_trait;
use open_web_codex_platform_contracts::{
    error::ProviderCatalogFailure, ProviderCatalog, ProviderCredentialInput, ProviderKind,
    ProviderModelSummary, ProviderSummary, UpdateProviderModelRequest, UpsertProviderRequest,
};
use open_web_codex_profile_host::ProfileHost;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use thiserror::Error;
use tokio::sync::RwLock;
use url::Url;

/// Platform policy for Profiles that do not yet have an explicit Runtime
/// reasoning setting. An explicit value remains owned by the Profile config.
pub const DEFAULT_REASONING_EFFORT: &str = "medium";

#[derive(Debug, Error)]
pub enum ProviderServiceError {
    #[error("invalid Provider input: {0}")]
    InvalidInput(String),
    #[error("Provider '{0}' does not exist")]
    NotFound(String),
    #[error("Provider operation is not allowed: {0}")]
    Forbidden(String),
    #[error("Codex Provider operation failed: {0}")]
    Runtime(String),
    #[error("Codex returned an invalid Provider response: {0}")]
    InvalidResponse(String),
    #[error("Codex Provider model catalog request failed: {0:?}")]
    ProviderCatalogFailure(ProviderCatalogFailure),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelProviderModelsListResponse {
    result: ModelProviderModelsListResult,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum ModelProviderModelsListResult {
    Success { models: Vec<ProviderModelSummary> },
    Failure { error: CodexProviderCatalogFailure },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
enum CodexProviderCatalogFailure {
    Authentication,
    NotFound,
    RateLimited,
    Upstream,
    Timeout,
    Network,
    InvalidJson,
    IncompatibleSchema,
    EmptyCatalog,
}

impl From<CodexProviderCatalogFailure> for ProviderCatalogFailure {
    fn from(value: CodexProviderCatalogFailure) -> Self {
        match value {
            CodexProviderCatalogFailure::Authentication => Self::Authentication,
            CodexProviderCatalogFailure::NotFound => Self::NotFound,
            CodexProviderCatalogFailure::RateLimited => Self::RateLimited,
            CodexProviderCatalogFailure::Upstream => Self::Upstream,
            CodexProviderCatalogFailure::Timeout => Self::Timeout,
            CodexProviderCatalogFailure::Network => Self::Network,
            CodexProviderCatalogFailure::InvalidJson => Self::InvalidJson,
            CodexProviderCatalogFailure::IncompatibleSchema => Self::IncompatibleSchema,
            CodexProviderCatalogFailure::EmptyCatalog => Self::EmptyCatalog,
        }
    }
}

#[async_trait]
pub trait ProviderTransport: Send + Sync {
    async fn request(&self, method: &str, params: Value) -> Result<Value, String>;
}

/// Typed Provider operations consumed by platform routes. Browser code never
/// receives raw app-server methods or Codex configuration paths.
#[async_trait]
pub trait ProviderOperations: Send + Sync {
    async fn list(&self) -> Result<ProviderCatalog, ProviderServiceError>;
    async fn upsert(
        &self,
        id: &str,
        request: UpsertProviderRequest,
    ) -> Result<ProviderCatalog, ProviderServiceError>;
    async fn select(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError>;
    async fn select_model(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ProviderCatalog, ProviderServiceError>;
    async fn delete(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError>;
    async fn refresh_models(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError>;
    async fn replace_models(
        &self,
        provider_id: &str,
        models: Vec<ProviderModelSummary>,
    ) -> Result<ProviderCatalog, ProviderServiceError>;
    async fn update_model(
        &self,
        provider_id: &str,
        model_id: &str,
        request: UpdateProviderModelRequest,
    ) -> Result<ProviderCatalog, ProviderServiceError>;
}

#[async_trait]
impl ProviderTransport for ProfileHost {
    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        ProfileHost::request(self, method, params)
            .await
            .map_err(|error| error.to_string())
    }
}

#[derive(Clone)]
pub struct ProviderService {
    transport: Arc<dyn ProviderTransport>,
}

impl ProviderService {
    pub fn new(transport: Arc<dyn ProviderTransport>) -> Self {
        Self { transport }
    }

    pub fn for_profile_host(host: ProfileHost) -> Self {
        Self::new(Arc::new(host))
    }

    pub async fn list(&self) -> Result<ProviderCatalog, ProviderServiceError> {
        let response = self
            .transport
            .request("modelProvider/list", json!({}))
            .await
            .map_err(ProviderServiceError::Runtime)?;
        parse_catalog(response)
    }

    pub async fn upsert(
        &self,
        id: &str,
        request: UpsertProviderRequest,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        validate_provider_id(id)?;
        let catalog = self.list().await?;
        let existing = catalog.data.iter().find(|provider| provider.id == id);
        if existing.is_some_and(|provider| provider.kind == ProviderKind::BuiltIn) {
            return Err(ProviderServiceError::Forbidden(format!(
                "built-in Provider '{id}' cannot be edited"
            )));
        }
        let name = required_trimmed(&request.name, "Provider name")?;
        let base_url = validate_base_url(&request.base_url)?;
        if !matches!(request.wire_api.as_str(), "chat" | "responses") {
            return Err(ProviderServiceError::InvalidInput(
                "wire API must be 'chat' or 'responses'".to_string(),
            ));
        }
        let supports_function_tools = request.wire_api == "chat";
        validate_credentials(&request.credentials, existing.is_some())?;

        let provider_path = provider_path(id)?;
        let mut edits = Vec::new();
        if existing.is_none() {
            let mut provider = Map::new();
            provider.insert("name".to_string(), json!(name));
            provider.insert("base_url".to_string(), json!(base_url));
            provider.insert("wire_api".to_string(), json!(request.wire_api));
            provider.insert(
                "supports_function_tools".to_string(),
                json!(supports_function_tools),
            );
            apply_new_credentials(&request.credentials, &mut provider);
            edits.push(config_edit(provider_path.clone(), Value::Object(provider)));
        } else {
            edits.push(config_edit(format!("{provider_path}.name"), json!(name)));
            edits.push(config_edit(
                format!("{provider_path}.base_url"),
                json!(base_url),
            ));
            edits.push(config_edit(
                format!("{provider_path}.wire_api"),
                json!(request.wire_api),
            ));
            edits.push(config_edit(
                format!("{provider_path}.supports_function_tools"),
                json!(supports_function_tools),
            ));
            append_existing_credential_edits(&request.credentials, &provider_path, &mut edits);
        }
        if existing.is_none() || request.select {
            edits.push(config_edit("model_provider".to_string(), json!(id)));
        }
        self.write_config(edits).await?;
        self.list().await
    }

    pub async fn select(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        let catalog = self.require_provider(id).await?;
        if catalog.current_provider_id != id {
            self.write_config(vec![config_edit("model_provider".to_string(), json!(id))])
                .await?;
        }
        self.list().await
    }

    pub async fn select_model(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        let model_id = required_trimmed(model_id, "model id")?;
        let catalog = self.require_provider(provider_id).await?;
        let provider = catalog
            .data
            .iter()
            .find(|provider| provider.id == provider_id)
            .expect("required Provider exists");
        if !provider.models.is_empty()
            && !provider
                .models
                .iter()
                .any(|model| model.model_id == model_id && model.show_in_picker)
        {
            return Err(ProviderServiceError::NotFound(format!(
                "{provider_id}/{model_id}"
            )));
        }
        self.write_config(vec![
            config_edit("model_provider".to_string(), json!(provider_id)),
            config_edit("model".to_string(), json!(model_id)),
        ])
        .await?;
        let mut catalog = self.list().await?;
        catalog.current_model_id = Some(model_id.to_string());
        Ok(catalog)
    }

    pub async fn delete(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        let catalog = self.require_provider(id).await?;
        let provider = catalog
            .data
            .iter()
            .find(|provider| provider.id == id)
            .expect("required Provider exists");
        if provider.kind == ProviderKind::BuiltIn || !provider.can_delete {
            return Err(ProviderServiceError::Forbidden(format!(
                "Provider '{id}' cannot be deleted"
            )));
        }
        if catalog.current_provider_id == id {
            return Err(ProviderServiceError::Forbidden(
                "select another Provider before deleting the current Provider".to_string(),
            ));
        }
        self.write_config(vec![config_edit(provider_path(id)?, Value::Null)])
            .await?;
        self.list().await
    }

    pub async fn refresh_models(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        let catalog = self.require_provider(id).await?;
        let provider = catalog
            .data
            .iter()
            .find(|provider| provider.id == id)
            .expect("required Provider exists");
        if provider.kind == ProviderKind::BuiltIn || !provider.can_fetch_models {
            return Err(ProviderServiceError::Forbidden(format!(
                "Provider '{id}' uses a bundled model catalog"
            )));
        }
        let configured_search_capabilities = provider
            .models
            .iter()
            .map(|model| (model.model_id.as_str(), model.supports_search_tool))
            .collect::<BTreeMap<_, _>>();

        let response = self
            .transport
            .request("modelProvider/models/list", json!({ "providerId": id }))
            .await
            .map_err(ProviderServiceError::Runtime)?;
        let mut models = parse_model_provider_models_list(response)?;
        for model in &mut models {
            if let Some(supports_search_tool) =
                configured_search_capabilities.get(model.model_id.as_str())
            {
                model.supports_search_tool = *supports_search_tool;
            }
        }
        let persisted_models = models
            .iter()
            .filter_map(provider_model_config_from_catalog)
            .collect::<Vec<_>>();
        if persisted_models.len() != models.len() {
            return Err(ProviderServiceError::ProviderCatalogFailure(
                ProviderCatalogFailure::IncompatibleSchema,
            ));
        }
        self.write_config(vec![config_edit(
            format!("{}.models", provider_path(id)?),
            json!(persisted_models),
        )])
        .await?;
        let mut catalog = self.list().await?;
        let provider_index = require_catalog_provider(&catalog, id)?;
        catalog.data[provider_index].model_count = models.len();
        catalog.data[provider_index].models = models;
        Ok(catalog)
    }

    pub async fn update_model(
        &self,
        provider_id: &str,
        model_id: &str,
        request: UpdateProviderModelRequest,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        if request.context_window < 1_024 {
            return Err(ProviderServiceError::InvalidInput(
                "context window must be at least 1024 tokens".to_string(),
            ));
        }
        let model_id = required_trimmed(model_id, "model id")?;
        let catalog = self.require_provider(provider_id).await?;
        let provider = catalog
            .data
            .iter()
            .find(|provider| provider.id == provider_id)
            .expect("required Provider exists");
        if provider.kind == ProviderKind::BuiltIn || !provider.can_edit {
            return Err(ProviderServiceError::Forbidden(
                "built-in model metadata cannot be edited".to_string(),
            ));
        }
        let models = update_provider_models(
            &provider.models,
            model_id,
            request.context_window,
            request.supports_search_tool,
        );
        self.write_model_catalog(provider_id, models).await
    }

    pub async fn replace_models(
        &self,
        provider_id: &str,
        models: Vec<ProviderModelSummary>,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        let catalog = self.require_provider(provider_id).await?;
        let provider = catalog
            .data
            .iter()
            .find(|provider| provider.id == provider_id)
            .expect("required Provider exists");
        if provider.kind == ProviderKind::BuiltIn || !provider.can_edit {
            return Err(ProviderServiceError::Forbidden(
                "built-in model metadata cannot be edited".to_string(),
            ));
        }
        self.write_model_catalog(provider_id, models).await
    }

    async fn write_model_catalog(
        &self,
        provider_id: &str,
        models: Vec<ProviderModelSummary>,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        if models.is_empty() {
            return Err(ProviderServiceError::InvalidInput(
                "Provider model catalog cannot be empty".to_string(),
            ));
        }
        let mut model_ids = BTreeSet::new();
        let persisted_models = models
            .iter()
            .map(|model| {
                if !model_ids.insert(model.model_id.as_str()) {
                    return None;
                }
                provider_model_config_from_catalog(model)
            })
            .collect::<Option<Vec<_>>>()
            .ok_or(ProviderServiceError::ProviderCatalogFailure(
                ProviderCatalogFailure::IncompatibleSchema,
            ))?;
        self.write_config(vec![config_edit(
            format!("{}.models", provider_path(provider_id)?),
            json!(persisted_models),
        )])
        .await?;
        let mut catalog = self.list().await?;
        let provider_index = require_catalog_provider(&catalog, provider_id)?;
        catalog.data[provider_index].model_count = models.len();
        catalog.data[provider_index].models = models;
        Ok(catalog)
    }

    /// Apply the Platform default only to Profiles without an explicit
    /// `model_reasoning_effort` value. The Runtime remains the owner of the
    /// effective setting and performs the typed write.
    pub async fn ensure_default_reasoning_effort(&self) -> Result<bool, ProviderServiceError> {
        let response = self
            .transport
            .request(
                "config/read",
                json!({ "includeLayers": false, "cwd": null }),
            )
            .await
            .map_err(ProviderServiceError::Runtime)?;
        let config = unwrap_result(&response).get("config").ok_or_else(|| {
            ProviderServiceError::InvalidResponse(
                "config/read response is missing config".to_string(),
            )
        })?;
        let configured_effort = config.get("model_reasoning_effort").ok_or_else(|| {
            ProviderServiceError::InvalidResponse(
                "config/read response is missing model_reasoning_effort".to_string(),
            )
        })?;
        if configured_effort.is_string() {
            return Ok(false);
        }
        if !configured_effort.is_null() {
            return Err(ProviderServiceError::InvalidResponse(
                "config/read returned an invalid model_reasoning_effort".to_string(),
            ));
        }

        self.write_config(vec![config_edit(
            "model_reasoning_effort".to_string(),
            json!(DEFAULT_REASONING_EFFORT),
        )])
        .await?;
        Ok(true)
    }

    async fn require_provider(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        validate_provider_id(id)?;
        let catalog = self.list().await?;
        if catalog.data.iter().any(|provider| provider.id == id) {
            Ok(catalog)
        } else {
            Err(ProviderServiceError::NotFound(id.to_string()))
        }
    }

    async fn write_config(&self, edits: Vec<Value>) -> Result<(), ProviderServiceError> {
        let response = self
            .transport
            .request(
                "config/batchWrite",
                json!({ "edits": edits, "reloadUserConfig": true }),
            )
            .await
            .map_err(ProviderServiceError::Runtime)?;
        let response = unwrap_result(&response);
        if response.get("status").and_then(Value::as_str) == Some("error") {
            let message = response
                .pointer("/error/message")
                .and_then(Value::as_str)
                .or_else(|| response.get("message").and_then(Value::as_str))
                .unwrap_or("Provider configuration write failed");
            return Err(ProviderServiceError::Runtime(message.to_string()));
        }
        Ok(())
    }
}

#[async_trait]
impl ProviderOperations for ProviderService {
    async fn list(&self) -> Result<ProviderCatalog, ProviderServiceError> {
        ProviderService::list(self).await
    }

    async fn upsert(
        &self,
        id: &str,
        request: UpsertProviderRequest,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        ProviderService::upsert(self, id, request).await
    }

    async fn select(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        ProviderService::select(self, id).await
    }

    async fn select_model(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        ProviderService::select_model(self, provider_id, model_id).await
    }

    async fn delete(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        ProviderService::delete(self, id).await
    }

    async fn refresh_models(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        ProviderService::refresh_models(self, id).await
    }

    async fn replace_models(
        &self,
        provider_id: &str,
        models: Vec<ProviderModelSummary>,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        ProviderService::replace_models(self, provider_id, models).await
    }

    async fn update_model(
        &self,
        provider_id: &str,
        model_id: &str,
        request: UpdateProviderModelRequest,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        ProviderService::update_model(self, provider_id, model_id, request).await
    }
}

/// Functional in-memory Provider implementation used by the server's explicit
/// fake mode. It mirrors the public contract without exposing a fake raw RPC
/// surface to browser code.
#[derive(Clone)]
pub struct InMemoryProviderService {
    catalog: Arc<RwLock<ProviderCatalog>>,
}

impl Default for InMemoryProviderService {
    fn default() -> Self {
        let openai_model = ProviderModelSummary {
            model_id: "gpt-5.1-codex".to_string(),
            model_name: Some("GPT-5.1 Codex".to_string()),
            max_token_len: Some(200_000),
            max_output_tokens: None,
            show_in_picker: true,
            context_window: Some(200_000),
            supports_search_tool: false,
        };
        let mock_model = ProviderModelSummary {
            model_id: "mock-codex".to_string(),
            model_name: Some("Mock Codex".to_string()),
            max_token_len: Some(64_000),
            max_output_tokens: Some(8_192),
            show_in_picker: true,
            context_window: Some(64_000),
            supports_search_tool: false,
        };
        Self {
            catalog: Arc::new(RwLock::new(ProviderCatalog {
                current_provider_id: "mock".to_string(),
                current_model_id: Some("mock-codex".to_string()),
                data: vec![
                    ProviderSummary {
                        id: "openai".to_string(),
                        name: "OpenAI".to_string(),
                        base_url: None,
                        env_key: Some("OPENAI_API_KEY".to_string()),
                        wire_api: "responses".to_string(),
                        supports_function_tools: true,
                        kind: ProviderKind::BuiltIn,
                        is_current: false,
                        model_count: 1,
                        can_edit: false,
                        can_delete: false,
                        can_fetch_models: false,
                        models: vec![openai_model],
                    },
                    ProviderSummary {
                        id: "mock".to_string(),
                        name: "Mock Provider".to_string(),
                        base_url: Some("http://127.0.0.1:9999/v1".to_string()),
                        env_key: None,
                        wire_api: "responses".to_string(),
                        supports_function_tools: false,
                        kind: ProviderKind::Custom,
                        is_current: true,
                        model_count: 1,
                        can_edit: true,
                        can_delete: true,
                        can_fetch_models: true,
                        models: vec![mock_model],
                    },
                ],
            })),
        }
    }
}

#[async_trait]
impl ProviderOperations for InMemoryProviderService {
    async fn list(&self) -> Result<ProviderCatalog, ProviderServiceError> {
        Ok(self.catalog.read().await.clone())
    }

    async fn upsert(
        &self,
        id: &str,
        request: UpsertProviderRequest,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        validate_provider_id(id)?;
        let name = required_trimmed(&request.name, "Provider name")?.to_string();
        let base_url = validate_base_url(&request.base_url)?;
        if !matches!(request.wire_api.as_str(), "chat" | "responses") {
            return Err(ProviderServiceError::InvalidInput(
                "wire API must be 'chat' or 'responses'".to_string(),
            ));
        }
        let supports_function_tools = request.wire_api == "chat";

        let mut catalog = self.catalog.write().await;
        let existing_index = catalog.data.iter().position(|provider| provider.id == id);
        if existing_index.is_some_and(|index| catalog.data[index].kind == ProviderKind::BuiltIn) {
            return Err(ProviderServiceError::Forbidden(format!(
                "built-in Provider '{id}' cannot be edited"
            )));
        }
        validate_credentials(&request.credentials, existing_index.is_some())?;

        let env_key = match &request.credentials {
            ProviderCredentialInput::Preserve => {
                existing_index.and_then(|index| catalog.data[index].env_key.clone())
            }
            ProviderCredentialInput::Environment { env_key } => Some(env_key.trim().to_string()),
            ProviderCredentialInput::Direct { .. } | ProviderCredentialInput::NoCredential => None,
        };
        if let Some(index) = existing_index {
            let provider = &mut catalog.data[index];
            provider.name = name;
            provider.base_url = Some(base_url);
            provider.wire_api = request.wire_api;
            provider.env_key = env_key;
            provider.supports_function_tools = supports_function_tools;
        } else {
            catalog.data.push(ProviderSummary {
                id: id.to_string(),
                name,
                base_url: Some(base_url),
                env_key,
                wire_api: request.wire_api,
                supports_function_tools,
                kind: ProviderKind::Custom,
                is_current: false,
                model_count: 0,
                can_edit: true,
                can_delete: true,
                can_fetch_models: true,
                models: Vec::new(),
            });
        }
        if request.select {
            select_catalog_provider(&mut catalog, id);
        }
        Ok(catalog.clone())
    }

    async fn select(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        let mut catalog = self.catalog.write().await;
        require_catalog_provider(&catalog, id)?;
        select_catalog_provider(&mut catalog, id);
        catalog.current_model_id = catalog
            .data
            .iter()
            .find(|provider| provider.id == id)
            .and_then(|provider| provider.models.iter().find(|model| model.show_in_picker))
            .map(|model| model.model_id.clone());
        Ok(catalog.clone())
    }

    async fn select_model(
        &self,
        provider_id: &str,
        model_id: &str,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        let model_id = required_trimmed(model_id, "model id")?;
        let mut catalog = self.catalog.write().await;
        let index = require_catalog_provider(&catalog, provider_id)?;
        let provider = &catalog.data[index];
        if !provider.models.is_empty()
            && !provider
                .models
                .iter()
                .any(|model| model.model_id == model_id && model.show_in_picker)
        {
            return Err(ProviderServiceError::NotFound(format!(
                "{provider_id}/{model_id}"
            )));
        }
        select_catalog_provider(&mut catalog, provider_id);
        catalog.current_model_id = Some(model_id.to_string());
        Ok(catalog.clone())
    }

    async fn delete(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        let mut catalog = self.catalog.write().await;
        let index = require_catalog_provider(&catalog, id)?;
        let provider = &catalog.data[index];
        if provider.kind == ProviderKind::BuiltIn || !provider.can_delete {
            return Err(ProviderServiceError::Forbidden(format!(
                "Provider '{id}' cannot be deleted"
            )));
        }
        if catalog.current_provider_id == id {
            return Err(ProviderServiceError::Forbidden(
                "select another Provider before deleting the current Provider".to_string(),
            ));
        }
        catalog.data.remove(index);
        Ok(catalog.clone())
    }

    async fn refresh_models(&self, id: &str) -> Result<ProviderCatalog, ProviderServiceError> {
        let catalog = self.catalog.read().await;
        let index = require_catalog_provider(&catalog, id)?;
        if !catalog.data[index].can_fetch_models {
            return Err(ProviderServiceError::Forbidden(format!(
                "Provider '{id}' uses a bundled model catalog"
            )));
        }
        if catalog.data[index].models.is_empty() {
            return Err(ProviderServiceError::ProviderCatalogFailure(
                ProviderCatalogFailure::EmptyCatalog,
            ));
        }
        Ok(catalog.clone())
    }

    async fn replace_models(
        &self,
        provider_id: &str,
        models: Vec<ProviderModelSummary>,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        let mut catalog = self.catalog.write().await;
        let provider_index = require_catalog_provider(&catalog, provider_id)?;
        if catalog.data[provider_index].kind == ProviderKind::BuiltIn
            || !catalog.data[provider_index].can_edit
        {
            return Err(ProviderServiceError::Forbidden(
                "built-in model metadata cannot be edited".to_string(),
            ));
        }
        if models.is_empty() {
            return Err(ProviderServiceError::InvalidInput(
                "Provider model catalog cannot be empty".to_string(),
            ));
        }
        let mut model_ids = BTreeSet::new();
        if models.iter().any(|model| {
            !model_ids.insert(model.model_id.as_str())
                || provider_model_config_from_catalog(model).is_none()
        }) {
            return Err(ProviderServiceError::ProviderCatalogFailure(
                ProviderCatalogFailure::IncompatibleSchema,
            ));
        }
        catalog.data[provider_index].model_count = models.len();
        catalog.data[provider_index].models = models;
        Ok(catalog.clone())
    }

    async fn update_model(
        &self,
        provider_id: &str,
        model_id: &str,
        request: UpdateProviderModelRequest,
    ) -> Result<ProviderCatalog, ProviderServiceError> {
        if request.context_window < 1_024 {
            return Err(ProviderServiceError::InvalidInput(
                "context window must be at least 1024 tokens".to_string(),
            ));
        }
        let model_id = required_trimmed(model_id, "model id")?;
        let mut catalog = self.catalog.write().await;
        let index = require_catalog_provider(&catalog, provider_id)?;
        let provider = &mut catalog.data[index];
        if !provider.can_edit || provider.kind == ProviderKind::BuiltIn {
            return Err(ProviderServiceError::Forbidden(
                "built-in model metadata cannot be edited".to_string(),
            ));
        }
        if let Some(model) = provider
            .models
            .iter_mut()
            .find(|model| model.model_id == model_id)
        {
            model.context_window = Some(request.context_window);
            if let Some(supports_search_tool) = request.supports_search_tool {
                model.supports_search_tool = supports_search_tool;
            }
        } else {
            provider.models.push(ProviderModelSummary {
                model_id: model_id.to_string(),
                model_name: Some(model_id.to_string()),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: Some(request.context_window),
                supports_search_tool: request.supports_search_tool.unwrap_or(false),
            });
        }
        provider.model_count = provider.models.len();
        Ok(catalog.clone())
    }
}

fn require_catalog_provider(
    catalog: &ProviderCatalog,
    id: &str,
) -> Result<usize, ProviderServiceError> {
    validate_provider_id(id)?;
    catalog
        .data
        .iter()
        .position(|provider| provider.id == id)
        .ok_or_else(|| ProviderServiceError::NotFound(id.to_string()))
}

fn select_catalog_provider(catalog: &mut ProviderCatalog, id: &str) {
    catalog.current_provider_id = id.to_string();
    catalog.current_model_id = None;
    for provider in &mut catalog.data {
        provider.is_current = provider.id == id;
    }
}

fn parse_catalog(response: Value) -> Result<ProviderCatalog, ProviderServiceError> {
    serde_json::from_value(unwrap_result(&response).clone())
        .map_err(|error| ProviderServiceError::InvalidResponse(error.to_string()))
}

fn unwrap_result(response: &Value) -> &Value {
    response.get("result").unwrap_or(response)
}

fn validate_provider_id(id: &str) -> Result<(), ProviderServiceError> {
    if id.trim().is_empty()
        || !id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err(ProviderServiceError::InvalidInput(
            "Provider id may contain only letters, numbers, '-' and '_'".to_string(),
        ));
    }
    Ok(())
}

fn provider_path(id: &str) -> Result<String, ProviderServiceError> {
    validate_provider_id(id)?;
    let quoted = serde_json::to_string(id)
        .map_err(|error| ProviderServiceError::InvalidInput(error.to_string()))?;
    Ok(format!("model_providers.{quoted}"))
}

fn required_trimmed<'a>(value: &'a str, label: &str) -> Result<&'a str, ProviderServiceError> {
    let value = value.trim();
    if value.is_empty() {
        Err(ProviderServiceError::InvalidInput(format!(
            "{label} is required"
        )))
    } else {
        Ok(value)
    }
}

fn validate_base_url(value: &str) -> Result<String, ProviderServiceError> {
    let value = required_trimmed(value, "Provider base URL")?;
    let parsed = Url::parse(value).map_err(|_| {
        ProviderServiceError::InvalidInput("Provider base URL is invalid".to_string())
    })?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(ProviderServiceError::InvalidInput(
            "Provider base URL must use http or https".to_string(),
        ));
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn validate_credentials(
    credentials: &ProviderCredentialInput,
    provider_exists: bool,
) -> Result<(), ProviderServiceError> {
    match credentials {
        ProviderCredentialInput::Preserve if !provider_exists => {
            Err(ProviderServiceError::InvalidInput(
                "cannot preserve credentials for a new Provider".to_string(),
            ))
        }
        ProviderCredentialInput::Environment { env_key } => {
            let env_key = required_trimmed(env_key, "API key environment variable")?;
            if !env_key.chars().all(|character| {
                character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
            }) {
                return Err(ProviderServiceError::InvalidInput(
                    "API key environment variable must contain only A-Z, 0-9 and '_'".to_string(),
                ));
            }
            Ok(())
        }
        ProviderCredentialInput::Direct { api_key } => {
            required_trimmed(api_key, "API key").map(|_| ())
        }
        _ => Ok(()),
    }
}

fn apply_new_credentials(credentials: &ProviderCredentialInput, provider: &mut Map<String, Value>) {
    match credentials {
        ProviderCredentialInput::Environment { env_key } => {
            provider.insert("env_key".to_string(), json!(env_key.trim()));
        }
        ProviderCredentialInput::Direct { api_key } => {
            provider.insert(
                "experimental_bearer_token".to_string(),
                json!(api_key.trim()),
            );
        }
        ProviderCredentialInput::Preserve | ProviderCredentialInput::NoCredential => {}
    }
}

fn append_existing_credential_edits(
    credentials: &ProviderCredentialInput,
    provider_path: &str,
    edits: &mut Vec<Value>,
) {
    match credentials {
        ProviderCredentialInput::Preserve => {}
        ProviderCredentialInput::Environment { env_key } => {
            edits.push(config_edit(
                format!("{provider_path}.env_key"),
                json!(env_key.trim()),
            ));
            edits.push(config_edit(
                format!("{provider_path}.experimental_bearer_token"),
                Value::Null,
            ));
        }
        ProviderCredentialInput::Direct { api_key } => {
            edits.push(config_edit(format!("{provider_path}.env_key"), Value::Null));
            edits.push(config_edit(
                format!("{provider_path}.experimental_bearer_token"),
                json!(api_key.trim()),
            ));
        }
        ProviderCredentialInput::NoCredential => {
            edits.push(config_edit(format!("{provider_path}.env_key"), Value::Null));
            edits.push(config_edit(
                format!("{provider_path}.experimental_bearer_token"),
                Value::Null,
            ));
        }
    }
}

fn config_edit(key_path: String, value: Value) -> Value {
    json!({
        "keyPath": key_path,
        "value": value,
        "mergeStrategy": "replace",
    })
}

fn parse_model_provider_models_list(
    response: Value,
) -> Result<Vec<ProviderModelSummary>, ProviderServiceError> {
    let response: ModelProviderModelsListResponse =
        serde_json::from_value(response).map_err(|_| {
            ProviderServiceError::ProviderCatalogFailure(ProviderCatalogFailure::IncompatibleSchema)
        })?;
    match response.result {
        ModelProviderModelsListResult::Success { models } if models.is_empty() => Err(
            ProviderServiceError::ProviderCatalogFailure(ProviderCatalogFailure::EmptyCatalog),
        ),
        ModelProviderModelsListResult::Success { models } => Ok(models),
        ModelProviderModelsListResult::Failure { error } => {
            Err(ProviderServiceError::ProviderCatalogFailure(error.into()))
        }
    }
}

fn provider_model_config_from_catalog(model: &ProviderModelSummary) -> Option<Value> {
    let model_id = model.model_id.as_str();
    if model_id.trim().is_empty()
        || model_id != model_id.trim()
        || model_id.chars().any(char::is_control)
    {
        return None;
    }
    if let Some(model_name) = model.model_name.as_deref() {
        if model_name.trim().is_empty()
            || model_name != model_name.trim()
            || model_name.chars().any(char::is_control)
        {
            return None;
        }
    }
    let mut persisted = Map::new();
    persisted.insert("model_id".to_string(), json!(model_id));
    persisted.insert(
        "model_name".to_string(),
        json!(model.model_name.as_deref().unwrap_or(model_id)),
    );
    persisted.insert("show_in_picker".to_string(), json!(model.show_in_picker));
    persisted.insert(
        "supports_search_tool".to_string(),
        json!(model.supports_search_tool),
    );
    if let Some(value) = model.max_token_len {
        persisted.insert("max_token_len".to_string(), json!(value));
    }
    if let Some(value) = model.max_output_tokens {
        persisted.insert("max_output_tokens".to_string(), json!(value));
    }
    if let Some(value) = model.context_window {
        persisted.insert("context_window".to_string(), json!(value));
    }
    Some(Value::Object(persisted))
}

fn update_provider_models(
    raw_models: &[ProviderModelSummary],
    model_id: &str,
    context_window: i64,
    supports_search_tool: Option<bool>,
) -> Vec<ProviderModelSummary> {
    let mut found = false;
    let mut models = raw_models
        .iter()
        .map(|model| {
            let mut model = model.clone();
            if model.model_id == model_id {
                found = true;
                model.context_window = Some(context_window);
                if let Some(supports_search_tool) = supports_search_tool {
                    model.supports_search_tool = supports_search_tool;
                }
            }
            model
        })
        .collect::<Vec<_>>();
    if !found {
        models.push(ProviderModelSummary {
            model_id: model_id.to_string(),
            model_name: Some(model_id.to_string()),
            max_token_len: None,
            max_output_tokens: None,
            show_in_picker: true,
            context_window: Some(context_window),
            supports_search_tool: supports_search_tool.unwrap_or(false),
        });
    }
    models
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use super::{
        parse_model_provider_models_list, provider_model_config_from_catalog, provider_path,
        update_provider_models, validate_base_url, validate_credentials, InMemoryProviderService,
        ProviderOperations, ProviderService, ProviderServiceError, ProviderTransport,
        DEFAULT_REASONING_EFFORT,
    };
    use async_trait::async_trait;
    use open_web_codex_platform_contracts::{
        error::ProviderCatalogFailure, ProviderCredentialInput, ProviderModelSummary,
        UpsertProviderRequest,
    };
    use serde_json::{json, Value};
    use tokio::sync::Mutex;

    struct MockTransport {
        responses: Mutex<VecDeque<Result<Value, String>>>,
        calls: Mutex<Vec<(String, Value)>>,
    }

    impl MockTransport {
        fn new(responses: Vec<Value>) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(responses.into_iter().map(Ok).collect()),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl ProviderTransport for MockTransport {
        async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
            self.calls.lock().await.push((method.to_string(), params));
            self.responses
                .lock()
                .await
                .pop_front()
                .expect("mock response for Provider call")
        }
    }

    fn catalog(current_provider_id: &str, providers: Value) -> Value {
        json!({
            "data": providers,
            "currentProviderId": current_provider_id,
        })
    }

    fn provider(id: &str, current: bool, models: Value) -> Value {
        json!({
            "id": id,
            "name": id,
            "baseUrl": format!("https://{id}.example/v1"),
            "envKey": null,
            "wireApi": "chat",
            "supportsFunctionTools": false,
            "kind": "custom",
            "isCurrent": current,
            "modelCount": models.as_array().map_or(0, Vec::len),
            "canEdit": true,
            "canDelete": !current,
            "canFetchModels": true,
            "models": models,
        })
    }

    #[tokio::test]
    async fn in_memory_provider_function_tools_follow_wire_api() {
        let service = InMemoryProviderService::default();
        let initial = service.list().await.expect("list default Providers");
        assert!(
            !initial
                .data
                .iter()
                .find(|provider| provider.id == "mock")
                .expect("mock Provider")
                .supports_function_tools
        );

        let created = service
            .upsert(
                "custom",
                UpsertProviderRequest {
                    name: "Custom".to_string(),
                    base_url: "https://custom.example/v1".to_string(),
                    wire_api: "chat".to_string(),
                    credentials: ProviderCredentialInput::NoCredential,
                    supports_function_tools: Some(false),
                    select: false,
                },
            )
            .await
            .expect("create Provider");
        assert!(
            created
                .data
                .iter()
                .find(|provider| provider.id == "custom")
                .expect("created Provider")
                .supports_function_tools
        );

        let responses = service
            .upsert(
                "custom",
                UpsertProviderRequest {
                    name: "Custom updated".to_string(),
                    base_url: "https://custom.example/v2".to_string(),
                    wire_api: "responses".to_string(),
                    credentials: ProviderCredentialInput::Preserve,
                    supports_function_tools: Some(true),
                    select: false,
                },
            )
            .await
            .expect("switch Provider to Responses");
        assert!(
            !responses
                .data
                .iter()
                .find(|provider| provider.id == "custom")
                .expect("Responses Provider")
                .supports_function_tools
        );

        let chat = service
            .upsert(
                "custom",
                UpsertProviderRequest {
                    name: "Custom Chat".to_string(),
                    base_url: "https://custom.example/v3".to_string(),
                    wire_api: "chat".to_string(),
                    credentials: ProviderCredentialInput::Preserve,
                    supports_function_tools: None,
                    select: false,
                },
            )
            .await
            .expect("switch Provider to Chat");
        assert!(
            chat.data
                .iter()
                .find(|provider| provider.id == "custom")
                .expect("Chat Provider")
                .supports_function_tools
        );
    }

    fn provider_catalog_failure_wire(failure: ProviderCatalogFailure) -> &'static str {
        match failure {
            ProviderCatalogFailure::Authentication => "authentication",
            ProviderCatalogFailure::NotFound => "notFound",
            ProviderCatalogFailure::RateLimited => "rateLimited",
            ProviderCatalogFailure::Upstream => "upstream",
            ProviderCatalogFailure::Timeout => "timeout",
            ProviderCatalogFailure::Network => "network",
            ProviderCatalogFailure::InvalidJson => "invalidJson",
            ProviderCatalogFailure::IncompatibleSchema => "incompatibleSchema",
            ProviderCatalogFailure::EmptyCatalog => "emptyCatalog",
        }
    }

    #[test]
    fn validates_provider_ids_and_quoted_config_paths() {
        assert_eq!(
            provider_path("deepseek-custom").unwrap(),
            "model_providers.\"deepseek-custom\""
        );
        assert!(matches!(
            provider_path("bad.id"),
            Err(ProviderServiceError::InvalidInput(_))
        ));
    }

    #[test]
    fn accepts_only_http_provider_urls() {
        assert_eq!(
            validate_base_url("https://api.example.com/v1/").unwrap(),
            "https://api.example.com/v1"
        );
        assert!(validate_base_url("file:///tmp/provider").is_err());
    }

    #[test]
    fn validates_credential_modes_without_echoing_secrets() {
        let error = validate_credentials(
            &ProviderCredentialInput::Direct {
                api_key: "".to_string(),
            },
            false,
        )
        .expect_err("empty direct key is invalid");
        assert!(!error.to_string().contains("api_key"));
        assert!(validate_credentials(&ProviderCredentialInput::Preserve, false).is_err());
        assert!(validate_credentials(
            &ProviderCredentialInput::Environment {
                env_key: "DEEPSEEK_API_KEY".to_string(),
            },
            false,
        )
        .is_ok());
    }

    #[test]
    fn model_refresh_persistence_omits_null_toml_values() {
        let model = provider_model_config_from_catalog(&ProviderModelSummary {
            model_id: "deepseek-v4-flash".to_string(),
            model_name: None,
            max_token_len: None,
            max_output_tokens: None,
            show_in_picker: true,
            context_window: None,
            supports_search_tool: false,
        })
        .expect("valid Provider model");

        assert_eq!(model["model_name"], "deepseek-v4-flash");
        assert!(!model.as_object().unwrap().contains_key("context_window"));
        assert!(!model.as_object().unwrap().values().any(Value::is_null));
    }

    #[test]
    fn model_catalog_persistence_rejects_noncanonical_ids_and_preserves_names() {
        for model_id in [" deepseek-v4-flash", "deepseek-v4-flash ", "deepseek\nv4"] {
            assert!(provider_model_config_from_catalog(&ProviderModelSummary {
                model_id: model_id.to_string(),
                model_name: Some("Display name is preserved".to_string()),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: None,
                supports_search_tool: false,
            })
            .is_none());
        }

        let persisted = provider_model_config_from_catalog(&ProviderModelSummary {
            model_id: "deepseek-v4-flash".to_string(),
            model_name: Some("Display name".to_string()),
            max_token_len: None,
            max_output_tokens: None,
            show_in_picker: true,
            context_window: None,
            supports_search_tool: false,
        })
        .expect("valid Provider model");
        assert_eq!(persisted["model_id"], "deepseek-v4-flash");
        assert_eq!(persisted["model_name"], "Display name");

        for model_name in ["", " Display name", "Display name ", "Display\nname"] {
            assert!(provider_model_config_from_catalog(&ProviderModelSummary {
                model_id: "deepseek-v4-flash".to_string(),
                model_name: Some(model_name.to_string()),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: None,
                supports_search_tool: false,
            })
            .is_none());
        }
    }

    #[test]
    fn model_context_update_preserves_other_runtime_metadata() {
        let existing = vec![
            ProviderModelSummary {
                model_id: "deepseek-v4-flash".to_string(),
                model_name: Some("DeepSeek V4 Flash".to_string()),
                max_token_len: Some(64_000),
                max_output_tokens: Some(8_192),
                show_in_picker: true,
                context_window: Some(64_000),
                supports_search_tool: false,
            },
            ProviderModelSummary {
                model_id: "deepseek-v4-pro".to_string(),
                model_name: Some("DeepSeek V4 Pro".to_string()),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: Some(128_000),
                supports_search_tool: false,
            },
        ];
        let models = update_provider_models(&existing, "deepseek-v4-flash", 128_000, Some(true));

        assert_eq!(models[0].model_name.as_deref(), Some("DeepSeek V4 Flash"));
        assert_eq!(models[0].max_token_len, Some(64_000));
        assert_eq!(models[0].max_output_tokens, Some(8_192));
        assert_eq!(models[0].context_window, Some(128_000));
        assert!(models[0].supports_search_tool);
        assert_eq!(models.len(), 2);
        assert_eq!(models[1], existing[1]);
    }

    #[test]
    fn model_metadata_update_adds_a_manually_named_model_without_a_fetched_catalog() {
        let models = update_provider_models(&[], "vendor/model-x", 200_000, None);

        assert_eq!(
            models,
            vec![ProviderModelSummary {
                model_id: "vendor/model-x".to_string(),
                model_name: Some("vendor/model-x".to_string()),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: Some(200_000),
                supports_search_tool: false,
            }]
        );
    }

    #[tokio::test]
    async fn replace_models_writes_the_complete_catalog_atomically() {
        let initial = catalog("deepseek", json!([provider("deepseek", true, json!([]))]));
        let transport = MockTransport::new(vec![
            json!({ "result": initial.clone() }),
            json!({ "result": { "status": "ok" } }),
            json!({ "result": initial }),
        ]);
        let service = ProviderService::new(transport.clone());
        let models = vec![
            ProviderModelSummary {
                model_id: "deepseek-v4-flash".to_string(),
                model_name: Some("deepseek-v4-flash".to_string()),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: Some(128_000),
                supports_search_tool: true,
            },
            ProviderModelSummary {
                model_id: "deepseek-v4-pro".to_string(),
                model_name: Some("deepseek-v4-pro".to_string()),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: Some(128_000),
                supports_search_tool: false,
            },
        ];

        let result = service
            .replace_models("deepseek", models.clone())
            .await
            .expect("replace complete model catalog");

        let calls = transport.calls.lock().await;
        assert_eq!(calls.len(), 3);
        let edit = &calls[1].1["edits"][0];
        assert_eq!(edit["keyPath"], "model_providers.\"deepseek\".models");
        assert_eq!(edit["value"].as_array().map(Vec::len), Some(2));
        assert_eq!(edit["value"][0]["model_id"], "deepseek-v4-flash");
        assert_eq!(edit["value"][0]["supports_search_tool"], true);
        assert_eq!(edit["value"][1]["model_id"], "deepseek-v4-pro");
        assert_eq!(edit["value"][1]["supports_search_tool"], false);
        assert_eq!(result.data[0].models, models);
    }

    #[tokio::test]
    async fn upsert_composes_official_methods_and_returns_no_direct_credential() {
        let initial = catalog(
            "provider-a",
            json!([provider("provider-a", true, json!([]))]),
        );
        let final_catalog = catalog(
            "provider-b",
            json!([
                provider("provider-a", false, json!([])),
                provider("provider-b", true, json!([])),
            ]),
        );
        let transport = MockTransport::new(vec![
            json!({ "result": initial }),
            json!({ "result": { "status": "ok" } }),
            json!({ "result": final_catalog }),
        ]);
        let service = ProviderService::new(transport.clone());

        let result = service
            .upsert(
                "provider-b",
                UpsertProviderRequest {
                    name: "Provider B".to_string(),
                    base_url: "https://provider-b.example/v1/".to_string(),
                    wire_api: "chat".to_string(),
                    credentials: ProviderCredentialInput::Direct {
                        api_key: "direct-secret".to_string(),
                    },
                    supports_function_tools: Some(false),
                    select: true,
                },
            )
            .await
            .expect("upsert Provider");

        let calls = transport.calls.lock().await;
        assert_eq!(
            calls.iter().map(|call| call.0.as_str()).collect::<Vec<_>>(),
            [
                "modelProvider/list",
                "config/batchWrite",
                "modelProvider/list",
            ]
        );
        assert_eq!(
            calls[1].1["edits"][0]["keyPath"],
            "model_providers.\"provider-b\""
        );
        assert_eq!(
            calls[1].1["edits"][0]["value"]["experimental_bearer_token"],
            "direct-secret"
        );
        assert_eq!(
            calls[1].1["edits"][0]["value"]["supports_function_tools"].as_bool(),
            Some(true)
        );
        assert!(!serde_json::to_string(&result)
            .expect("serialize Provider catalog")
            .contains("direct-secret"));
    }

    #[tokio::test]
    async fn refresh_preserves_current_provider_and_writes_only_target_catalog() {
        let provider_a_models = json!([{
            "modelId": "a-model",
            "modelName": "A model",
            "maxTokenLen": null,
            "maxOutputTokens": null,
            "showInPicker": true,
            "contextWindow": 32_000,
        }]);
        let initial = catalog(
            "provider-a",
            json!([
                provider("provider-a", true, provider_a_models.clone()),
                provider(
                    "provider-b",
                    false,
                    json!([{
                        "modelId": "b-model",
                        "modelName": "B model",
                        "maxTokenLen": null,
                        "maxOutputTokens": null,
                        "showInPicker": true,
                        "contextWindow": null,
                        "supportsSearchTool": true,
                    }]),
                ),
            ]),
        );
        // Codex owns Provider discovery, but its modelProvider/list projection
        // does not echo a custom Provider's freshly fetched model catalog.
        // The refresh result must therefore carry the typed catalog returned by
        // modelProvider/models/list instead of replacing it with this empty
        // discovery projection.
        let final_catalog = catalog(
            "provider-a",
            json!([
                provider("provider-a", true, provider_a_models),
                provider("provider-b", false, json!([])),
            ]),
        );
        let transport = MockTransport::new(vec![
            initial,
            json!({
                "result": {
                    "type": "success",
                    "models": [{
                        "modelId": "b-model",
                        "modelName": "B model",
                        "maxTokenLen": null,
                        "maxOutputTokens": null,
                        "showInPicker": true,
                        "contextWindow": null,
                        "supportsSearchTool": false,
                    }]
                }
            }),
            json!({ "status": "ok" }),
            final_catalog,
        ]);
        let service = ProviderService::new(transport.clone());

        let result = service
            .refresh_models("provider-b")
            .await
            .expect("refresh Provider models");

        assert_eq!(result.current_provider_id, "provider-a");
        assert_eq!(result.data[0].models[0].model_id, "a-model");
        assert_eq!(result.data[1].models[0].model_id, "b-model");
        assert!(result.data[1].models[0].supports_search_tool);
        let calls = transport.calls.lock().await;
        assert_eq!(
            calls.iter().map(|call| call.0.as_str()).collect::<Vec<_>>(),
            [
                "modelProvider/list",
                "modelProvider/models/list",
                "config/batchWrite",
                "modelProvider/list",
            ]
        );
        assert_eq!(calls[1].1, json!({ "providerId": "provider-b" }));
        assert_eq!(
            calls[2].1["edits"][0]["keyPath"],
            "model_providers.\"provider-b\".models"
        );
        assert_eq!(
            calls[2].1["edits"][0]["value"][0]["supports_search_tool"],
            true
        );
        assert!(!calls[2].1["edits"]
            .as_array()
            .expect("config edits")
            .iter()
            .any(|edit| edit["keyPath"] == "model_provider"));
    }

    #[tokio::test]
    async fn refresh_failure_matrix_has_no_config_write() {
        let failures = [
            ProviderCatalogFailure::Authentication,
            ProviderCatalogFailure::NotFound,
            ProviderCatalogFailure::RateLimited,
            ProviderCatalogFailure::Upstream,
            ProviderCatalogFailure::Timeout,
            ProviderCatalogFailure::Network,
            ProviderCatalogFailure::InvalidJson,
            ProviderCatalogFailure::IncompatibleSchema,
            ProviderCatalogFailure::EmptyCatalog,
        ];

        for failure in failures {
            let initial = catalog(
                "provider-a",
                json!([
                    provider("provider-a", true, json!([])),
                    provider(
                        "provider-b",
                        false,
                        json!([{
                            "modelId": "old-model",
                            "modelName": "Old model",
                            "maxTokenLen": null,
                            "maxOutputTokens": null,
                            "showInPicker": true,
                            "contextWindow": null,
                        }])
                    ),
                ]),
            );
            let transport = MockTransport::new(vec![
                initial,
                json!({
                    "result": {
                        "type": "failure",
                        "error": provider_catalog_failure_wire(failure),
                    }
                }),
            ]);
            let service = ProviderService::new(transport.clone());

            let error = service
                .refresh_models("provider-b")
                .await
                .expect_err("typed Provider catalog failure");
            assert!(matches!(
                error,
                ProviderServiceError::ProviderCatalogFailure(actual) if actual == failure
            ));
            let calls = transport.calls.lock().await;
            assert_eq!(
                calls.iter().map(|call| call.0.as_str()).collect::<Vec<_>>(),
                ["modelProvider/list", "modelProvider/models/list"]
            );
        }
    }

    #[tokio::test]
    async fn refresh_rejects_noncanonical_display_name_without_config_write() {
        let initial = catalog(
            "provider-a",
            json!([
                provider("provider-a", true, json!([])),
                provider(
                    "provider-b",
                    false,
                    json!([{
                        "modelId": "old-model",
                        "modelName": "Old model",
                        "maxTokenLen": null,
                        "maxOutputTokens": null,
                        "showInPicker": true,
                        "contextWindow": null,
                    }])
                ),
            ]),
        );
        let transport = MockTransport::new(vec![
            initial,
            json!({
                "result": {
                    "type": "success",
                    "models": [{
                        "modelId": "new-model",
                        "modelName": " New model",
                        "maxTokenLen": null,
                        "maxOutputTokens": null,
                        "showInPicker": true,
                        "contextWindow": null,
                    }]
                }
            }),
        ]);
        let service = ProviderService::new(transport.clone());

        let error = service
            .refresh_models("provider-b")
            .await
            .expect_err("noncanonical model name must reject the whole catalog");
        assert!(matches!(
            error,
            ProviderServiceError::ProviderCatalogFailure(
                ProviderCatalogFailure::IncompatibleSchema
            )
        ));
        let calls = transport.calls.lock().await;
        assert_eq!(
            calls.iter().map(|call| call.0.as_str()).collect::<Vec<_>>(),
            ["modelProvider/list", "modelProvider/models/list"]
        );
    }

    #[tokio::test]
    async fn refresh_rejects_malformed_catalog_shapes_without_config_write() {
        let malformed_responses = [
            Value::String("malformed catalog response".to_string()),
            json!({}),
            json!({
                "result": {
                    "type": "unknown"
                }
            }),
        ];

        for response in malformed_responses {
            let initial = catalog(
                "provider-a",
                json!([
                    provider("provider-a", true, json!([])),
                    provider(
                        "provider-b",
                        false,
                        json!([{
                            "modelId": "old-model",
                            "modelName": "Old model",
                            "maxTokenLen": null,
                            "maxOutputTokens": null,
                            "showInPicker": true,
                            "contextWindow": null,
                        }])
                    ),
                ]),
            );
            let transport = MockTransport::new(vec![initial, response]);
            let service = ProviderService::new(transport.clone());

            let error = service
                .refresh_models("provider-b")
                .await
                .expect_err("malformed catalog shape must be typed incompatible schema");
            assert!(matches!(
                error,
                ProviderServiceError::ProviderCatalogFailure(
                    ProviderCatalogFailure::IncompatibleSchema
                )
            ));
            let calls = transport.calls.lock().await;
            assert_eq!(
                calls.iter().map(|call| call.0.as_str()).collect::<Vec<_>>(),
                ["modelProvider/list", "modelProvider/models/list"]
            );
        }
    }

    #[test]
    fn typed_model_catalog_parser_rejects_empty_success_catalog() {
        let error = parse_model_provider_models_list(json!({
            "result": { "type": "success", "models": [] }
        }))
        .expect_err("empty catalog is a typed failure");
        assert!(matches!(
            error,
            ProviderServiceError::ProviderCatalogFailure(ProviderCatalogFailure::EmptyCatalog)
        ));
    }

    #[test]
    fn typed_model_catalog_parser_preserves_all_model_metadata() {
        let models = parse_model_provider_models_list(json!({
            "result": {
                "type": "success",
                "models": [{
                    "modelId": "deepseek-v4-flash",
                    "modelName": "DeepSeek V4 Flash",
                    "maxTokenLen": 64000,
                    "maxOutputTokens": 8192,
                    "showInPicker": false,
                    "contextWindow": 128000,
                }]
            }
        }))
        .expect("typed model catalog");
        assert_eq!(models.len(), 1);
        let persisted = provider_model_config_from_catalog(&models[0]).expect("persisted model");
        assert_eq!(persisted["model_id"], "deepseek-v4-flash");
        assert_eq!(persisted["model_name"], "DeepSeek V4 Flash");
        assert_eq!(persisted["max_token_len"], 64000);
        assert_eq!(persisted["max_output_tokens"], 8192);
        assert_eq!(persisted["show_in_picker"], false);
        assert_eq!(persisted["context_window"], 128000);
    }

    #[tokio::test]
    async fn model_selection_persists_provider_and_model_together() {
        let models = json!([{
            "modelId": "deepseek-v4-flash",
            "modelName": "DeepSeek V4 Flash",
            "maxTokenLen": 128_000,
            "maxOutputTokens": null,
            "showInPicker": true,
            "contextWindow": 128_000,
        }]);
        let initial = catalog(
            "provider-a",
            json!([provider("deepseek", false, models.clone())]),
        );
        let selected = catalog("deepseek", json!([provider("deepseek", true, models)]));
        let transport = MockTransport::new(vec![initial, json!({ "status": "ok" }), selected]);
        let service = ProviderService::new(transport.clone());

        let result = service
            .select_model("deepseek", "deepseek-v4-flash")
            .await
            .expect("select Provider model");

        assert_eq!(result.current_provider_id, "deepseek");
        assert_eq!(
            result.current_model_id.as_deref(),
            Some("deepseek-v4-flash")
        );
        let calls = transport.calls.lock().await;
        assert_eq!(calls[1].0, "config/batchWrite");
        assert_eq!(calls[1].1["edits"][0]["keyPath"], "model_provider");
        assert_eq!(calls[1].1["edits"][0]["value"], "deepseek");
        assert_eq!(calls[1].1["edits"][1]["keyPath"], "model");
        assert_eq!(calls[1].1["edits"][1]["value"], "deepseek-v4-flash");
    }

    #[tokio::test]
    async fn unset_reasoning_effort_receives_the_platform_default() {
        let transport = MockTransport::new(vec![
            json!({
                "config": { "model_reasoning_effort": null }
            }),
            json!({ "status": "ok" }),
        ]);
        let service = ProviderService::new(transport.clone());

        assert!(service
            .ensure_default_reasoning_effort()
            .await
            .expect("set default reasoning effort"));

        let calls = transport.calls.lock().await;
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "config/read");
        assert_eq!(calls[0].1, json!({ "includeLayers": false, "cwd": null }));
        assert_eq!(calls[1].0, "config/batchWrite");
        assert_eq!(
            calls[1].1["edits"],
            json!([{
                "keyPath": "model_reasoning_effort",
                "value": DEFAULT_REASONING_EFFORT,
                "mergeStrategy": "replace",
            }])
        );
        assert_eq!(calls[1].1["reloadUserConfig"], true);
    }

    #[tokio::test]
    async fn explicit_reasoning_effort_is_not_overwritten_by_the_platform_default() {
        let transport = MockTransport::new(vec![json!({
            "config": { "model_reasoning_effort": "high" }
        })]);
        let service = ProviderService::new(transport.clone());

        assert!(!service
            .ensure_default_reasoning_effort()
            .await
            .expect("preserve explicit reasoning effort"));

        let calls = transport.calls.lock().await;
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "config/read");
    }
}
