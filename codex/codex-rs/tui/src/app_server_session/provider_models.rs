//! Provider-scoped model catalog refresh calls for the TUI app-server session.

use super::*;

#[derive(Clone, Copy)]
enum ModelListRefresh {
    UseCachedProviderModels,
    ForceProviderFetch,
}

impl ModelListRefresh {
    fn force_refresh(self) -> Option<bool> {
        match self {
            Self::UseCachedProviderModels => None,
            Self::ForceProviderFetch => Some(true),
        }
    }
}

impl AppServerSession {
    pub(crate) async fn fetch_available_models(&mut self) -> Result<Vec<ModelPreset>> {
        self.fetch_available_models_for(ModelListRefresh::UseCachedProviderModels)
            .await
    }

    pub(crate) async fn force_fetch_available_models(&mut self) -> Result<Vec<ModelPreset>> {
        self.fetch_available_models_for(ModelListRefresh::ForceProviderFetch)
            .await
    }

    async fn fetch_available_models_for(
        &mut self,
        refresh: ModelListRefresh,
    ) -> Result<Vec<ModelPreset>> {
        let model_request_id = self.next_request_id();
        let models: ModelListResponse = self
            .client
            .request_typed(ClientRequest::ModelList {
                request_id: model_request_id,
                params: ModelListParams {
                    cursor: None,
                    limit: None,
                    include_hidden: Some(true),
                    force_refresh: refresh.force_refresh(),
                },
            })
            .await
            .wrap_err("model/list failed while refreshing provider models")?;
        let available_models = models
            .data
            .into_iter()
            .map(model_preset_from_api_model)
            .collect::<Vec<_>>();
        self.available_models = available_models.clone();
        Ok(available_models)
    }
}
