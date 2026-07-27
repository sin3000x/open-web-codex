//! Third-party Chat Completions transport attachment for the Core model client.

use super::*;
use codex_api::ChatCompletionsApiRequest;
use codex_api::ChatCompletionsClient as ApiChatCompletionsClient;
use codex_api::ChatCompletionsOptions as ApiChatCompletionsOptions;
use codex_api::responses_request_to_chat_completions_request;

const CHAT_COMPLETIONS_ENDPOINT: &str = "/chat/completions";

fn session_telemetry_for_chat_request(
    session_telemetry: &SessionTelemetry,
    request: &ChatCompletionsApiRequest,
) -> SessionTelemetry {
    session_telemetry.clone().with_inference_request(
        request.service_tier.as_deref(),
        None::<&ReasoningEffortConfig>,
    )
}

impl ModelClientSession {
    async fn build_chat_completions_options(
        &self,
        responses_metadata: &CodexResponsesMetadata,
    ) -> ApiChatCompletionsOptions {
        let mut extra_headers = ApiHeaderMap::new();
        add_originator_header(&mut extra_headers, self.client.state.originator.as_str());
        if let Some(header_value) = self.client.generate_attestation_header_for().await {
            extra_headers.insert(X_OAI_ATTESTATION_HEADER, header_value);
        }
        ApiChatCompletionsOptions {
            session_id: Some(responses_metadata.session_id.to_string()),
            thread_id: Some(responses_metadata.thread_id.to_string()),
            session_source: Some(self.client.state.session_source.clone()),
            extra_headers,
        }
    }

    #[allow(clippy::too_many_arguments)]
    #[instrument(
        name = "model_client.stream_chat_completions_api",
        level = "info",
        skip_all,
        fields(
            model = %model_info.slug,
            wire_api = %self.client.state.provider.info().wire_api,
            transport = "chat_http",
            http.method = "POST",
            api.path = "chat/completions",
            turn.has_metadata_header = responses_metadata.has_turn_metadata()
        )
    )]
    pub(super) async fn stream_chat_completions_api(
        &self,
        prompt: &Prompt,
        model_info: &ModelInfo,
        session_telemetry: &SessionTelemetry,
        effort: Option<ReasoningEffortConfig>,
        summary: ReasoningSummaryConfig,
        service_tier: Option<String>,
        responses_metadata: &CodexResponsesMetadata,
        inference_trace: &InferenceTraceContext,
    ) -> Result<ResponseStream> {
        let auth_manager = self.client.state.provider.auth_manager();
        let mut auth_recovery = auth_manager
            .as_ref()
            .map(AuthManager::unauthorized_recovery);
        let mut pending_retry = PendingUnauthorizedRetry::default();
        loop {
            let client_setup = self.client.current_client_setup().await?;
            let transport = self
                .client
                .build_api_transport(&client_setup.api_provider, CHAT_COMPLETIONS_ENDPOINT)?;
            let request_auth_context = AuthRequestTelemetryContext::new(
                client_setup.auth.as_ref().map(CodexAuth::auth_mode),
                client_setup.api_auth.as_ref(),
                client_setup.agent_identity_telemetry.clone(),
                pending_retry,
            );
            let (request_telemetry, sse_telemetry) = Self::build_streaming_telemetry(
                session_telemetry,
                request_auth_context,
                RequestRouteTelemetry::for_endpoint(CHAT_COMPLETIONS_ENDPOINT),
                self.client.state.auth_env_telemetry.clone(),
            );
            let mut options = self
                .build_chat_completions_options(responses_metadata)
                .await;
            let mut responses_request = self.client.build_responses_request(
                &client_setup.api_provider,
                prompt,
                model_info,
                effort.clone(),
                summary,
                service_tier.clone(),
                responses_metadata,
            )?;
            self.client
                .prepare_response_items_for_request(&mut responses_request.input);
            let request = responses_request_to_chat_completions_request(responses_request);
            let request_session_telemetry =
                session_telemetry_for_chat_request(session_telemetry, &request);
            let inference_trace_attempt = inference_trace.start_attempt();
            inference_trace_attempt.add_request_headers(&mut options.extra_headers);
            inference_trace_attempt.record_started(&request);
            let client = ApiChatCompletionsClient::new(
                transport,
                client_setup.api_provider,
                client_setup.api_auth,
            )
            .with_telemetry(Some(request_telemetry), Some(sse_telemetry));
            let stream_result = client.stream_request(request, options).await;

            match stream_result {
                Ok(stream) => {
                    let (stream, _) = map_response_stream(
                        stream,
                        request_session_telemetry,
                        inference_trace_attempt,
                        Arc::clone(&self.client.state.provider),
                    );
                    return Ok(stream);
                }
                Err(ApiError::Transport(
                    unauthorized_transport @ TransportError::Http { status, .. },
                )) if status == StatusCode::UNAUTHORIZED => {
                    let response_debug_context =
                        extract_response_debug_context(&unauthorized_transport);
                    inference_trace_attempt.record_failed(
                        &unauthorized_transport,
                        response_debug_context.request_id.as_deref(),
                        /*output_items*/ &[],
                    );
                    pending_retry = PendingUnauthorizedRetry::from_recovery(
                        handle_unauthorized(
                            unauthorized_transport,
                            &mut auth_recovery,
                            session_telemetry,
                            &self.client.state.provider,
                        )
                        .await?,
                    );
                }
                Err(err) => {
                    let response_debug_context =
                        extract_response_debug_context_from_api_error(&err);
                    let err = self.client.state.provider.map_api_error(err);
                    inference_trace_attempt.record_failed(
                        &err,
                        response_debug_context.request_id.as_deref(),
                        /*output_items*/ &[],
                    );
                    return Err(err);
                }
            }
        }
    }
}
