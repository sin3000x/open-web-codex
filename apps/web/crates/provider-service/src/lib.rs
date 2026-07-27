//! Platform-owned Provider CRUD orchestration.
//!
//! Codex remains responsible for Provider metadata, model discovery/cache and
//! transport semantics. This service validates authorized platform inputs and
//! composes the typed app-server Provider/config methods.

pub mod secured;

use std::sync::Arc;

use async_trait::async_trait;
use open_web_codex_platform_contracts::{
    ProviderCatalog, ProviderCredentialInput, ProviderKind, ProviderModelSummary, ProviderSummary,
    UpdateProviderModelRequest, UpsertProviderRequest,
};
use open_web_codex_profile_host::ProfileHost;
use serde_json::{json, Map, Value};
use thiserror::Error;
use tokio::sync::RwLock;
use url::Url;

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
        validate_credentials(&request.credentials, existing.is_some())?;

        let provider_path = provider_path(id)?;
        let mut edits = Vec::new();
        if existing.is_none() {
            let mut provider = Map::new();
            provider.insert("name".to_string(), json!(name));
            provider.insert("base_url".to_string(), json!(base_url));
            provider.insert("wire_api".to_string(), json!(request.wire_api));
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

        if catalog.current_provider_id != id {
            self.write_config(vec![config_edit("model_provider".to_string(), json!(id))])
                .await?;
        }
        let response = self
            .transport
            .request("model/list", json!({ "forceRefresh": true }))
            .await
            .map_err(ProviderServiceError::Runtime)?;
        let models = model_list_data(&response);
        if models.is_empty() {
            return Err(ProviderServiceError::Runtime(format!(
                "Provider '{id}' returned no models"
            )));
        }
        let persisted_models = models
            .iter()
            .filter_map(provider_model_config_from_catalog)
            .collect::<Vec<_>>();
        self.write_config(vec![config_edit(
            format!("{}.models", provider_path(id)?),
            json!(persisted_models),
        )])
        .await?;
        self.list().await
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
        let models =
            upsert_provider_model_context(&provider.models, model_id, request.context_window);
        self.write_config(vec![config_edit(
            format!("{}.models", provider_path(provider_id)?),
            json!(models),
        )])
        .await?;
        self.list().await
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
        };
        let mock_model = ProviderModelSummary {
            model_id: "mock-codex".to_string(),
            model_name: Some("Mock Codex".to_string()),
            max_token_len: Some(64_000),
            max_output_tokens: Some(8_192),
            show_in_picker: true,
            context_window: Some(64_000),
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
        } else {
            catalog.data.push(ProviderSummary {
                id: id.to_string(),
                name,
                base_url: Some(base_url),
                env_key,
                wire_api: request.wire_api,
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
        let mut catalog = self.catalog.write().await;
        let index = require_catalog_provider(&catalog, id)?;
        if !catalog.data[index].can_fetch_models {
            return Err(ProviderServiceError::Forbidden(format!(
                "Provider '{id}' uses a bundled model catalog"
            )));
        }
        if catalog.data[index].models.is_empty() {
            catalog.data[index].models.push(ProviderModelSummary {
                model_id: format!("{id}-model"),
                model_name: Some(format!("{id} model")),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: None,
            });
            catalog.data[index].model_count = 1;
        }
        select_catalog_provider(&mut catalog, id);
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
        } else {
            provider.models.push(ProviderModelSummary {
                model_id: model_id.to_string(),
                model_name: Some(model_id.to_string()),
                max_token_len: None,
                max_output_tokens: None,
                show_in_picker: true,
                context_window: Some(request.context_window),
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

fn model_list_data(response: &Value) -> Vec<Value> {
    unwrap_result(response)
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn provider_model_config_from_catalog(value: &Value) -> Option<Value> {
    let model = value.as_object()?;
    let model_id = model
        .get("model")
        .or_else(|| model.get("id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let mut persisted = Map::new();
    persisted.insert("model_id".to_string(), json!(model_id));
    persisted.insert(
        "model_name".to_string(),
        model
            .get("displayName")
            .or_else(|| model.get("display_name"))
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| json!(model_id)),
    );
    persisted.insert("show_in_picker".to_string(), json!(true));
    if let Some(context_window) = model
        .get("contextWindow")
        .or_else(|| model.get("context_window"))
        .filter(|value| !value.is_null())
    {
        persisted.insert("context_window".to_string(), context_window.clone());
    }
    Some(Value::Object(persisted))
}

fn upsert_provider_model_context(
    raw_models: &[ProviderModelSummary],
    model_id: &str,
    context_window: i64,
) -> Vec<Value> {
    let mut found = false;
    let mut models = raw_models
        .iter()
        .map(|model| {
            let next_context = if model.model_id == model_id {
                found = true;
                Some(context_window)
            } else {
                model.context_window
            };
            let mut persisted = Map::new();
            persisted.insert("model_id".to_string(), json!(model.model_id));
            persisted.insert(
                "model_name".to_string(),
                json!(model.model_name.as_deref().unwrap_or(&model.model_id)),
            );
            persisted.insert("show_in_picker".to_string(), json!(model.show_in_picker));
            if let Some(value) = model.max_token_len {
                persisted.insert("max_token_len".to_string(), json!(value));
            }
            if let Some(value) = model.max_output_tokens {
                persisted.insert("max_output_tokens".to_string(), json!(value));
            }
            if let Some(value) = next_context {
                persisted.insert("context_window".to_string(), json!(value));
            }
            Value::Object(persisted)
        })
        .collect::<Vec<_>>();
    if !found {
        models.push(json!({
            "model_id": model_id,
            "model_name": model_id,
            "show_in_picker": true,
            "context_window": context_window,
        }));
    }
    models
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Arc;

    use super::{
        provider_model_config_from_catalog, provider_path, upsert_provider_model_context,
        validate_base_url, validate_credentials, ProviderService, ProviderServiceError,
        ProviderTransport,
    };
    use async_trait::async_trait;
    use open_web_codex_platform_contracts::{
        ProviderCredentialInput, ProviderModelSummary, UpsertProviderRequest,
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
            "kind": "custom",
            "isCurrent": current,
            "modelCount": models.as_array().map_or(0, Vec::len),
            "canEdit": true,
            "canDelete": !current,
            "canFetchModels": true,
            "models": models,
        })
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
        let model = provider_model_config_from_catalog(&json!({
            "model": "deepseek-v4-flash",
            "displayName": null,
            "contextWindow": null,
        }))
        .unwrap();

        assert_eq!(model["model_name"], "deepseek-v4-flash");
        assert!(!model.as_object().unwrap().contains_key("context_window"));
        assert!(!model.as_object().unwrap().values().any(Value::is_null));
    }

    #[test]
    fn model_context_update_preserves_other_runtime_metadata() {
        let existing = vec![ProviderModelSummary {
            model_id: "deepseek-v4-flash".to_string(),
            model_name: Some("DeepSeek V4 Flash".to_string()),
            max_token_len: Some(64_000),
            max_output_tokens: Some(8_192),
            show_in_picker: true,
            context_window: Some(64_000),
        }];
        let models = upsert_provider_model_context(&existing, "deepseek-v4-flash", 128_000);

        assert_eq!(models[0]["model_name"], "DeepSeek V4 Flash");
        assert_eq!(models[0]["max_token_len"], 64_000);
        assert_eq!(models[0]["max_output_tokens"], 8_192);
        assert_eq!(models[0]["context_window"], 128_000);
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
        assert!(!serde_json::to_string(&result)
            .expect("serialize Provider catalog")
            .contains("direct-secret"));
    }

    #[tokio::test]
    async fn refresh_updates_only_the_selected_provider_catalog() {
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
                provider("provider-b", false, json!([])),
            ]),
        );
        let final_catalog = catalog(
            "provider-b",
            json!([
                provider("provider-a", false, provider_a_models),
                provider(
                    "provider-b",
                    true,
                    json!([{
                        "modelId": "b-model",
                        "modelName": "B model",
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
            json!({ "status": "ok" }),
            json!({ "data": [{ "model": "b-model", "displayName": "B model" }] }),
            json!({ "status": "ok" }),
            final_catalog,
        ]);
        let service = ProviderService::new(transport.clone());

        let result = service
            .refresh_models("provider-b")
            .await
            .expect("refresh Provider models");

        assert_eq!(result.current_provider_id, "provider-b");
        assert_eq!(result.data[0].models[0].model_id, "a-model");
        assert_eq!(result.data[1].models[0].model_id, "b-model");
        let calls = transport.calls.lock().await;
        assert_eq!(calls[2].0, "model/list");
        assert_eq!(calls[2].1, json!({ "forceRefresh": true }));
        assert_eq!(
            calls[3].1["edits"][0]["keyPath"],
            "model_providers.\"provider-b\".models"
        );
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
}
