use crate::auth::SharedAuthProvider;
use crate::endpoint::session::EndpointSession;
use crate::error::ApiError;
use crate::provider::Provider;
use codex_client::HttpTransport;
use codex_client::RequestTelemetry;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::TruncationPolicyConfig;
use codex_protocol::openai_models::WebSearchToolType;
use codex_protocol::openai_models::default_input_modalities;
use http::HeaderMap;
use http::Method;
use http::header::ETAG;
use std::sync::Arc;

#[derive(serde::Deserialize)]
struct OpenAiModelEntry {
    id: String,
}

impl From<OpenAiModelEntry> for ModelInfo {
    fn from(entry: OpenAiModelEntry) -> Self {
        Self {
            slug: entry.id.clone(),
            display_name: entry.id,
            description: None,
            default_reasoning_level: Some(ReasoningEffort::None),
            supported_reasoning_levels: Vec::new(),
            shell_type: ConfigShellToolType::ShellCommand,
            visibility: ModelVisibility::List,
            supported_in_api: true,
            priority: 0,
            additional_speed_tiers: Vec::new(),
            service_tiers: Vec::new(),
            default_service_tier: None,
            availability_nux: None,
            upgrade: None,
            base_instructions: "base instructions".to_string(),
            model_messages: None,
            include_skills_usage_instructions: false,
            supports_reasoning_summary_parameter: false,
            default_reasoning_summary: ReasoningSummary::Auto,
            support_verbosity: false,
            default_verbosity: None,
            apply_patch_tool_type: None,
            web_search_tool_type: WebSearchToolType::Text,
            truncation_policy: TruncationPolicyConfig::tokens(128_000),
            supports_parallel_tool_calls: false,
            supports_image_detail_original: false,
            context_window: None,
            max_context_window: None,
            auto_compact_token_limit: None,
            comp_hash: None,
            effective_context_window_percent: 95,
            experimental_supported_tools: Vec::new(),
            input_modalities: default_input_modalities(),
            used_fallback_model_metadata: false,
            supports_search_tool: false,
            use_responses_lite: false,
            auto_review_model_override: None,
            tool_mode: None,
            multi_agent_version: None,
        }
    }
}

/// Client for fetching model lists from an OpenAI-compatible `/models` endpoint.
///
/// Supports both the Codex-native response format (`{"models": [...]}`) and
/// the standard OpenAI response format (`{"data": [...]}`).
pub struct ModelsClient<T: HttpTransport> {
    session: EndpointSession<T>,
    use_openai_models_format: bool,
}

impl<T: HttpTransport> ModelsClient<T> {
    pub fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            session: EndpointSession::new(transport, provider, auth),
            use_openai_models_format: false,
        }
    }

    pub fn with_telemetry(self, request: Option<Arc<dyn RequestTelemetry>>) -> Self {
        Self {
            session: self.session.with_request_telemetry(request),
            ..self
        }
    }

    /// When set, the client parses the standard OpenAI `/v1/models` response
    /// format (`{"data": [{"id": "...", "object": "model", ...}]}`) instead of
    /// the Codex-native format (`{"models": [...]}`).
    ///
    /// This should be set for third-party providers that speak the Chat
    /// Completions API (`WireApi::Chat`), as they typically expose the
    /// standard OpenAI models endpoint.
    pub fn with_openai_models_format(self, use_openai: bool) -> Self {
        Self {
            use_openai_models_format: use_openai,
            ..self
        }
    }

    fn path() -> &'static str {
        "models"
    }

    fn append_client_version_query(req: &mut codex_client::Request, client_version: &str) {
        let separator = if req.url.contains('?') { '&' } else { '?' };
        req.url = format!("{}{}client_version={client_version}", req.url, separator);
    }

    pub fn request_url(provider: &Provider, client_version: &str) -> String {
        let mut request = provider.build_request(Method::GET, Self::path());
        Self::append_client_version_query(&mut request, client_version);
        request.url
    }

    pub async fn list_models(
        &self,
        request_url: String,
        extra_headers: HeaderMap,
    ) -> Result<(Vec<ModelInfo>, Option<String>), ApiError> {
        let resp = self
            .session
            .execute_with(
                Method::GET,
                Self::path(),
                extra_headers,
                /*body*/ None,
                move |req| {
                    req.url.clone_from(&request_url);
                },
            )
            .await?;

        let header_etag = resp
            .headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(ToString::to_string);

        let models = if self.use_openai_models_format {
            #[derive(serde::Deserialize)]
            struct OpenAiModelsData {
                data: Vec<OpenAiModelEntry>,
            }
            let data: OpenAiModelsData = serde_json::from_slice(&resp.body).map_err(|e| {
                ApiError::Stream(format!(
                    "failed to decode OpenAI /v1/models response: {e}; body: {}",
                    String::from_utf8_lossy(&resp.body)
                ))
            })?;
            data.data.into_iter().map(ModelInfo::from).collect()
        } else {
            let ModelsResponse { models } = serde_json::from_slice::<ModelsResponse>(&resp.body)
                .map_err(|e| {
                    ApiError::Stream(format!(
                        "failed to decode models response: {e}; body: {}",
                        String::from_utf8_lossy(&resp.body)
                    ))
                })?;
            models
        };

        Ok((models, header_etag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::AuthProvider;
    use crate::provider::RetryConfig;
    use codex_client::Request;
    use codex_client::Response;
    use codex_client::StreamResponse;
    use codex_client::TransportError;
    use http::HeaderMap;
    use http::StatusCode;
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::time::Duration;

    #[derive(Clone)]
    struct CapturingTransport {
        last_request: Arc<Mutex<Option<Request>>>,
        body: Arc<Vec<u8>>,
        etag: Option<String>,
    }

    impl Default for CapturingTransport {
        fn default() -> Self {
            Self {
                last_request: Arc::new(Mutex::new(None)),
                body: Arc::new(serde_json::to_vec(&ModelsResponse { models: Vec::new() }).unwrap()),
                etag: None,
            }
        }
    }

    impl HttpTransport for CapturingTransport {
        async fn execute(&self, req: Request) -> Result<Response, TransportError> {
            *self.last_request.lock().unwrap() = Some(req);
            let mut headers = HeaderMap::new();
            if let Some(etag) = &self.etag {
                headers.insert(ETAG, etag.parse().unwrap());
            }
            Ok(Response {
                status: StatusCode::OK,
                headers,
                body: self.body.as_ref().clone().into(),
            })
        }

        async fn stream(&self, _req: Request) -> Result<StreamResponse, TransportError> {
            Err(TransportError::Build("stream should not run".to_string()))
        }
    }

    #[derive(Clone, Default)]
    struct DummyAuth;

    impl AuthProvider for DummyAuth {
        fn add_auth_headers(&self, _headers: &mut HeaderMap) {}
    }

    fn provider(base_url: &str) -> Provider {
        Provider {
            name: "test".to_string(),
            base_url: base_url.to_string(),
            query_params: None,
            headers: HeaderMap::new(),
            retry: RetryConfig {
                max_attempts: 1,
                base_delay: Duration::from_millis(1),
                retry_429: false,
                retry_5xx: true,
                retry_transport: true,
            },
            stream_idle_timeout: Duration::from_secs(1),
        }
    }

    #[tokio::test]
    async fn appends_client_version_query() {
        let response = ModelsResponse { models: Vec::new() };

        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(serde_json::to_vec(&response).unwrap()),
            etag: None,
        };

        let provider = provider("https://example.com/api/codex");
        let request_url = ModelsClient::<CapturingTransport>::request_url(&provider, "0.99.0");
        let client = ModelsClient::new(transport.clone(), provider, Arc::new(DummyAuth));

        let (models, _) = client
            .list_models(request_url, HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 0);

        let url = transport
            .last_request
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .url
            .clone();
        assert_eq!(
            url,
            "https://example.com/api/codex/models?client_version=0.99.0"
        );
    }

    #[tokio::test]
    async fn parses_models_response() {
        let response = ModelsResponse {
            models: vec![
                serde_json::from_value(json!({
                    "slug": "gpt-test",
                    "display_name": "gpt-test",
                    "description": "desc",
                    "default_reasoning_level": "medium",
                    "supported_reasoning_levels": [{"effort": "low", "description": "low"}, {"effort": "medium", "description": "medium"}, {"effort": "high", "description": "high"}],
                    "shell_type": "shell_command",
                    "visibility": "list",
                    "minimal_client_version": [0, 99, 0],
                    "supported_in_api": true,
                    "priority": 1,
                    "upgrade": null,
                    "base_instructions": "base instructions",
                    "support_verbosity": false,
                    "default_verbosity": null,
                    "apply_patch_tool_type": null,
                    "truncation_policy": {"mode": "bytes", "limit": 10_000},
                    "supports_parallel_tool_calls": false,
                    "supports_image_detail_original": false,
                    "context_window": 272_000,
                    "experimental_supported_tools": [],
                }))
                .unwrap(),
            ],
        };

        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(serde_json::to_vec(&response).unwrap()),
            etag: None,
        };

        let provider = provider("https://example.com/api/codex");
        let request_url = ModelsClient::<CapturingTransport>::request_url(&provider, "0.99.0");
        let client = ModelsClient::new(transport, provider, Arc::new(DummyAuth));

        let (models, _) = client
            .list_models(request_url, HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].slug, "gpt-test");
        assert_eq!(models[0].supported_in_api, true);
        assert_eq!(models[0].priority, 1);
    }

    #[tokio::test]
    async fn parses_openai_models_response_without_protocol_owned_dto() {
        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(
                serde_json::to_vec(&json!({
                    "object": "list",
                    "data": [{
                        "id": "deepseek-chat",
                        "object": "model",
                        "created": 1_700_000_000,
                        "owned_by": "deepseek"
                    }]
                }))
                .unwrap(),
            ),
            etag: None,
        };

        let provider = provider("https://example.com/v1");
        let request_url = ModelsClient::<CapturingTransport>::request_url(&provider, "0.99.0");
        let client = ModelsClient::new(transport, provider, Arc::new(DummyAuth))
            .with_openai_models_format(true);

        let (models, _) = client
            .list_models(request_url, HeaderMap::new())
            .await
            .expect("OpenAI-compatible models request should succeed");

        assert_eq!(
            models,
            vec![ModelInfo {
                slug: "deepseek-chat".to_string(),
                display_name: "deepseek-chat".to_string(),
                description: None,
                default_reasoning_level: Some(ReasoningEffort::None),
                supported_reasoning_levels: Vec::new(),
                shell_type: ConfigShellToolType::ShellCommand,
                visibility: ModelVisibility::List,
                supported_in_api: true,
                priority: 0,
                additional_speed_tiers: Vec::new(),
                service_tiers: Vec::new(),
                default_service_tier: None,
                availability_nux: None,
                upgrade: None,
                base_instructions: "base instructions".to_string(),
                model_messages: None,
                include_skills_usage_instructions: false,
                supports_reasoning_summary_parameter: false,
                default_reasoning_summary: ReasoningSummary::Auto,
                support_verbosity: false,
                default_verbosity: None,
                apply_patch_tool_type: None,
                web_search_tool_type: WebSearchToolType::Text,
                truncation_policy: TruncationPolicyConfig::tokens(128_000),
                supports_parallel_tool_calls: false,
                supports_image_detail_original: false,
                context_window: None,
                max_context_window: None,
                auto_compact_token_limit: None,
                comp_hash: None,
                effective_context_window_percent: 95,
                experimental_supported_tools: Vec::new(),
                input_modalities: default_input_modalities(),
                used_fallback_model_metadata: false,
                supports_search_tool: false,
                use_responses_lite: false,
                auto_review_model_override: None,
                tool_mode: None,
                multi_agent_version: None,
            }]
        );
    }

    #[tokio::test]
    async fn list_models_includes_etag() {
        let response = ModelsResponse { models: Vec::new() };

        let transport = CapturingTransport {
            last_request: Arc::new(Mutex::new(None)),
            body: Arc::new(serde_json::to_vec(&response).unwrap()),
            etag: Some("\"abc\"".to_string()),
        };

        let provider = provider("https://example.com/api/codex");
        let request_url = ModelsClient::<CapturingTransport>::request_url(&provider, "0.1.0");
        let client = ModelsClient::new(transport, provider, Arc::new(DummyAuth));

        let (models, etag) = client
            .list_models(request_url, HeaderMap::new())
            .await
            .expect("request should succeed");

        assert_eq!(models.len(), 0);
        assert_eq!(etag, Some("\"abc\"".to_string()));
    }
}
