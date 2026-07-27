//! Provider-selection attachment for the Core model client.

use super::*;

impl ModelClient {
    pub(crate) fn uses_provider(&self, provider: &SharedModelProvider) -> bool {
        self.state.provider.info() == provider.info()
    }

    pub(crate) fn new_session_for_provider(
        &self,
        provider: &SharedModelProvider,
    ) -> ModelClientSession {
        if self.uses_provider(provider) {
            return self.new_session();
        }

        Self::new(
            provider.auth_manager(),
            self.agent_identity_policy,
            self.state.thread_id,
            provider.info().clone(),
            self.state.session_source.clone(),
            self.state.originator.clone(),
            self.state.model_verbosity,
            self.state.enable_request_compression,
            self.state.include_timing_metrics,
            self.state.beta_features_header.clone(),
            self.state.concurrent_reasoning_summaries_enabled,
            self.state.attestation_provider.clone(),
            self.http_client_factory.clone(),
        )
        .with_prompt_cache_key_override(self.prompt_cache_key_override.clone())
        .new_session()
    }
}
