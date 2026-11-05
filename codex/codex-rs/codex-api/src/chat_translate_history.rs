use super::ChatMessage;
use super::ChatToolCall;
use super::ChatToolCallFunction;
use super::unsupported;
use crate::error::ApiError;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputBody;
use codex_protocol::models::FunctionCallOutputContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_protocol::models::plaintext_agent_message_content;

pub(crate) fn responses_input_to_chat_messages(
    input: &[ResponseItem],
    instructions: &str,
) -> Result<Vec<ChatMessage>, ApiError> {
    let mut messages = Vec::with_capacity(input.len() + 1);
    let mut expected_tool_outputs = Vec::new();
    if !instructions.is_empty() {
        messages.push(ChatMessage::Text {
            role: "system".to_string(),
            content: instructions.to_string(),
        });
    }

    let mut index = 0;
    while let Some(item) = input.get(index) {
        if !expected_tool_outputs.is_empty()
            && !matches!(
                item,
                ResponseItem::FunctionCallOutput { .. } | ResponseItem::ToolSearchOutput { .. }
            )
        {
            return Err(unsupported(
                "history with a tool-call group not followed by its tool results",
            ));
        }
        match item {
            ResponseItem::Message { role, content, .. } => {
                let role = chat_role(role)?;
                let text = response_message_text(&role, content)?;
                if role == "assistant" {
                    index += 1;
                    let tool_calls = take_function_calls(input, &mut index)?;
                    if tool_calls.is_empty() {
                        push_assistant(&mut messages, text, None, None);
                    } else {
                        push_chat_tool_calls(
                            &mut messages,
                            text,
                            None,
                            tool_calls,
                            &mut expected_tool_outputs,
                        );
                    }
                    continue;
                }
                push_text(&mut messages, role, text);
            }
            ResponseItem::AgentMessage { content, .. } => {
                // Preserve the established assistant-side Chat projection for an
                // entirely plaintext native mailbox message. The Runtime keeps
                // the native AgentMessage metadata in its own history.
                let text = plaintext_agent_message_content(content)
                    .ok_or_else(|| unsupported("non-plaintext native Agent message"))?;
                push_text(&mut messages, "assistant".to_string(), text);
            }
            ResponseItem::Reasoning {
                summary,
                content,
                encrypted_content,
                ..
            } => {
                let reasoning_content = raw_chat_reasoning_content(
                    summary,
                    content.as_deref(),
                    encrypted_content.as_deref(),
                )?;
                index += 1;
                let assistant_content = match input.get(index) {
                    Some(ResponseItem::Message { role, content, .. }) if role == "assistant" => {
                        index += 1;
                        response_message_text("assistant", content)?
                    }
                    // A mailbox message can arrive at the safe reasoning boundary,
                    // before the interrupted model response has emitted assistant text.
                    // Keep the reasoning in its own empty assistant envelope; the next
                    // loop iteration preserves the mailbox message as its own Chat item.
                    _ => String::new(),
                };
                let tool_calls = take_function_calls(input, &mut index)?;
                if tool_calls.is_empty() {
                    push_assistant(
                        &mut messages,
                        assistant_content,
                        Some(reasoning_content),
                        None,
                    );
                } else {
                    push_chat_tool_calls(
                        &mut messages,
                        assistant_content,
                        Some(reasoning_content),
                        tool_calls,
                        &mut expected_tool_outputs,
                    );
                }
                continue;
            }
            ResponseItem::FunctionCall { .. } | ResponseItem::ToolSearchCall { .. } => {
                let tool_calls = take_function_calls(input, &mut index)?;
                push_chat_tool_calls(
                    &mut messages,
                    String::new(),
                    None,
                    tool_calls,
                    &mut expected_tool_outputs,
                );
                continue;
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => {
                let Some(position) = expected_tool_outputs
                    .iter()
                    .position(|expected| expected == call_id)
                else {
                    return Err(unsupported(
                        "history tool result without its preceding tool-call group",
                    ));
                };
                expected_tool_outputs.remove(position);
                push_tool_result(
                    &mut messages,
                    call_id.clone(),
                    function_output_to_chat_text(output)?,
                );
            }
            ResponseItem::ToolSearchOutput {
                call_id: Some(call_id),
                status,
                execution,
                tools,
                ..
            } => {
                if status != "completed" || execution != "client" {
                    return Err(unsupported("non-client completed tool_search output"));
                }
                let Some(position) = expected_tool_outputs
                    .iter()
                    .position(|expected| expected == call_id)
                else {
                    return Err(unsupported(
                        "history tool_search result without its preceding tool-call group",
                    ));
                };
                expected_tool_outputs.remove(position);
                let tools =
                    serde_json::to_string(tools).map_err(|error| ApiError::InvalidRequest {
                        message: format!("failed to serialize tool_search result: {error}"),
                    })?;
                push_tool_result(&mut messages, call_id.clone(), tools);
            }
            ResponseItem::AdditionalTools { .. }
            | ResponseItem::LocalShellCall { .. }
            | ResponseItem::CustomToolCall { .. }
            | ResponseItem::CustomToolCallOutput { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::CompactionTrigger {}
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::Other => return Err(unsupported("Responses-only response items")),
        }
        index += 1;
    }
    if !expected_tool_outputs.is_empty() {
        return Err(unsupported(
            "history with a tool-call group not followed by its tool results",
        ));
    }
    Ok(messages)
}

fn response_message_text(role: &str, content: &[ContentItem]) -> Result<String, ApiError> {
    content
        .iter()
        .map(|item| content_item_to_text(role, item))
        .collect::<Result<Vec<_>, _>>()
        .map(|text| text.concat())
}

fn raw_chat_reasoning_content(
    summary: &[codex_protocol::models::ReasoningItemReasoningSummary],
    content: Option<&[codex_protocol::models::ReasoningItemContent]>,
    encrypted_content: Option<&str>,
) -> Result<String, ApiError> {
    if !summary.is_empty() || encrypted_content.is_some() {
        return Err(unsupported("non-raw reasoning history"));
    }
    match content {
        Some([codex_protocol::models::ReasoningItemContent::ReasoningText { text }]) => {
            Ok(text.clone())
        }
        _ => Err(unsupported("non-raw reasoning history")),
    }
}

/// Chat providers require assistant tool-call arguments to be a JSON document.
/// A model may emit unparsable arguments, which Core answers with a parse-error
/// tool output; the canonical history keeps the raw text, and only this wire
/// projection wraps it so the provider can still accept the follow-up request.
fn chat_wire_function_arguments(arguments: &str) -> Result<String, ApiError> {
    if serde_json::from_str::<serde_json::Value>(arguments).is_ok() {
        return Ok(arguments.to_string());
    }
    serde_json::to_string(&serde_json::json!({ "unparsed_arguments": arguments })).map_err(
        |error| ApiError::InvalidRequest {
            message: format!("failed to serialize unparsed tool arguments: {error}"),
        },
    )
}

fn take_function_calls(
    input: &[ResponseItem],
    index: &mut usize,
) -> Result<Vec<ChatToolCall>, ApiError> {
    let mut tool_calls = Vec::new();
    while let Some(item) = input.get(*index) {
        match item {
            ResponseItem::FunctionCall {
                name,
                namespace,
                arguments,
                call_id,
                ..
            } => {
                tool_calls.push(ChatToolCall {
                    id: call_id.clone(),
                    r#type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: namespace
                            .as_deref()
                            .map(|namespace| format!("{namespace}__{name}"))
                            .unwrap_or_else(|| name.clone()),
                        arguments: chat_wire_function_arguments(arguments)?,
                    },
                });
            }
            ResponseItem::ToolSearchCall {
                call_id: Some(call_id),
                execution,
                arguments,
                ..
            } if execution == "client" => {
                tool_calls.push(ChatToolCall {
                    id: call_id.clone(),
                    r#type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: "tool_search".to_string(),
                        arguments: serde_json::to_string(arguments).map_err(|error| {
                            ApiError::InvalidRequest {
                                message: format!(
                                    "failed to serialize tool_search arguments: {error}"
                                ),
                            }
                        })?,
                    },
                });
            }
            ResponseItem::ToolSearchCall { .. } => {
                return Err(unsupported("non-client tool_search call"));
            }
            _ => break,
        }
        *index += 1;
    }
    Ok(tool_calls)
}

fn push_chat_tool_calls(
    messages: &mut Vec<ChatMessage>,
    content: String,
    reasoning_content: Option<String>,
    tool_calls: Vec<ChatToolCall>,
    expected_tool_outputs: &mut Vec<String>,
) {
    expected_tool_outputs.extend(tool_calls.iter().map(|tool_call| tool_call.id.clone()));
    messages.push(ChatMessage::Assistant {
        role: "assistant".to_string(),
        content,
        reasoning_content,
        tool_calls: Some(tool_calls),
    });
}

fn function_output_to_chat_text(output: &FunctionCallOutputPayload) -> Result<String, ApiError> {
    match &output.body {
        FunctionCallOutputBody::Text(text) => Ok(text.clone()),
        FunctionCallOutputBody::ContentItems(items) => items
            .iter()
            .map(|item| match item {
                FunctionCallOutputContentItem::InputText { text } => Ok(text.as_str()),
                FunctionCallOutputContentItem::InputImage { .. } => {
                    Err(unsupported("image tool output"))
                }
                FunctionCallOutputContentItem::InputAudio { .. } => {
                    Err(unsupported("audio tool output"))
                }
                FunctionCallOutputContentItem::EncryptedContent { .. } => {
                    Err(unsupported("encrypted tool output"))
                }
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|text| text.join("\n")),
    }
}

fn chat_role(role: &str) -> Result<String, ApiError> {
    match role {
        // DeepSeek's Chat subset has no developer role; preserve pinned Core developer input as system.
        "developer" | "system" => Ok("system".to_string()),
        "user" => Ok("user".to_string()),
        "assistant" => Ok("assistant".to_string()),
        _ => Err(unsupported("unsupported message role")),
    }
}

fn content_item_to_text(role: &str, item: &ContentItem) -> Result<String, ApiError> {
    match (role, item) {
        ("assistant", ContentItem::OutputText { text })
        | ("system" | "user", ContentItem::InputText { text }) => Ok(text.clone()),
        (_, ContentItem::InputText { .. } | ContentItem::OutputText { .. }) => {
            Err(unsupported("message content does not match its role"))
        }
        (_, ContentItem::InputImage { .. }) => Err(unsupported("image input")),
        (_, ContentItem::InputAudio { .. }) => Err(unsupported("audio input")),
    }
}

fn push_text(messages: &mut Vec<ChatMessage>, role: String, text: String) {
    messages.push(ChatMessage::Text {
        role,
        content: text,
    });
}

fn push_assistant(
    messages: &mut Vec<ChatMessage>,
    content: String,
    reasoning_content: Option<String>,
    tool_calls: Option<Vec<ChatToolCall>>,
) {
    messages.push(ChatMessage::Assistant {
        role: "assistant".to_string(),
        content,
        reasoning_content,
        tool_calls,
    });
}

fn push_tool_result(messages: &mut Vec<ChatMessage>, tool_call_id: String, content: String) {
    let insert_at = messages
        .iter()
        .rposition(|message| matches!(message, ChatMessage::Assistant { .. }))
        .map(|idx| idx + 1)
        .unwrap_or(messages.len());
    let mut insert_at = insert_at;
    while insert_at < messages.len()
        && matches!(messages[insert_at], ChatMessage::ToolResult { .. })
    {
        insert_at += 1;
    }
    messages.insert(
        insert_at,
        ChatMessage::ToolResult {
            role: "tool".to_string(),
            tool_call_id,
            content,
        },
    );
}
