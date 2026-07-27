//! Rendering helpers for the custom Provider onboarding form.

use super::*;

pub(super) fn provider_setup_form_lines(state: &ProviderSetupState) -> Vec<Line<'static>> {
    let mut lines = vec![
        provider_setup_form_row(
            state.field == ProviderSetupField::Id,
            "id",
            &state.id,
            "provider-id",
        ),
        provider_setup_form_row(
            state.field == ProviderSetupField::Name,
            "name",
            &state.name,
            "My Provider",
        ),
        provider_setup_form_row(
            state.field == ProviderSetupField::BaseUrl,
            "url",
            &state.base_url,
            "https://api.example.com/v1",
        ),
        provider_setup_form_row(
            state.field == ProviderSetupField::EnvKey,
            "apikey env",
            &state.env_key,
            "ENV_VAR_NAME or -",
        ),
    ];

    let marker = if state.field == ProviderSetupField::WireApi {
        "  › ".cyan()
    } else {
        "    ".into()
    };
    let chat = if state.wire_api == WireApi::Chat {
        " Chat ".cyan().bold()
    } else {
        " Chat ".dim()
    };
    let responses = if state.wire_api == WireApi::Responses {
        " Responses ".cyan().bold()
    } else {
        " Responses ".dim()
    };
    lines.push(Line::from(vec![
        marker,
        "wire type  ".dim(),
        chat,
        " ".into(),
        responses,
    ]));

    let marker = if state.field == ProviderSetupField::FetchModels {
        "  › ".cyan()
    } else {
        "    ".into()
    };
    let label = if state.is_saving {
        "Fetching models..."
    } else {
        "Fetch models"
    };
    lines.push(Line::from(vec![marker, label.green().bold()]));
    lines
}

fn provider_setup_form_row(
    selected: bool,
    label: &'static str,
    value: &str,
    placeholder: &str,
) -> Line<'static> {
    let marker = if selected {
        "  › ".cyan()
    } else {
        "    ".into()
    };
    let value = if value.is_empty() {
        placeholder.to_string().dim()
    } else if selected {
        value.to_string().cyan()
    } else {
        value.to_string().into()
    };
    Line::from(vec![marker, format!("{label:<11}").dim(), value])
}

pub(super) fn provider_setup_model_lines(state: &ProviderSetupState) -> Vec<Line<'static>> {
    let selected = state.selected_model_index;
    state
        .models
        .iter()
        .enumerate()
        .take(5)
        .map(|(index, model)| {
            let context_window = model.context_window.or(model.max_token_len);
            let context_suffix = context_window
                .map(|window| format!(" — {}K ctx", window / 1024))
                .unwrap_or_default();
            let label = model
                .model_name
                .as_deref()
                .unwrap_or(model.model_id.as_str());
            let display = if label == model.model_id {
                format!("{label}{context_suffix}")
            } else {
                format!("{} ({}){}", label, model.model_id, context_suffix)
            };
            if index == selected {
                Line::from(vec!["  › ".cyan(), display.cyan().bold()])
            } else {
                Line::from(vec!["    ".into(), display.into()])
            }
        })
        .collect()
}

pub(super) fn provider_setup_field_title(field: ProviderSetupField) -> &'static str {
    match field {
        ProviderSetupField::Id => "Provider id",
        ProviderSetupField::Name => "Display name",
        ProviderSetupField::BaseUrl => "Base URL",
        ProviderSetupField::EnvKey => "API key env var",
        ProviderSetupField::WireApi => "Wire API",
        ProviderSetupField::FetchModels => "Fetch models",
        ProviderSetupField::Model => "Select model",
        ProviderSetupField::ContextWindow => "Context window",
        ProviderSetupField::Confirm => "Confirm",
    }
}

pub(super) fn provider_setup_placeholder(field: ProviderSetupField) -> &'static str {
    match field {
        ProviderSetupField::Id => "deepseek",
        ProviderSetupField::Name => "DeepSeek",
        ProviderSetupField::BaseUrl => "https://api.deepseek.com/v1",
        ProviderSetupField::EnvKey => "DEEPSEEK_API_KEY or - for no env var",
        ProviderSetupField::WireApi => "chat or responses",
        ProviderSetupField::FetchModels => "",
        ProviderSetupField::Model => "",
        ProviderSetupField::ContextWindow => "262144",
        ProviderSetupField::Confirm => "",
    }
}
