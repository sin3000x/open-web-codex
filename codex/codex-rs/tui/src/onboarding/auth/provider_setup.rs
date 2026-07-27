//! Custom Provider setup state, rendering, and persistence for onboarding.
//!
//! Keeping this retained workflow below the authentication module minimizes replay churn when the
//! upstream ChatGPT and API-key onboarding flows change.

use super::*;
use codex_model_provider::fetch_provider_models as fetch_remote_provider_models;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::ProviderModelInfo;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::built_in_model_providers;

mod render;

use render::provider_setup_field_title;
use render::provider_setup_form_lines;
use render::provider_setup_model_lines;
use render::provider_setup_placeholder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderSetupField {
    Id,
    Name,
    BaseUrl,
    EnvKey,
    WireApi,
    FetchModels,
    Model,
    ContextWindow,
    Confirm,
}

#[derive(Clone)]
pub(crate) struct ProviderSetupState {
    field: ProviderSetupField,
    id: String,
    name: String,
    base_url: String,
    env_key: String,
    wire_api: WireApi,
    model: String,
    models: Vec<ProviderModelInfo>,
    selected_model_index: usize,
    context_window: i64,
    input: String,
    input_is_prefill: bool,
    is_saving: bool,
}

impl ProviderSetupState {
    fn new() -> Self {
        Self {
            field: ProviderSetupField::Id,
            id: String::new(),
            name: String::new(),
            base_url: String::new(),
            env_key: String::new(),
            wire_api: WireApi::Chat,
            model: "deepseek-chat".to_string(),
            models: Vec::new(),
            selected_model_index: 0,
            context_window: 262_144,
            input: String::new(),
            input_is_prefill: true,
            is_saving: false,
        }
    }

    fn start_field(&mut self, field: ProviderSetupField) {
        self.field = field;
        self.input = match field {
            ProviderSetupField::Id => self.id.clone(),
            ProviderSetupField::Name => self.name.clone(),
            ProviderSetupField::BaseUrl => self.base_url.clone(),
            ProviderSetupField::EnvKey => self.env_key.clone(),
            ProviderSetupField::WireApi => String::new(),
            ProviderSetupField::FetchModels => String::new(),
            ProviderSetupField::Model => String::new(),
            ProviderSetupField::ContextWindow => self.context_window.to_string(),
            ProviderSetupField::Confirm => String::new(),
        };
        self.input_is_prefill = field != ProviderSetupField::Confirm;
    }

    fn apply_input(&mut self) -> Result<(), String> {
        let value = self.input.trim();
        match self.field {
            ProviderSetupField::Id => {
                if value.is_empty() {
                    return Err("Provider id cannot be empty".to_string());
                }
                self.id = value.to_string();
                self.start_field(ProviderSetupField::Name);
            }
            ProviderSetupField::Name => {
                if value.is_empty() {
                    return Err("Provider name cannot be empty".to_string());
                }
                self.name = value.to_string();
                self.start_field(ProviderSetupField::BaseUrl);
            }
            ProviderSetupField::BaseUrl => {
                if value.is_empty() {
                    return Err("Base URL cannot be empty".to_string());
                }
                self.base_url = value.to_string();
                self.start_field(ProviderSetupField::EnvKey);
            }
            ProviderSetupField::EnvKey => {
                if value.is_empty() {
                    return Err("Env var cannot be empty; use - for no env var".to_string());
                }
                self.env_key = value.to_string();
                self.start_field(ProviderSetupField::WireApi);
            }
            ProviderSetupField::WireApi | ProviderSetupField::FetchModels => {}
            ProviderSetupField::Model => {
                if self.models.is_empty() {
                    return Err("No models were fetched for this provider".to_string());
                }
                if let Some(model) = self.models.get(self.selected_model_index) {
                    self.model = model.model_id.clone();
                    self.context_window = model
                        .context_window
                        .or(model.max_token_len)
                        .unwrap_or(self.context_window);
                }
                self.start_field(ProviderSetupField::ContextWindow);
            }
            ProviderSetupField::ContextWindow => {
                let parsed = value
                    .parse::<i64>()
                    .map_err(|_| "Context window must be a number".to_string())?;
                if parsed <= 0 {
                    return Err("Context window must be greater than zero".to_string());
                }
                self.context_window = parsed;
                self.start_field(ProviderSetupField::Confirm);
            }
            ProviderSetupField::Confirm => {}
        }
        Ok(())
    }

    fn set_fetched_models(&mut self, models: Vec<ProviderModelInfo>) -> Result<(), String> {
        if models.is_empty() {
            return Err("Provider returned no models.".to_string());
        }
        self.models = models;
        self.selected_model_index = 0;
        self.model = self.models[0].model_id.clone();
        self.context_window = self.models[0]
            .context_window
            .or(self.models[0].max_token_len)
            .unwrap_or(self.context_window);
        self.start_field(ProviderSetupField::Model);
        Ok(())
    }

    fn move_model_selection(&mut self, direction: ModelSelectionDirection) {
        if self.models.is_empty() {
            return;
        }
        self.selected_model_index = match direction {
            ModelSelectionDirection::Previous => self
                .selected_model_index
                .checked_sub(1)
                .unwrap_or(self.models.len() - 1),
            ModelSelectionDirection::Next => (self.selected_model_index + 1) % self.models.len(),
        };
        self.model = self.models[self.selected_model_index].model_id.clone();
    }

    fn move_form_selection(&mut self, direction: ModelSelectionDirection) {
        const ROWS: [ProviderSetupField; 6] = [
            ProviderSetupField::Id,
            ProviderSetupField::Name,
            ProviderSetupField::BaseUrl,
            ProviderSetupField::EnvKey,
            ProviderSetupField::WireApi,
            ProviderSetupField::FetchModels,
        ];
        let current = ROWS
            .iter()
            .position(|field| *field == self.field)
            .unwrap_or(0);
        let next = match direction {
            ModelSelectionDirection::Previous => current.checked_sub(1).unwrap_or(ROWS.len() - 1),
            ModelSelectionDirection::Next => (current + 1) % ROWS.len(),
        };
        self.start_field(ROWS[next]);
    }

    fn selected_form_field_mut(&mut self) -> Option<&mut String> {
        match self.field {
            ProviderSetupField::Id => Some(&mut self.id),
            ProviderSetupField::Name => Some(&mut self.name),
            ProviderSetupField::BaseUrl => Some(&mut self.base_url),
            ProviderSetupField::EnvKey => Some(&mut self.env_key),
            ProviderSetupField::WireApi
            | ProviderSetupField::FetchModels
            | ProviderSetupField::Model
            | ProviderSetupField::ContextWindow
            | ProviderSetupField::Confirm => None,
        }
    }

    fn toggle_wire_api(&mut self) {
        self.wire_api = match self.wire_api {
            WireApi::Chat => WireApi::Responses,
            WireApi::Responses => WireApi::Chat,
        };
    }

    fn provider(&self) -> ModelProviderInfo {
        let mut models = self.models.clone();
        if let Some(model) = models.iter_mut().find(|model| model.model_id == self.model) {
            model.context_window = Some(self.context_window);
        }

        ModelProviderInfo {
            name: self.name.trim().to_string(),
            base_url: Some(self.base_url.trim().to_string()),
            env_key: (self.env_key.trim() != "-").then(|| self.env_key.trim().to_string()),
            wire_api: self.wire_api,
            models,
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy)]
enum ModelSelectionDirection {
    Previous,
    Next,
}

impl AuthModeWidget {
    /// Returns whether the custom-provider setup flow is editing a text field.
    pub(crate) fn is_provider_setup_text_entry_active(&self) -> bool {
        self.sign_in_state.read().is_ok_and(|guard| {
            matches!(
                &*guard,
                SignInState::ProviderSetup(state)
                    if !matches!(
                        state.field,
                        ProviderSetupField::Confirm | ProviderSetupField::WireApi | ProviderSetupField::Model
                    ) && !state.is_saving
            )
        })
    }

    /// Returns whether the custom-provider setup field currently contains text.
    pub(crate) fn provider_setup_has_text(&self) -> bool {
        self.sign_in_state.read().is_ok_and(|guard| {
            matches!(
                &*guard,
                SignInState::ProviderSetup(state) if !state.input.is_empty()
            )
        })
    }

    pub(super) fn render_provider_setup(
        &self,
        area: Rect,
        buf: &mut Buffer,
        state: &ProviderSetupState,
    ) {
        let [intro_area, input_area, footer_area] = Layout::vertical([
            Constraint::Min(8),
            Constraint::Length(match state.field {
                ProviderSetupField::Confirm => 0,
                ProviderSetupField::Id
                | ProviderSetupField::Name
                | ProviderSetupField::BaseUrl
                | ProviderSetupField::EnvKey
                | ProviderSetupField::WireApi
                | ProviderSetupField::FetchModels => 10,
                ProviderSetupField::Model => 7,
                _ => 3,
            }),
            Constraint::Min(4),
        ])
        .areas(area);

        let mut intro_lines: Vec<Line> = vec![
            Line::from(vec![
                "> ".into(),
                "Configure a custom OpenAI-compatible provider".bold(),
            ]),
            "".into(),
            "  Provider settings are stored in config.toml.".into(),
            "  Provider API keys are read from the env var you configure below."
                .dim()
                .into(),
            "".into(),
            Line::from(vec!["  id: ".dim(), state.id.clone().into()]),
            Line::from(vec!["  name: ".dim(), state.name.clone().into()]),
            Line::from(vec!["  base_url: ".dim(), state.base_url.clone().into()]),
            Line::from(vec!["  env_key: ".dim(), state.env_key.clone().into()]),
            Line::from(vec![
                "  wire_api: ".dim(),
                state.wire_api.to_string().into(),
            ]),
            Line::from(vec!["  model: ".dim(), state.model.clone().into()]),
            Line::from(vec![
                "  context_window: ".dim(),
                state.context_window.to_string().into(),
            ]),
            "".into(),
        ];
        if state.field == ProviderSetupField::Confirm {
            intro_lines.push(if state.is_saving {
                "  Saving provider configuration...".cyan().into()
            } else {
                Line::from(vec![
                    "  Press ".cyan(),
                    self.confirm_binding().into(),
                    " to confirm".cyan(),
                ])
            });
        }
        Paragraph::new(intro_lines)
            .wrap(Wrap { trim: false })
            .render(intro_area, buf);

        match state.field {
            ProviderSetupField::Confirm => {}
            ProviderSetupField::Id
            | ProviderSetupField::Name
            | ProviderSetupField::BaseUrl
            | ProviderSetupField::EnvKey
            | ProviderSetupField::WireApi
            | ProviderSetupField::FetchModels => {
                Paragraph::new(provider_setup_form_lines(state))
                    .wrap(Wrap { trim: false })
                    .block(
                        Block::default()
                            .title("Provider")
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Cyan)),
                    )
                    .render(input_area, buf);
            }
            ProviderSetupField::Model => {
                Paragraph::new(provider_setup_model_lines(state))
                    .wrap(Wrap { trim: false })
                    .block(
                        Block::default()
                            .title(provider_setup_field_title(state.field))
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Cyan)),
                    )
                    .render(input_area, buf);
            }
            _ => {
                let content_line: Line = if state.input.is_empty() {
                    provider_setup_placeholder(state.field).dim().into()
                } else {
                    Line::from(state.input.clone())
                };
                Paragraph::new(content_line)
                    .wrap(Wrap { trim: false })
                    .block(
                        Block::default()
                            .title(provider_setup_field_title(state.field))
                            .borders(Borders::ALL)
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Cyan)),
                    )
                    .render(input_area, buf);
            }
        }

        let mut footer_lines: Vec<Line> = Vec::new();
        if matches!(
            state.field,
            ProviderSetupField::Id
                | ProviderSetupField::Name
                | ProviderSetupField::BaseUrl
                | ProviderSetupField::EnvKey
                | ProviderSetupField::WireApi
                | ProviderSetupField::FetchModels
        ) {
            footer_lines.push(
                "  Use ↑/↓ to choose a field; ←/→ changes wire type"
                    .dim()
                    .into(),
            );
        } else if state.field == ProviderSetupField::Model {
            footer_lines.push("  Use ↑/↓ or 1/2 to choose a model".dim().into());
        }
        footer_lines.extend([
            Line::from(vec![
                "  Press ".dim(),
                self.confirm_binding().into(),
                if state.field == ProviderSetupField::Confirm {
                    " to save".dim()
                } else if state.field == ProviderSetupField::FetchModels {
                    " to fetch models".dim()
                } else {
                    " to continue".dim()
                },
            ]),
            Line::from(vec![
                "  Press ".dim(),
                self.cancel_binding().into(),
                " to cancel".dim(),
            ]),
        ]);
        if let Some(error) = self.error_message() {
            footer_lines.push("".into());
            footer_lines.push(error.red().into());
        }
        Paragraph::new(footer_lines)
            .wrap(Wrap { trim: false })
            .render(footer_area, buf);
    }

    pub(super) fn start_provider_setup(&mut self) {
        if !self.is_api_login_allowed() {
            self.disallow_api_login();
            return;
        }
        self.set_error(/*message*/ None);
        *self.sign_in_state.write().unwrap() =
            SignInState::ProviderSetup(ProviderSetupState::new());
        self.request_frame.schedule_frame();
    }

    pub(super) fn handle_provider_setup_key_event(&mut self, key_event: &KeyEvent) -> bool {
        let mut should_save: Option<ProviderSetupState> = None;
        let mut should_fetch: Option<ProviderSetupState> = None;
        let mut should_request_frame = false;
        let mut error_message: Option<String> = None;

        {
            let mut guard = self.sign_in_state.write().unwrap();
            let SignInState::ProviderSetup(state) = &mut *guard else {
                return false;
            };

            if state.is_saving {
                return true;
            }

            if matches!(
                state.field,
                ProviderSetupField::Id
                    | ProviderSetupField::Name
                    | ProviderSetupField::BaseUrl
                    | ProviderSetupField::EnvKey
                    | ProviderSetupField::WireApi
                    | ProviderSetupField::FetchModels
            ) {
                if keys::MOVE_UP.is_pressed(*key_event) {
                    state.move_form_selection(ModelSelectionDirection::Previous);
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                } else if keys::MOVE_DOWN.is_pressed(*key_event) {
                    state.move_form_selection(ModelSelectionDirection::Next);
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                } else if matches!(key_event.code, KeyCode::Left | KeyCode::Right)
                    && state.field == ProviderSetupField::WireApi
                {
                    state.toggle_wire_api();
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                } else if keys::CONFIRM.is_pressed(*key_event)
                    && state.field == ProviderSetupField::FetchModels
                {
                    let provider = state.provider();
                    if built_in_model_providers(None).contains_key(&state.id) {
                        error_message = Some(format!(
                            "Provider id '{}' is built in. Choose a custom id such as '{}-custom'.",
                            state.id, state.id
                        ));
                    } else if let Err(err) = provider.validate() {
                        error_message = Some(format!("Invalid provider: {err}"));
                    } else {
                        state.is_saving = true;
                        should_fetch = Some(state.clone());
                        self.set_error(/*message*/ None);
                    }
                    should_request_frame = true;
                } else if keys::CANCEL.is_pressed(*key_event) {
                    *guard = SignInState::PickMode;
                    self.set_error(/*message*/ None);
                    self.highlighted_mode = SignInOption::CustomProvider;
                    should_request_frame = true;
                } else {
                    match key_event.code {
                        KeyCode::Backspace => {
                            if let Some(value) = state.selected_form_field_mut() {
                                value.pop();
                                self.set_error(/*message*/ None);
                                should_request_frame = true;
                            }
                        }
                        KeyCode::Char(c)
                            if key_event.kind == KeyEventKind::Press
                                && !key_event.modifiers.contains(KeyModifiers::SUPER)
                                && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                                && !key_event.modifiers.contains(KeyModifiers::ALT) =>
                        {
                            if let Some(value) = state.selected_form_field_mut() {
                                value.push(c);
                                self.set_error(/*message*/ None);
                                should_request_frame = true;
                            }
                        }
                        _ => {}
                    }
                }
            } else if state.field == ProviderSetupField::Model {
                if keys::MOVE_UP.is_pressed(*key_event) || keys::SELECT_FIRST.is_pressed(*key_event)
                {
                    state.move_model_selection(ModelSelectionDirection::Previous);
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                } else if keys::MOVE_DOWN.is_pressed(*key_event)
                    || keys::SELECT_SECOND.is_pressed(*key_event)
                {
                    state.move_model_selection(ModelSelectionDirection::Next);
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                } else if keys::CONFIRM.is_pressed(*key_event) {
                    match state.apply_input() {
                        Ok(()) => self.set_error(/*message*/ None),
                        Err(err) => error_message = Some(err),
                    }
                    should_request_frame = true;
                } else if keys::CANCEL.is_pressed(*key_event) {
                    state.start_field(ProviderSetupField::WireApi);
                    self.set_error(/*message*/ None);
                    should_request_frame = true;
                }
            } else if keys::CANCEL.is_pressed(*key_event) {
                *guard = SignInState::PickMode;
                self.set_error(/*message*/ None);
                self.highlighted_mode = SignInOption::CustomProvider;
                should_request_frame = true;
            } else if keys::CONFIRM.is_pressed(*key_event) {
                if state.field == ProviderSetupField::Confirm {
                    if built_in_model_providers(None).contains_key(&state.id) {
                        error_message = Some(format!(
                            "Provider id '{}' is built in. Choose a custom id such as '{}-custom'.",
                            state.id, state.id
                        ));
                    } else {
                        let provider = state.provider();
                        if let Err(err) = provider.validate() {
                            error_message = Some(format!("Invalid provider: {err}"));
                        } else {
                            state.is_saving = true;
                            should_save = Some(state.clone());
                        }
                    }
                    should_request_frame = true;
                } else {
                    match state.apply_input() {
                        Ok(()) => self.set_error(/*message*/ None),
                        Err(err) => error_message = Some(err),
                    }
                    should_request_frame = true;
                }
            } else {
                match key_event.code {
                    KeyCode::Backspace => {
                        if state.input_is_prefill {
                            state.input.clear();
                            state.input_is_prefill = false;
                        } else {
                            state.input.pop();
                        }
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                    KeyCode::Char(c)
                        if key_event.kind == KeyEventKind::Press
                            && !key_event.modifiers.contains(KeyModifiers::SUPER)
                            && !key_event.modifiers.contains(KeyModifiers::CONTROL)
                            && !key_event.modifiers.contains(KeyModifiers::ALT)
                            && !matches!(
                                state.field,
                                ProviderSetupField::Confirm
                                    | ProviderSetupField::WireApi
                                    | ProviderSetupField::Model
                            ) =>
                    {
                        if state.input_is_prefill {
                            state.input.clear();
                            state.input_is_prefill = false;
                        }
                        state.input.push(c);
                        self.set_error(/*message*/ None);
                        should_request_frame = true;
                    }
                    _ => {}
                }
            }
        }

        if let Some(err) = error_message {
            self.set_error(Some(err));
        }
        if let Some(state) = should_fetch {
            self.fetch_provider_models(state);
        } else if let Some(state) = should_save {
            self.save_provider_setup(state);
        } else if should_request_frame {
            self.request_frame.schedule_frame();
        }
        true
    }

    pub(super) fn handle_provider_setup_paste(&mut self, pasted: String) -> bool {
        let pasted = pasted.trim();
        if pasted.is_empty() {
            return false;
        }

        let mut guard = self.sign_in_state.write().unwrap();
        let SignInState::ProviderSetup(state) = &mut *guard else {
            return false;
        };
        if matches!(
            state.field,
            ProviderSetupField::Confirm | ProviderSetupField::WireApi | ProviderSetupField::Model
        ) || state.is_saving
        {
            return true;
        }
        if state.input_is_prefill {
            state.input.clear();
            state.input_is_prefill = false;
        }
        state.input.push_str(pasted);
        drop(guard);
        self.set_error(/*message*/ None);
        self.request_frame.schedule_frame();
        true
    }

    fn fetch_provider_models(&mut self, state: ProviderSetupState) {
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            let provider = state.provider();
            let fetch_result = async {
                let models = fetch_remote_provider_models(provider, /*auth_manager*/ None)
                    .await
                    .map_err(|err| err.to_string())?;
                if models.is_empty() {
                    return Err("Provider returned no models.".to_string());
                }
                Ok(models
                    .into_iter()
                    .map(codex_protocol::openai_models::ModelPreset::from)
                    .map(|model| ProviderModelInfo::from(&model))
                    .collect::<Vec<_>>())
            }
            .await;

            let mut restored = state;
            restored.is_saving = false;
            match fetch_result.and_then(|models| {
                restored.set_fetched_models(models)?;
                Ok(())
            }) {
                Ok(()) => {
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() = SignInState::ProviderSetup(restored);
                }
                Err(err) => {
                    *error.write().unwrap() = Some(format!(
                        "Failed to fetch provider models: {err}. Check the base URL, API key env var, and wire API."
                    ));
                    restored.start_field(ProviderSetupField::WireApi);
                    *sign_in_state.write().unwrap() = SignInState::ProviderSetup(restored);
                }
            }
            request_frame.schedule_frame();
        });
        self.request_frame.schedule_frame();
    }

    fn save_provider_setup(&mut self, state: ProviderSetupState) {
        let request_handle = self.app_server_request_handle.clone();
        let sign_in_state = self.sign_in_state.clone();
        let error = self.error.clone();
        let request_frame = self.request_frame.clone();
        tokio::spawn(async move {
            let provider = state.provider();
            let mut edits = match config_update::build_model_provider_edit(&state.id, &provider) {
                Ok(edit) => vec![edit],
                Err(err) => {
                    *error.write().unwrap() = Some(format!(
                        "Failed to serialize provider '{}': {err}",
                        state.id
                    ));
                    let mut restored = state;
                    restored.is_saving = false;
                    *sign_in_state.write().unwrap() = SignInState::ProviderSetup(restored);
                    request_frame.schedule_frame();
                    return;
                }
            };
            edits.push(config_update::build_model_provider_selection_edit(
                &state.id,
            ));
            edits.push(config_update::build_model_selection_edit(&state.model));

            match config_update::write_config_batch(request_handle, edits).await {
                Ok(_) => {
                    *error.write().unwrap() = None;
                    *sign_in_state.write().unwrap() = SignInState::ProviderConfigured;
                }
                Err(err) => {
                    let error_message = config_update::format_config_error(&err);
                    *error.write().unwrap() = Some(format!(
                        "Failed to save provider configuration: {error_message}"
                    ));
                    let mut restored = state;
                    restored.is_saving = false;
                    *sign_in_state.write().unwrap() = SignInState::ProviderSetup(restored);
                }
            }
            request_frame.schedule_frame();
        });
        self.request_frame.schedule_frame();
    }
}
