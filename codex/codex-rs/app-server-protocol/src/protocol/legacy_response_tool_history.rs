//! Compatibility projection for legacy rollouts that persisted raw response tool calls.
//!
//! Canonical paginated rollouts persist semantic `ItemCompleted(TurnItem)` records and do not
//! need this fallback. Older rollouts may contain only matching `FunctionCall`/`CustomToolCall`
//! response items, so this module materializes those unmatched pairs as `DynamicToolCall` items.

use std::collections::HashMap;

use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;

use crate::protocol::v2::DynamicToolCallOutputContentItem;
use crate::protocol::v2::DynamicToolCallStatus;
use crate::protocol::v2::ThreadItem;

#[derive(Clone)]
struct ResponseToolCallSnapshot {
    turn_id: Option<String>,
    namespace: Option<String>,
    tool: String,
    arguments: serde_json::Value,
}

#[derive(Default)]
pub(super) struct LegacyResponseToolHistory {
    calls: HashMap<String, ResponseToolCallSnapshot>,
}

pub(super) enum LegacyResponseToolHistoryUpdate {
    Started {
        turn_id: Option<String>,
        item: ThreadItem,
    },
    Completed {
        turn_id: Option<String>,
        call_id: String,
        item: ThreadItem,
    },
}

impl LegacyResponseToolHistory {
    pub(super) fn handle_response_item(
        &mut self,
        item: &ResponseItem,
    ) -> Option<LegacyResponseToolHistoryUpdate> {
        match item {
            ResponseItem::FunctionCall {
                name,
                namespace,
                arguments,
                call_id,
                ..
            } => {
                let arguments = serde_json::from_str(arguments)
                    .unwrap_or_else(|_| serde_json::Value::String(arguments.clone()));
                let turn_id = item.turn_id().map(str::to_owned);
                let item = self.start(
                    call_id,
                    turn_id.as_deref(),
                    namespace.clone(),
                    name.clone(),
                    arguments,
                );
                Some(LegacyResponseToolHistoryUpdate::Started { turn_id, item })
            }
            ResponseItem::CustomToolCall {
                call_id,
                name,
                input,
                ..
            } => {
                let turn_id = item.turn_id().map(str::to_owned);
                let item = self.start(
                    call_id,
                    turn_id.as_deref(),
                    None,
                    name.clone(),
                    serde_json::Value::String(input.clone()),
                );
                Some(LegacyResponseToolHistoryUpdate::Started { turn_id, item })
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            }
            | ResponseItem::CustomToolCallOutput {
                call_id, output, ..
            } => {
                let (turn_id, item) = self.complete(call_id, item.turn_id(), output)?;
                Some(LegacyResponseToolHistoryUpdate::Completed {
                    turn_id,
                    call_id: call_id.clone(),
                    item,
                })
            }
            _ => None,
        }
    }

    fn start(
        &mut self,
        call_id: &str,
        turn_id: Option<&str>,
        namespace: Option<String>,
        tool: String,
        arguments: serde_json::Value,
    ) -> ThreadItem {
        tracing::debug!(
            target: "codex_app_server_protocol::legacy_response_tool_history",
            legacy_response_tool_history = true,
            "legacy response tool history compatibility path used"
        );
        self.calls.insert(
            call_id.to_string(),
            ResponseToolCallSnapshot {
                turn_id: turn_id.map(str::to_owned),
                namespace: namespace.clone(),
                tool: tool.clone(),
                arguments: arguments.clone(),
            },
        );
        ThreadItem::DynamicToolCall {
            id: call_id.to_string(),
            namespace,
            tool,
            arguments,
            status: DynamicToolCallStatus::InProgress,
            content_items: None,
            success: None,
            duration_ms: None,
        }
    }

    fn complete(
        &self,
        call_id: &str,
        output_turn_id: Option<&str>,
        output: &FunctionCallOutputPayload,
    ) -> Option<(Option<String>, ThreadItem)> {
        let snapshot = self.calls.get(call_id)?.clone();
        let turn_id = output_turn_id
            .map(str::to_owned)
            .or_else(|| snapshot.turn_id.clone());
        let success = output.success.unwrap_or(true);
        let status = if success {
            DynamicToolCallStatus::Completed
        } else {
            DynamicToolCallStatus::Failed
        };
        Some((
            turn_id,
            ThreadItem::DynamicToolCall {
                id: call_id.to_string(),
                namespace: snapshot.namespace,
                tool: snapshot.tool,
                arguments: snapshot.arguments,
                status,
                content_items: Some(output_content_items(output)),
                success: Some(success),
                duration_ms: None,
            },
        ))
    }

    pub(super) fn fail_incomplete(&self, items: &mut [ThreadItem]) {
        let mut incomplete_count = 0_u64;
        for item in items {
            if !self.calls.contains_key(item.id()) {
                continue;
            }
            if let ThreadItem::DynamicToolCall {
                status, success, ..
            } = item
                && *status == DynamicToolCallStatus::InProgress
            {
                *status = DynamicToolCallStatus::Failed;
                *success = Some(false);
                incomplete_count = incomplete_count.saturating_add(1);
            }
        }
        if incomplete_count > 0 {
            tracing::debug!(
                target: "codex_app_server_protocol::legacy_response_tool_history",
                legacy_response_tool_history = true,
                incomplete_count,
                "legacy response tool history closed incomplete calls"
            );
        }
    }
}

fn output_content_items(
    output: &FunctionCallOutputPayload,
) -> Vec<DynamicToolCallOutputContentItem> {
    match &output.body {
        FunctionCallOutputBody::Text(text) => {
            vec![DynamicToolCallOutputContentItem::InputText { text: text.clone() }]
        }
        FunctionCallOutputBody::ContentItems(items) => items
            .iter()
            .filter_map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => {
                    Some(DynamicToolCallOutputContentItem::InputText { text: text.clone() })
                }
                FunctionCallOutputContentItem::InputImage { image_url, .. } => {
                    Some(DynamicToolCallOutputContentItem::InputImage {
                        image_url: image_url.clone(),
                    })
                }
                FunctionCallOutputContentItem::InputAudio { audio_url } => {
                    Some(DynamicToolCallOutputContentItem::InputAudio {
                        audio_url: audio_url.clone(),
                    })
                }
                FunctionCallOutputContentItem::EncryptedContent { .. } => None,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::thread_history::build_turns_from_rollout_items;
    use crate::protocol::v2::ThreadItem;
    use codex_protocol::ThreadId;
    use codex_protocol::items::CommandExecutionItem as CoreCommandExecutionItem;
    use codex_protocol::items::CommandExecutionStatus as CoreCommandExecutionStatus;
    use codex_protocol::items::TurnItem as CoreTurnItem;
    use codex_protocol::models::InternalChatMessageMetadataPassthrough;
    use codex_protocol::models::ResponseItem;
    use codex_protocol::parse_command::ParsedCommand;
    use codex_protocol::protocol::EventMsg;
    use codex_protocol::protocol::ExecCommandSource;
    use codex_protocol::protocol::ItemCompletedEvent;
    use codex_protocol::protocol::RolloutItem;
    use codex_protocol::protocol::TurnCompleteEvent;
    use codex_protocol::protocol::TurnStartedEvent;
    use codex_utils_absolute_path::test_support::PathBufExt;
    use codex_utils_absolute_path::test_support::test_path_buf;
    use pretty_assertions::assert_eq;
    use std::time::Duration;

    #[test]
    fn output_content_items_preserves_audio_from_legacy_tool_history() {
        let output = FunctionCallOutputPayload {
            body: FunctionCallOutputBody::ContentItems(vec![
                FunctionCallOutputContentItem::InputAudio {
                    audio_url: "data:audio/wav;base64,AA==".to_string(),
                },
            ]),
            success: Some(true),
        };

        assert_eq!(
            output_content_items(&output),
            vec![DynamicToolCallOutputContentItem::InputAudio {
                audio_url: "data:audio/wav;base64,AA==".to_string(),
            }]
        );
    }

    #[test]
    fn rebuilds_function_calls_from_persisted_response_items() {
        let turn_id = "turn-tools";
        let metadata = || {
            Some(InternalChatMessageMetadataPassthrough {
                turn_id: Some(turn_id.to_string()),
            })
        };
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: turn_id.to_string(),
                trace_id: None,
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "exec_command".into(),
                namespace: None,
                arguments: r#"{"cmd":"git status"}"#.into(),
                call_id: "call-1".into(),
                internal_chat_message_metadata_passthrough: metadata(),
            }),
            RolloutItem::ResponseItem(ResponseItem::FunctionCallOutput {
                id: None,
                call_id: "call-1".into(),
                output: FunctionCallOutputPayload::from_text("Process exited with code 0".into()),
                internal_chat_message_metadata_passthrough: metadata(),
            }),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: turn_id.to_string(),
                started_at: None,
                last_agent_message: None,
                error: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].items.len(), 1);
        assert_eq!(
            turns[0].items[0],
            ThreadItem::DynamicToolCall {
                id: "call-1".into(),
                namespace: None,
                tool: "exec_command".into(),
                arguments: serde_json::json!({"cmd":"git status"}),
                status: DynamicToolCallStatus::Completed,
                content_items: Some(vec![DynamicToolCallOutputContentItem::InputText {
                    text: "Process exited with code 0".into(),
                }]),
                success: Some(true),
                duration_ms: None,
            }
        );
    }

    #[test]
    fn closes_persisted_response_tool_without_output_when_turn_finishes() {
        let turn_id = "turn-interrupted-tool";
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: turn_id.to_string(),
                trace_id: None,
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "exec_command".into(),
                namespace: None,
                arguments: r#"{"cmd":"false"}"#.into(),
                call_id: "call-no-output".into(),
                internal_chat_message_metadata_passthrough: Some(
                    InternalChatMessageMetadataPassthrough {
                        turn_id: Some(turn_id.to_string()),
                    },
                ),
            }),
            RolloutItem::EventMsg(EventMsg::TurnComplete(TurnCompleteEvent {
                turn_id: turn_id.to_string(),
                started_at: None,
                last_agent_message: None,
                error: None,
                completed_at: None,
                duration_ms: None,
                time_to_first_token_ms: None,
            })),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert!(matches!(
            turns[0].items[0],
            ThreadItem::DynamicToolCall {
                status: DynamicToolCallStatus::Failed,
                success: Some(false),
                ..
            }
        ));
    }

    #[test]
    fn semantic_tool_events_override_generic_response_tool_history() {
        let turn_id = "turn-native";
        let metadata = || {
            Some(InternalChatMessageMetadataPassthrough {
                turn_id: Some(turn_id.to_string()),
            })
        };
        let command_item = CoreTurnItem::CommandExecution(CoreCommandExecutionItem {
            id: "call-native".into(),
            process_id: None,
            command: vec!["git".into(), "status".into()],
            cwd: test_path_buf("/tmp").abs().into(),
            parsed_cmd: vec![ParsedCommand::Unknown {
                cmd: "git status".into(),
            }],
            source: ExecCommandSource::Agent,
            interaction_input: None,
            status: CoreCommandExecutionStatus::Completed,
            stdout: Some("clean\n".into()),
            stderr: Some(String::new()),
            aggregated_output: Some("clean\n".into()),
            exit_code: Some(0),
            duration: Some(Duration::from_millis(4)),
            formatted_output: Some("clean\n".into()),
        });
        let items = vec![
            RolloutItem::EventMsg(EventMsg::TurnStarted(TurnStartedEvent {
                turn_id: turn_id.into(),
                trace_id: None,
                started_at: None,
                model_context_window: None,
                collaboration_mode_kind: Default::default(),
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCall {
                id: None,
                name: "exec_command".into(),
                namespace: None,
                arguments: r#"{"cmd":"git status"}"#.into(),
                call_id: "call-native".into(),
                internal_chat_message_metadata_passthrough: metadata(),
            }),
            RolloutItem::EventMsg(EventMsg::ItemCompleted(ItemCompletedEvent {
                thread_id: ThreadId::new(),
                turn_id: turn_id.into(),
                item: command_item,
                completed_at_ms: 1,
            })),
            RolloutItem::ResponseItem(ResponseItem::FunctionCallOutput {
                id: None,
                call_id: "call-native".into(),
                output: FunctionCallOutputPayload::from_text("generic output".into()),
                internal_chat_message_metadata_passthrough: metadata(),
            }),
        ];

        let turns = build_turns_from_rollout_items(&items);
        assert_eq!(turns[0].items.len(), 1);
        assert!(matches!(
            turns[0].items[0],
            ThreadItem::CommandExecution { .. }
        ));
    }
}
