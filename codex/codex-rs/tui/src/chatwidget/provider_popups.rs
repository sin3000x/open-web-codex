//! Provider management popups and slash-command helpers.

use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::WireApi;
use codex_model_provider_info::built_in_model_providers;
use ratatui::style::Stylize;
use ratatui::text::Line;
use ratatui::text::Span;

use super::*;
use crate::app_event::ProviderFormDraft;
use crate::app_event::ProviderFormMode;
use crate::app_event_sender::AppEventSender;
use crate::bottom_pane::BottomPaneView;
use crate::bottom_pane::ViewCompletion;
use crate::chatwidget::provider_sections::ProviderListSection;
use crate::chatwidget::provider_sections::ProviderSectionCounts;
use crate::chatwidget::provider_sections::provider_description;
use crate::chatwidget::provider_sections::provider_fetch_models_description;
use crate::chatwidget::provider_sections::provider_list_section;
use crate::chatwidget::provider_sections::provider_section_counts;

const PROVIDERS_USAGE: &str = "Usage: /providers [add|edit|delete|use|fetch] ...";
const PROVIDER_FORM_VIEW_ID: &str = "provider-form";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProviderFormRow {
    Id,
    Name,
    BaseUrl,
    EnvKey,
    WireApi,
    FetchModels,
    Cancel,
}

impl ProviderFormRow {
    const ADD_ROWS: [Self; 7] = [
        Self::Id,
        Self::Name,
        Self::BaseUrl,
        Self::EnvKey,
        Self::WireApi,
        Self::FetchModels,
        Self::Cancel,
    ];
    const EDIT_ROWS: [Self; 6] = [
        Self::Name,
        Self::BaseUrl,
        Self::EnvKey,
        Self::WireApi,
        Self::FetchModels,
        Self::Cancel,
    ];

    fn rows(mode: ProviderFormMode) -> &'static [Self] {
        match mode {
            ProviderFormMode::Add => &Self::ADD_ROWS,
            ProviderFormMode::Edit => &Self::EDIT_ROWS,
        }
    }
}

struct ProviderFormView {
    mode: ProviderFormMode,
    draft: ProviderFormDraft,
    selected: usize,
    completion: Option<ViewCompletion>,
    app_event_tx: AppEventSender,
    is_fetching: bool,
}

impl ProviderFormView {
    fn new(mode: ProviderFormMode, draft: ProviderFormDraft, app_event_tx: AppEventSender) -> Self {
        Self {
            mode,
            draft,
            selected: 0,
            completion: None,
            app_event_tx,
            is_fetching: false,
        }
    }

    fn selected_row(&self) -> ProviderFormRow {
        ProviderFormRow::rows(self.mode)[self.selected]
    }

    fn move_selection(&mut self, delta: isize) {
        let rows = ProviderFormRow::rows(self.mode);
        let len = rows.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    fn edit_selected_text(&mut self, ch: char) {
        if self.is_fetching {
            return;
        }
        match self.selected_row() {
            ProviderFormRow::Id => self.draft.id.push(ch),
            ProviderFormRow::Name => self.draft.name.push(ch),
            ProviderFormRow::BaseUrl => self.draft.base_url.push(ch),
            ProviderFormRow::EnvKey => self.draft.env_key.push(ch),
            ProviderFormRow::WireApi | ProviderFormRow::FetchModels | ProviderFormRow::Cancel => {}
        }
    }

    fn backspace_selected_text(&mut self) {
        match self.selected_row() {
            ProviderFormRow::Id => {
                self.draft.id.pop();
            }
            ProviderFormRow::Name => {
                self.draft.name.pop();
            }
            ProviderFormRow::BaseUrl => {
                self.draft.base_url.pop();
            }
            ProviderFormRow::EnvKey => {
                self.draft.env_key.pop();
            }
            ProviderFormRow::WireApi | ProviderFormRow::FetchModels | ProviderFormRow::Cancel => {}
        }
    }

    fn toggle_wire_api(&mut self) {
        self.draft.wire_api = match self.draft.wire_api {
            WireApi::Chat => WireApi::Responses,
            WireApi::Responses => WireApi::Chat,
        };
    }

    fn submit(&mut self) {
        match self.selected_row() {
            ProviderFormRow::FetchModels => {
                let provider = ModelProviderInfo {
                    name: self.draft.name.trim().to_string(),
                    base_url: Some(self.draft.base_url.trim().to_string()),
                    env_key: (self.draft.env_key.trim() != "-")
                        .then(|| self.draft.env_key.trim().to_string()),
                    wire_api: self.draft.wire_api,
                    ..Default::default()
                };
                self.is_fetching = true;
                self.draft.id = self.draft.id.trim().to_string();
                let draft = self.draft.clone();
                self.app_event_tx.send(AppEvent::ProviderConfigAction {
                    action: match self.mode {
                        ProviderFormMode::Add => {
                            crate::app_event::ProviderConfigAction::FetchModelsForNewProvider {
                                draft,
                                provider,
                            }
                        }
                        ProviderFormMode::Edit => crate::app_event::ProviderConfigAction::Upsert {
                            id: draft.id,
                            provider,
                        },
                    },
                });
            }
            ProviderFormRow::Cancel => {
                self.completion = Some(ViewCompletion::Cancelled);
                self.app_event_tx.send(AppEvent::OpenProviderManager);
            }
            ProviderFormRow::WireApi => self.toggle_wire_api(),
            ProviderFormRow::Id
            | ProviderFormRow::Name
            | ProviderFormRow::BaseUrl
            | ProviderFormRow::EnvKey => {}
        }
    }
}

impl BottomPaneView for ProviderFormView {
    fn handle_key_event(&mut self, key_event: KeyEvent) {
        if self.is_fetching || key_event.kind == KeyEventKind::Release {
            return;
        }
        match key_event.code {
            KeyCode::Up => self.move_selection(-1),
            KeyCode::Down | KeyCode::Tab => self.move_selection(1),
            KeyCode::Left | KeyCode::Right if self.selected_row() == ProviderFormRow::WireApi => {
                self.toggle_wire_api();
            }
            KeyCode::Backspace => self.backspace_selected_text(),
            KeyCode::Enter => self.submit(),
            KeyCode::Char(' ') if self.selected_row() == ProviderFormRow::FetchModels => {
                self.submit();
            }
            KeyCode::Esc => {
                self.completion = Some(ViewCompletion::Cancelled);
            }
            KeyCode::Char(c)
                if !key_event.modifiers.contains(KeyModifiers::CONTROL)
                    && !key_event.modifiers.contains(KeyModifiers::ALT)
                    && !key_event.modifiers.contains(KeyModifiers::SUPER) =>
            {
                self.edit_selected_text(c);
            }
            _ => {}
        }
    }

    fn is_complete(&self) -> bool {
        self.completion.is_some()
    }

    fn completion(&self) -> Option<ViewCompletion> {
        self.completion
    }

    fn view_id(&self) -> Option<&'static str> {
        Some(PROVIDER_FORM_VIEW_ID)
    }
}

impl Renderable for ProviderFormView {
    fn desired_height(&self, _width: u16) -> u16 {
        12
    }

    fn render(&self, area: Rect, buf: &mut Buffer) {
        use ratatui::widgets::Paragraph;
        if area.height == 0 || area.width == 0 {
            return;
        }
        let mut lines = vec![
            Line::from(provider_form_title(self.mode).bold()),
            Line::from(if self.is_fetching {
                "Fetching models...".cyan()
            } else {
                "Fill the form, then choose Fetch models.".dim()
            }),
            "".into(),
        ];
        for (index, row) in ProviderFormRow::rows(self.mode).iter().enumerate() {
            let selected = index == self.selected;
            let marker = if selected { "› ".cyan() } else { "  ".into() };
            let line = match row {
                ProviderFormRow::Id => {
                    provider_form_row_line(marker, "id", &self.draft.id, "provider-id", selected)
                }
                ProviderFormRow::Name => provider_form_row_line(
                    marker,
                    "name",
                    &self.draft.name,
                    "My Provider",
                    selected,
                ),
                ProviderFormRow::BaseUrl => provider_form_row_line(
                    marker,
                    "url",
                    &self.draft.base_url,
                    "https://api.example.com/v1",
                    selected,
                ),
                ProviderFormRow::EnvKey => provider_form_row_line(
                    marker,
                    "apikey env",
                    &self.draft.env_key,
                    "ENV_VAR_NAME or -",
                    selected,
                ),
                ProviderFormRow::WireApi => {
                    let chat = if self.draft.wire_api == WireApi::Chat {
                        " Chat ".cyan().bold()
                    } else {
                        " Chat ".dim()
                    };
                    let responses = if self.draft.wire_api == WireApi::Responses {
                        " Responses ".cyan().bold()
                    } else {
                        " Responses ".dim()
                    };
                    Line::from(vec![
                        marker,
                        "wire type  ".dim(),
                        chat,
                        " ".into(),
                        responses,
                    ])
                }
                ProviderFormRow::FetchModels => {
                    let label = if self.is_fetching {
                        "Fetching models..."
                    } else {
                        "Fetch models"
                    };
                    Line::from(vec![marker, label.green().bold()])
                }
                ProviderFormRow::Cancel => Line::from(vec![marker, "Cancel".dim()]),
            };
            lines.push(line);
        }
        lines.push("".into());
        lines.push(
            "↑/↓/Tab fields · ←/→ wire type · Enter/Space fetch · Esc cancel"
                .dim()
                .into(),
        );
        Paragraph::new(lines).render(area, buf);
    }
}

fn provider_form_row_line(
    marker: Span<'static>,
    label: &'static str,
    value: &str,
    placeholder: &str,
    selected: bool,
) -> Line<'static> {
    let value = if value.is_empty() {
        placeholder.to_string().dim()
    } else if selected {
        value.to_string().cyan()
    } else {
        value.to_string().into()
    };
    Line::from(vec![marker, format!("{label:<11}").dim(), value])
}

impl ChatWidget {
    pub(crate) fn dismiss_provider_form(&mut self) {
        self.bottom_pane
            .dismiss_active_view_if_id(PROVIDER_FORM_VIEW_ID);
    }

    pub(crate) fn open_provider_manager(&mut self) {
        self.bottom_pane.clear_active_views();
        let current_provider_id = self.config.model_provider_id.clone();
        let builtin_ids = built_in_model_providers(None);
        let mut providers: Vec<(String, ModelProviderInfo, bool, ProviderListSection)> = self
            .config
            .model_providers
            .iter()
            .map(|(id, provider)| {
                let is_builtin = builtin_ids.contains_key(id);
                let section = provider_list_section(id, is_builtin);
                (id.clone(), provider.clone(), is_builtin, section)
            })
            .collect();
        providers.sort_by(|(left_id, _, _, _), (right_id, _, _, _)| left_id.cmp(right_id));

        let provider_counts = provider_section_counts(&providers);

        let mut items = vec![
            SelectionItem {
                name: "Add provider".to_string(),
                description: Some("Create a custom OpenAI-compatible provider".to_string()),
                actions: vec![Box::new(|tx| {
                    tx.send(AppEvent::OpenProviderForm {
                        mode: ProviderFormMode::Add,
                        draft: ProviderFormDraft {
                            id: String::new(),
                            name: String::new(),
                            base_url: String::new(),
                            env_key: String::new(),
                            wire_api: WireApi::Chat,
                        },
                    });
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Refresh list".to_string(),
                description: Some("Reload provider configuration from disk".to_string()),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenProviderManager))],
                dismiss_on_select: true,
                ..Default::default()
            },
        ];

        push_provider_section(
            &mut items,
            "Built-in providers",
            "OpenAI providers managed by Codex; existing model behavior is unchanged.",
        );
        for (id, provider, is_builtin, _) in providers
            .iter()
            .filter(|(_, _, _, section)| *section == ProviderListSection::ManagedBuiltIn)
        {
            push_provider_list_item(&mut items, id, provider, *is_builtin, &current_provider_id);
        }

        push_provider_section(
            &mut items,
            "Local OSS providers",
            "Local-model providers managed by Codex; built-in but special.",
        );
        for (id, provider, is_builtin, _) in providers
            .iter()
            .filter(|(_, _, _, section)| *section == ProviderListSection::LocalOss)
        {
            push_provider_list_item(&mut items, id, provider, *is_builtin, &current_provider_id);
        }

        push_provider_section(
            &mut items,
            "Custom providers",
            "OpenAI-compatible providers you manage, with provider-specific cached models.",
        );
        for (id, provider, is_builtin, _) in providers
            .iter()
            .filter(|(_, _, _, section)| *section == ProviderListSection::Custom)
        {
            push_provider_list_item(&mut items, id, provider, *is_builtin, &current_provider_id);
        }

        let header = providers_header(provider_counts);
        self.bottom_pane.show_selection_view(SelectionViewParams {
            is_searchable: true,
            search_placeholder: Some("Search providers".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            items,
            header,
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(crate) fn open_provider_detail(&mut self, id: &str) {
        let Some(provider) = self.config.model_providers.get(id).cloned() else {
            self.add_error_message(format!("Provider '{id}' is not configured."));
            self.open_provider_manager();
            return;
        };
        let is_builtin = built_in_model_providers(None).contains_key(id);
        let current = id == self.config.model_provider_id;
        let section = provider_list_section(id, is_builtin);

        let mut items = vec![
            SelectionItem {
                name: "Use this provider".to_string(),
                description: Some(if current {
                    "Already selected in config".to_string()
                } else {
                    "Set model_provider for new sessions".to_string()
                }),
                display_shortcut: Some(provider_shortcut('u')),
                is_current: current,
                actions: vec![Box::new({
                    let id = id.to_string();
                    move |tx| {
                        tx.send(AppEvent::ProviderConfigAction {
                            action: crate::app_event::ProviderConfigAction::Use { id: id.clone() },
                        });
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Fetch models".to_string(),
                description: Some(provider_fetch_models_description(section, &provider)),
                display_shortcut: Some(provider_shortcut('f')),
                is_disabled: is_builtin,
                actions: vec![Box::new({
                    let id = id.to_string();
                    move |tx| {
                        tx.send(AppEvent::ProviderConfigAction {
                            action: crate::app_event::ProviderConfigAction::FetchModels {
                                id: id.clone(),
                            },
                        });
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Edit provider".to_string(),
                description: Some(if is_builtin {
                    "Built-in providers cannot be edited here".to_string()
                } else {
                    "Open the provider form with current values".to_string()
                }),
                display_shortcut: Some(provider_shortcut('e')),
                is_disabled: is_builtin,
                actions: vec![Box::new({
                    let draft = provider_form_draft(id, &provider);
                    move |tx| {
                        tx.send(AppEvent::OpenProviderForm {
                            mode: ProviderFormMode::Edit,
                            draft: draft.clone(),
                        });
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            },
            SelectionItem {
                name: "Delete provider".to_string(),
                description: Some(if is_builtin {
                    "Built-in providers cannot be deleted".to_string()
                } else {
                    "Open a confirmation prompt".to_string()
                }),
                display_shortcut: Some(provider_shortcut('d')),
                is_disabled: is_builtin,
                actions: vec![Box::new({
                    let id = id.to_string();
                    move |tx| tx.send(AppEvent::OpenProviderDeleteConfirm { id: id.clone() })
                })],
                dismiss_on_select: false,
                ..Default::default()
            },
            SelectionItem {
                name: "Back to providers".to_string(),
                description: Some("Return to the provider list".to_string()),
                display_shortcut: Some(provider_shortcut('b')),
                actions: vec![Box::new(|tx| tx.send(AppEvent::OpenProviderManager))],
                dismiss_on_select: false,
                ..Default::default()
            },
        ];

        if !is_builtin {
            items.push(SelectionItem {
                name: "Add similar provider".to_string(),
                description: Some("Open the provider form using these values".to_string()),
                actions: vec![Box::new({
                    let mut cloned = provider.clone();
                    cloned.name = format!("{} Copy", provider.name);
                    let draft = provider_form_draft(&format!("{id}-copy"), &cloned);
                    move |tx| {
                        tx.send(AppEvent::OpenProviderForm {
                            mode: ProviderFormMode::Add,
                            draft: draft.clone(),
                        });
                    }
                })],
                dismiss_on_select: true,
                ..Default::default()
            });
        }

        let header = provider_detail_header(id, &provider, is_builtin);
        self.bottom_pane.show_selection_view(SelectionViewParams {
            footer_hint: Some(standard_popup_hint_line()),
            items,
            header,
            on_cancel: Some(Box::new(|tx| tx.send(AppEvent::OpenProviderManager))),
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(crate) fn open_provider_delete_confirm(&mut self, id: &str) {
        let Some(provider) = self.config.model_providers.get(id) else {
            self.add_error_message(format!("Provider '{id}' is not configured."));
            self.open_provider_manager();
            return;
        };
        let title = provider_title(id, provider);
        self.bottom_pane.show_selection_view(SelectionViewParams {
            title: Some(format!("Delete {title}?")),
            subtitle: Some("This removes the provider from config.toml.".to_string()),
            footer_hint: Some(standard_popup_hint_line()),
            on_cancel: Some(Box::new({
                let id = id.to_string();
                move |tx| tx.send(AppEvent::OpenProviderDetail { id: id.clone() })
            })),
            items: vec![
                SelectionItem {
                    name: "Cancel".to_string(),
                    description: Some("Keep this provider".to_string()),
                    actions: vec![Box::new({
                        let id = id.to_string();
                        move |tx| tx.send(AppEvent::OpenProviderDetail { id: id.clone() })
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
                SelectionItem {
                    name: "Delete provider".to_string(),
                    description: Some("Remove it from model_providers".to_string()),
                    actions: vec![Box::new({
                        let id = id.to_string();
                        move |tx| {
                            tx.send(AppEvent::ProviderConfigAction {
                                action: crate::app_event::ProviderConfigAction::Delete {
                                    id: id.clone(),
                                },
                            });
                        }
                    })],
                    dismiss_on_select: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        self.request_redraw();
    }

    pub(crate) fn open_provider_form(&mut self, mode: ProviderFormMode, draft: ProviderFormDraft) {
        self.bottom_pane.show_view(Box::new(ProviderFormView::new(
            mode,
            draft,
            self.app_event_tx.clone(),
        )));
        self.request_redraw();
    }

    pub(crate) fn handle_provider_command_args(&mut self, args: &str) {
        let Some(parts) = shlex::split(args) else {
            self.add_error_message("Could not parse provider command arguments.".to_string());
            return;
        };
        let Some((subcommand, rest)) = parts.split_first() else {
            self.open_provider_manager();
            return;
        };
        match subcommand.as_str() {
            "add" => self.handle_provider_upsert_args(rest, /*is_edit*/ false),
            "edit" => self.handle_provider_upsert_args(rest, /*is_edit*/ true),
            "delete" | "remove" => self.handle_provider_delete_args(rest),
            "use" | "select" => self.handle_provider_use_args(rest),
            "fetch" | "models" => self.handle_provider_fetch_args(rest),
            _ => self.add_error_message(PROVIDERS_USAGE.to_string()),
        }
    }

    fn handle_provider_upsert_args(&mut self, args: &[String], is_edit: bool) {
        let [id, name, base_url, env_key, rest @ ..] = args else {
            self.add_error_message(
                "Usage: /providers add <id> <name> <base_url> <env_key|-> [chat|responses]"
                    .to_string(),
            );
            return;
        };
        if id.trim().is_empty() {
            self.add_error_message("Provider id cannot be empty.".to_string());
            return;
        }
        let builtin_ids = built_in_model_providers(None);
        if builtin_ids.contains_key(id) {
            self.add_error_message(format!("Built-in provider '{id}' cannot be edited here."));
            return;
        }
        let existing = self.config.model_providers.get(id).cloned();
        if is_edit && existing.is_none() {
            self.add_error_message(format!("Provider '{id}' does not exist."));
            return;
        }
        if !is_edit && existing.is_some() {
            self.add_error_message(format!(
                "Provider '{id}' already exists. Use /providers edit."
            ));
            return;
        }
        let wire_api = match rest {
            [] => WireApi::Chat,
            [wire] => match parse_wire_api(wire) {
                Some(wire_api) => wire_api,
                None => {
                    self.add_error_message("wire_api must be chat or responses.".to_string());
                    return;
                }
            },
            _ => {
                self.add_error_message(
                    "Usage: /providers edit <id> <name> <base_url> <env_key|-> [chat|responses]"
                        .to_string(),
                );
                return;
            }
        };

        let mut provider = existing.unwrap_or_default();
        provider.name = name.clone();
        provider.base_url = Some(base_url.clone());
        provider.env_key = (env_key != "-").then(|| env_key.clone());
        provider.wire_api = wire_api;
        if let Err(err) = provider.validate() {
            self.add_error_message(format!("Invalid provider '{id}': {err}"));
            return;
        }

        self.app_event_tx.send(AppEvent::ProviderConfigAction {
            action: crate::app_event::ProviderConfigAction::Upsert {
                id: id.clone(),
                provider,
            },
        });
    }

    fn handle_provider_delete_args(&mut self, args: &[String]) {
        let [id] = args else {
            self.add_error_message("Usage: /providers delete <id>".to_string());
            return;
        };
        self.app_event_tx.send(AppEvent::ProviderConfigAction {
            action: crate::app_event::ProviderConfigAction::Delete { id: id.clone() },
        });
    }

    fn handle_provider_use_args(&mut self, args: &[String]) {
        let [id] = args else {
            self.add_error_message("Usage: /providers use <id>".to_string());
            return;
        };
        self.app_event_tx.send(AppEvent::ProviderConfigAction {
            action: crate::app_event::ProviderConfigAction::Use { id: id.clone() },
        });
    }

    fn handle_provider_fetch_args(&mut self, args: &[String]) {
        let [id] = args else {
            self.add_error_message("Usage: /providers fetch <id>".to_string());
            return;
        };
        self.app_event_tx.send(AppEvent::ProviderConfigAction {
            action: crate::app_event::ProviderConfigAction::FetchModels { id: id.clone() },
        });
    }
}

fn push_provider_section(items: &mut Vec<SelectionItem>, name: &str, description: &str) {
    items.push(SelectionItem {
        name: name.to_string(),
        description: Some(description.to_string()),
        is_disabled: true,
        ..Default::default()
    });
}

fn push_provider_list_item(
    items: &mut Vec<SelectionItem>,
    id: &str,
    provider: &ModelProviderInfo,
    is_builtin: bool,
    current_provider_id: &str,
) {
    let title = provider_title(id, provider);
    let description = Some(provider_description(id, provider, is_builtin));
    let detail_id = id.to_string();
    items.push(SelectionItem {
        name: title,
        description,
        is_current: id == current_provider_id,
        actions: vec![Box::new(move |tx| {
            tx.send(AppEvent::OpenProviderDetail {
                id: detail_id.clone(),
            });
        })],
        dismiss_on_select: false,
        search_value: Some(format!(
            "{} {} {} {}",
            id,
            provider.name,
            provider.base_url.clone().unwrap_or_default(),
            provider.env_key.clone().unwrap_or_default()
        )),
        ..Default::default()
    });
}

fn provider_title(id: &str, provider: &ModelProviderInfo) -> String {
    if provider.name.trim().is_empty() {
        id.to_string()
    } else {
        format!("{} ({id})", provider.name)
    }
}

fn providers_header(counts: ProviderSectionCounts) -> Box<dyn Renderable> {
    let mut header = ColumnRenderable::new();
    header.push(Line::from("Manage Providers".bold()));
    header.push(Line::from(
        "Review provider details. Add and edit use an interactive form.".dim(),
    ));
    header.push(Line::from(vec![
        "Managed".bold(),
        format!(" {}", counts.managed).dim(),
        "  •  Local OSS".bold(),
        format!(" {}", counts.local_oss).dim(),
        "  •  Custom".bold(),
        format!(" {}", counts.custom).dim(),
    ]));
    header.push(Line::from(
        "Rows are grouped by provider type; local OSS providers are used by local-model flows."
            .dim(),
    ));
    Box::new(header)
}

fn provider_detail_header(
    id: &str,
    provider: &ModelProviderInfo,
    is_builtin: bool,
) -> Box<dyn Renderable> {
    let mut header = ColumnRenderable::new();
    header.push(Line::from(provider_title(id, provider).bold()));
    header.push(Line::from(
        provider_description(id, provider, is_builtin).dim(),
    ));
    Box::new(header)
}

fn provider_form_draft(id: &str, provider: &ModelProviderInfo) -> ProviderFormDraft {
    ProviderFormDraft {
        id: id.to_string(),
        name: provider.name.clone(),
        base_url: provider
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.example.com/v1".to_string()),
        env_key: provider.env_key.clone().unwrap_or_else(|| "-".to_string()),
        wire_api: provider.wire_api,
    }
}

fn provider_form_title(mode: ProviderFormMode) -> &'static str {
    match mode {
        ProviderFormMode::Add => "Add provider",
        ProviderFormMode::Edit => "Edit provider",
    }
}

fn parse_wire_api(value: &str) -> Option<WireApi> {
    match value {
        "chat" => Some(WireApi::Chat),
        "responses" => Some(WireApi::Responses),
        _ => None,
    }
}

fn provider_shortcut(ch: char) -> KeyBinding {
    KeyBinding::new(KeyCode::Char(ch), KeyModifiers::NONE)
}

#[cfg(test)]
#[path = "provider_popups_tests.rs"]
mod tests;
