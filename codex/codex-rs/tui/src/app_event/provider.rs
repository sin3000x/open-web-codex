//! Provider-specific payloads carried by the TUI application event bus.

use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;

#[derive(Debug, Clone)]
pub(crate) enum ProviderConfigAction {
    Upsert {
        id: String,
        provider: ModelProviderInfo,
    },
    FetchModelsForNewProvider {
        draft: ProviderFormDraft,
        provider: ModelProviderInfo,
    },
    Delete {
        id: String,
    },
    Use {
        id: String,
    },
    FetchModels {
        id: String,
    },
    /// Persist a context window update for a specific model under a provider.
    UpdateModelContextWindow {
        id: String,
        model_id: String,
        context_window: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProviderFormMode {
    Add,
    Edit,
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderFormDraft {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) base_url: String,
    pub(crate) env_key: String,
    pub(crate) wire_api: WireApi,
}
