//! Provider-scoped model metadata editing for the TUI model picker.

use super::*;

#[derive(Debug, Clone)]
pub(crate) struct PendingModelSelection {
    pub(crate) model: String,
    pub(crate) effort: Option<ReasoningEffortConfig>,
}

impl ChatWidget {
    pub(crate) fn open_model_context_window_popup(
        &mut self,
        model_id: &str,
        provider_id: &str,
        pending_selection: Option<PendingModelSelection>,
    ) {
        let Some(provider) = self.config.model_providers.get(provider_id).cloned() else {
            self.add_error_message(format!("Provider '{provider_id}' not found."));
            return;
        };
        let current_context_window = provider
            .models
            .iter()
            .find(|model| model.model_id == model_id)
            .and_then(|model| model.context_window)
            .unwrap_or(262_144);
        let tx = self.app_event_tx.clone();
        let provider_id = provider_id.to_string();
        let model_id = model_id.to_string();
        let view = CustomPromptView::new(
            format!("Context window for {model_id}"),
            format!("Number of tokens (current: {current_context_window})"),
            current_context_window.to_string(),
            Some(
                "Context window limit for this model in tokens; higher means longer conversations."
                    .to_string(),
            ),
            Box::new(move |value: String| {
                let parsed = value
                    .trim()
                    .parse::<i64>()
                    .unwrap_or(current_context_window);
                tx.send(AppEvent::ProviderConfigAction {
                    action: crate::app_event::ProviderConfigAction::UpdateModelContextWindow {
                        id: provider_id.clone(),
                        model_id: model_id.clone(),
                        context_window: parsed,
                    },
                });
                if let Some(selection) = pending_selection.as_ref() {
                    tx.send(AppEvent::UpdateModel(selection.model.clone()));
                    if let Some(effort) = selection.effort.as_ref() {
                        tx.send(AppEvent::UpdateReasoningEffort(Some(effort.clone())));
                    }
                    tx.send(AppEvent::PersistModelSelection {
                        model: selection.model.clone(),
                        effort: selection.effort.clone(),
                    });
                }
            }),
        );
        self.bottom_pane.show_view(Box::new(view));
        self.request_redraw();
    }
}
