//! Provider configuration and model-catalog actions for the TUI.
//!
//! This module isolates the retained Provider workflow from the high-churn app event dispatcher.

use super::super::*;
use crate::app_event::ProviderFormMode;
use std::sync::Arc;

enum ModelCatalogRefresh<'a> {
    CurrentProvider {
        app_server: &'a mut AppServerSession,
    },
    FetchAndPersistProvider {
        app_server: &'a mut AppServerSession,
        provider_id: &'a str,
    },
}

impl App {
    pub(super) async fn handle_provider_config_action(
        &mut self,
        app_server: &mut AppServerSession,
        action: crate::app_event::ProviderConfigAction,
    ) {
        let builtin_ids = codex_model_provider_info::built_in_model_providers(None);
        enum ProviderPostSaveAction {
            None,
            RefreshCurrentProvider,
            FetchAndOpenModels { provider_id: String },
        }

        let action = match action {
            crate::app_event::ProviderConfigAction::FetchModelsForNewProvider {
                draft,
                provider,
            } => {
                let id = draft.id.clone();
                if builtin_ids.contains_key(&id) {
                    self.chat_widget.add_error_message(format!(
                        "Built-in provider '{id}' cannot be edited here."
                    ));
                    self.chat_widget.dismiss_provider_form();
                    self.chat_widget
                        .open_provider_form(ProviderFormMode::Add, draft);
                    return;
                }
                if self.config.model_providers.contains_key(&id) {
                    self.chat_widget.add_error_message(format!(
                        "Provider '{id}' already exists. Choose a different id."
                    ));
                    self.chat_widget.dismiss_provider_form();
                    self.chat_widget
                        .open_provider_form(ProviderFormMode::Add, draft);
                    return;
                }
                if let Err(err) = provider.validate() {
                    self.chat_widget
                        .add_error_message(format!("Invalid provider '{id}': {err}"));
                    self.chat_widget.dismiss_provider_form();
                    self.chat_widget
                        .open_provider_form(ProviderFormMode::Add, draft);
                    return;
                }

                let models = match codex_model_provider::fetch_provider_models(
                    provider.clone(),
                    /*auth_manager*/ None,
                )
                .await
                {
                    Ok(models) if !models.is_empty() => models,
                    Ok(_) => {
                        self.chat_widget
                            .add_error_message(format!("Provider '{id}' returned no models."));
                        self.chat_widget.dismiss_provider_form();
                        self.chat_widget
                            .open_provider_form(ProviderFormMode::Add, draft);
                        return;
                    }
                    Err(err) => {
                        self.chat_widget.add_error_message(format!(
                        "Failed to fetch models for provider '{id}': {err}. Check the form values and try again."
                    ));
                        self.chat_widget.dismiss_provider_form();
                        self.chat_widget
                            .open_provider_form(ProviderFormMode::Add, draft);
                        return;
                    }
                };

                let provider_models = models
                    .into_iter()
                    .map(codex_protocol::openai_models::ModelPreset::from)
                    .map(|model| codex_model_provider_info::ProviderModelInfo::from(&model))
                    .collect::<Vec<_>>();
                let mut provider = provider;
                provider.models = provider_models.clone();

                let edit = match crate::config_update::build_model_provider_edit(&id, &provider) {
                    Ok(edit) => edit,
                    Err(err) => {
                        self.chat_widget.add_error_message(format!(
                            "Failed to serialize provider '{id}': {err}"
                        ));
                        self.chat_widget.dismiss_provider_form();
                        self.chat_widget
                            .open_provider_form(ProviderFormMode::Add, draft);
                        return;
                    }
                };
                let edits = vec![
                    edit,
                    crate::config_update::build_model_provider_selection_edit(&id),
                    crate::config_update::build_model_provider_models_edit(&id, &provider_models),
                ];

                match crate::config_update::write_config_batch(app_server.request_handle(), edits)
                    .await
                {
                    Ok(_) => {
                        self.refresh_in_memory_config_from_disk_best_effort("adding provider")
                            .await;
                        if self
                            .refresh_model_catalog_from_app_server(
                                ModelCatalogRefresh::CurrentProvider { app_server },
                            )
                            .await
                        {
                            self.chat_widget.dismiss_provider_form();
                            self.chat_widget.open_model_popup();
                        } else {
                            self.chat_widget.open_provider_manager();
                        }
                    }
                    Err(err) => {
                        let error = crate::config_update::format_config_error(&err);
                        self.chat_widget.add_error_message(format!(
                            "Fetched models, but failed to save provider '{id}': {error}"
                        ));
                        self.chat_widget.dismiss_provider_form();
                        self.chat_widget
                            .open_provider_form(ProviderFormMode::Add, draft);
                    }
                }
                return;
            }
            action => action,
        };

        let (edits, success_message, post_save_action) = match action {
            crate::app_event::ProviderConfigAction::Upsert { id, provider } => {
                if builtin_ids.contains_key(&id) {
                    self.chat_widget.add_error_message(format!(
                        "Built-in provider '{id}' cannot be edited here."
                    ));
                    return;
                }
                let edit = match crate::config_update::build_model_provider_edit(&id, &provider) {
                    Ok(edit) => edit,
                    Err(err) => {
                        self.chat_widget.add_error_message(format!(
                            "Failed to serialize provider '{id}': {err}"
                        ));
                        return;
                    }
                };
                let provider_is_new = !self.config.model_providers.contains_key(&id);
                let mut edits = vec![edit];
                let post_save_action = if provider_is_new {
                    edits.push(crate::config_update::build_model_provider_selection_edit(
                        &id,
                    ));
                    ProviderPostSaveAction::FetchAndOpenModels {
                        provider_id: id.clone(),
                    }
                } else if self.config.model_provider_id == id {
                    ProviderPostSaveAction::RefreshCurrentProvider
                } else {
                    ProviderPostSaveAction::None
                };
                let success_message = if provider_is_new {
                    tracing::info!("Saved and selected provider '{id}'.");
                    String::new()
                } else {
                    tracing::info!("Saved provider '{id}'.");
                    String::new()
                };
                (edits, success_message, post_save_action)
            }
            crate::app_event::ProviderConfigAction::FetchModelsForNewProvider { .. } => {
                unreachable!("new provider model fetch is handled before config edits are built")
            }
            crate::app_event::ProviderConfigAction::Delete { id } => {
                if builtin_ids.contains_key(&id) {
                    self.chat_widget
                        .add_error_message(format!("Built-in provider '{id}' cannot be deleted."));
                    return;
                }
                if !self.config.model_providers.contains_key(&id) {
                    self.chat_widget
                        .add_error_message(format!("Provider '{id}' does not exist."));
                    return;
                }
                if self.config.model_provider_id == id {
                    self.chat_widget.add_error_message(format!(
                        "Provider '{id}' is selected. Choose another provider before deleting it."
                    ));
                    return;
                }
                (
                    vec![crate::config_update::build_model_provider_delete_edit(&id)],
                    String::new(),
                    ProviderPostSaveAction::None,
                )
            }
            crate::app_event::ProviderConfigAction::Use { id } => {
                if !self.config.model_providers.contains_key(&id) {
                    self.chat_widget
                        .add_error_message(format!("Provider '{id}' does not exist."));
                    return;
                }
                (
                    vec![crate::config_update::build_model_provider_selection_edit(
                        &id,
                    )],
                    String::new(),
                    ProviderPostSaveAction::RefreshCurrentProvider,
                )
            }
            crate::app_event::ProviderConfigAction::FetchModels { id } => {
                if builtin_ids.contains_key(&id) {
                    self.chat_widget.add_error_message(format!(
                        "Built-in provider '{id}' uses Codex's built-in model catalog."
                    ));
                    return;
                }
                if !self.config.model_providers.contains_key(&id) {
                    self.chat_widget
                        .add_error_message(format!("Provider '{id}' does not exist."));
                    return;
                }
                (
                    vec![crate::config_update::build_model_provider_selection_edit(
                        &id,
                    )],
                    String::new(),
                    ProviderPostSaveAction::FetchAndOpenModels { provider_id: id },
                )
            }
            crate::app_event::ProviderConfigAction::UpdateModelContextWindow {
                id,
                model_id,
                context_window,
            } => {
                if !self.config.model_providers.contains_key(&id) {
                    self.chat_widget
                        .add_error_message(format!("Provider '{id}' does not exist."));
                    return;
                }
                let Some(provider) = self.config.model_providers.get(&id) else {
                    return;
                };
                let mut models = provider.models.clone();
                if let Some(model) = models.iter_mut().find(|m| m.model_id == model_id) {
                    model.context_window = Some(context_window);
                }
                (
                    vec![crate::config_update::build_model_provider_models_edit(
                        &id, &models,
                    )],
                    String::new(),
                    ProviderPostSaveAction::RefreshCurrentProvider,
                )
            }
        };

        match crate::config_update::write_config_batch(app_server.request_handle(), edits).await {
            Ok(_) => {
                self.refresh_in_memory_config_from_disk_best_effort("updating providers")
                    .await;
                if !success_message.is_empty() {
                    self.chat_widget.add_info_message(success_message, None);
                }
                let ref_succeeded = match &post_save_action {
                    ProviderPostSaveAction::None => false,
                    ProviderPostSaveAction::RefreshCurrentProvider => {
                        self.refresh_model_catalog_from_app_server(
                            ModelCatalogRefresh::CurrentProvider { app_server },
                        )
                        .await
                    }
                    ProviderPostSaveAction::FetchAndOpenModels { provider_id } => {
                        self.refresh_model_catalog_from_app_server(
                            ModelCatalogRefresh::FetchAndPersistProvider {
                                app_server,
                                provider_id: provider_id.as_str(),
                            },
                        )
                        .await
                    }
                };
                match (post_save_action, ref_succeeded) {
                    (ProviderPostSaveAction::FetchAndOpenModels { .. }, true) => {
                        self.chat_widget.open_model_popup();
                    }
                    (ProviderPostSaveAction::RefreshCurrentProvider, true) => {
                        // Context window updated and models refreshed — no navigation needed.
                    }
                    (ProviderPostSaveAction::None, _)
                    | (ProviderPostSaveAction::RefreshCurrentProvider, false)
                    | (ProviderPostSaveAction::FetchAndOpenModels { .. }, false) => {
                        self.chat_widget.open_provider_manager();
                    }
                }
            }
            Err(err) => {
                let error = crate::config_update::format_config_error(&err);
                self.chat_widget
                    .add_error_message(format!("Failed to save provider configuration: {error}"));
            }
        }
    }

    async fn refresh_model_catalog_from_app_server(
        &mut self,
        refresh: ModelCatalogRefresh<'_>,
    ) -> bool {
        let (app_server, persist_provider_id, fetch_result) = match refresh {
            ModelCatalogRefresh::CurrentProvider { app_server } => {
                let fetch_result = app_server.fetch_available_models().await;
                (app_server, None, fetch_result)
            }
            ModelCatalogRefresh::FetchAndPersistProvider {
                app_server,
                provider_id,
            } => {
                let fetch_result = app_server.force_fetch_available_models().await;
                (app_server, Some(provider_id), fetch_result)
            }
        };

        match fetch_result {
            Ok(available_models) => {
                let model_count = available_models.len();
                if let Some(provider_id) = persist_provider_id {
                    let provider_models = available_models
                        .iter()
                        .map(codex_model_provider_info::ProviderModelInfo::from)
                        .collect::<Vec<_>>();
                    let edit = crate::config_update::build_model_provider_models_edit(
                        provider_id,
                        &provider_models,
                    );
                    if let Err(err) = crate::config_update::write_config_batch(
                        app_server.request_handle(),
                        vec![edit],
                    )
                    .await
                    {
                        let error = crate::config_update::format_config_error(&err);
                        self.chat_widget.add_error_message(format!(
                            "Fetched models, but failed to save provider models: {error}"
                        ));
                        return false;
                    }
                    self.refresh_in_memory_config_from_disk_best_effort("saving provider models")
                        .await;
                }
                let model_catalog = Arc::new(ModelCatalog::new(available_models));
                self.model_catalog = model_catalog.clone();
                self.chat_widget.set_model_catalog(model_catalog);
                tracing::info!(
                    "Loaded {model_count} models for provider '{}'.",
                    self.config.model_provider_id
                );
                true
            }
            Err(err) => {
                self.chat_widget.add_error_message(format!(
                    "Provider saved, but failed to refresh models: {err:#}"
                ));
                false
            }
        }
    }
}
