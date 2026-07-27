//! Translation layer between Codex's Responses-shaped data model and the
//! OpenAI-compatible Chat Completions wire format.
//!
//! Third-party providers (DeepSeek, OpenRouter, vLLM, Ollama's OpenAI shim,
//! LM Studio, etc.) overwhelmingly speak the Chat Completions API
//! (`POST /v1/chat/completions`), not the Responses API. This module bridges
//! the gap so a `wire_api = "chat"` provider can reuse all of Codex's
//! Responses-shaped session state (`ResponseItem`, tool specs, reasoning) by
//! translating at the request boundary.
//!
//! Translation is intentionally **lossy but safe**: Responses-only concepts
//! (stateful `store`/`previous_response_id`, encrypted reasoning, server-side
//! `include`, OpenAI-hosted tools like `web_search`/`image_generation`) have
//! no Chat Completions equivalent and are dropped or approximated. The core
//! agent loop runs identically regardless of wire protocol because both paths
//! produce the same [`crate::common::ResponseEvent`] stream.

use crate::common::ResponsesApiRequest;
use codex_protocol::models::AgentMessageInputContent;
use codex_protocol::models::ContentItem;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

/// The reasoning effort hint expressed in the way most chat-compatible
/// providers understand it (OpenAI o-series and DeepSeek-R1 style).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ChatReasoningEffort {
    Low,
    Medium,
    High,
}

/// A single message in the Chat Completions `messages` array.
///
/// The shapes here deliberately follow the OpenAI Chat Completions schema so
/// that it serializes directly into a valid request body.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum ChatMessage {
    /// Plain text/content message: `{ "role", "content" }`.
    Text { role: String, content: String },
    /// Assistant message that carries tool calls.
    AssistantWithToolCalls {
        role: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        tool_calls: Vec<ChatToolCall>,
    },
    /// The result of a tool call: `{ "role": "tool", "tool_call_id", "content" }`.
    ToolResult {
        role: String,
        tool_call_id: String,
        content: String,
    },
}

/// A tool call attached to an assistant message, in Chat Completions shape:
/// `{ "id", "type": "function", "function": { "name", "arguments" } }`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: ChatToolCallFunction,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatToolCallFunction {
    pub name: String,
    /// Raw JSON arguments string, exactly as in the Responses `FunctionCall`.
    pub arguments: String,
}

/// A tool definition in Chat Completions shape:
/// `{ "type": "function", "function": { "name", "description", "strict", "parameters" } }`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatTool {
    #[serde(rename = "type")]
    pub r#type: String,
    pub function: ChatToolFunction,
    #[serde(skip)]
    pub target: ChatToolTarget,
}

/// The Responses-side identity for a function exposed on the flattened Chat
/// Completions tool surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatToolTarget {
    pub name: String,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ChatToolFunction {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Whether the provider should strictly validate tool arguments against
    /// the schema. Many chat-compatible providers do not support this flag;
    /// it is only emitted when `true`.
    #[serde(skip_serializing_if = "skip_false")]
    pub strict: bool,
    pub parameters: Value,
}

/// Request body for the OpenAI-compatible Chat Completions API
/// (`POST {base_url}/chat/completions`).
///
/// This is the wire payload used by third-party providers configured with
/// `wire_api = "chat"`. It is assembled from the same Responses-shaped session
/// state via [`responses_request_to_chat_completions_request`]. Responses-only
/// fields (`store`, `include`, `previous_response_id`, server-side reasoning)
/// are intentionally absent because they have no Chat Completions equivalent.
#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ChatCompletionsApiRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ChatTool>,
    /// `"auto"` by default; `"none"` when no compatible tool is exposed.
    pub tool_choice: String,
    pub stream: bool,
    /// Requested alongside `stream: true` so the final chunk carries token
    /// usage. Many providers only return usage when this is set.
    pub stream_options: ChatStreamOptions,
    /// Optional reasoning-effort hint understood by OpenAI o-series and
    /// DeepSeek-R1 style models. Omitted from the body when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ChatReasoningEffort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

/// `stream_options` object controlling usage reporting during streaming.
#[derive(Debug, Default, Serialize, Clone, PartialEq)]
pub struct ChatStreamOptions {
    /// Include token usage in the terminal streamed chunk.
    pub include_usage: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn skip_false(value: &bool) -> bool {
    !value
}

/// Converts an owned Responses request into the third-party Chat Completions
/// wire request. Core owns prompt assembly and request-scoped metadata; this
/// module owns every lossy wire-format decision.
pub fn responses_request_to_chat_completions_request(
    request: ResponsesApiRequest,
) -> ChatCompletionsApiRequest {
    let tools = request
        .tools
        .as_deref()
        .map(responses_tools_to_chat_tools)
        .unwrap_or_default();
    let tool_choice = if tools.is_empty() {
        "none".to_string()
    } else {
        request.tool_choice
    };
    let reasoning_effort = request
        .reasoning
        .and_then(|reasoning| reasoning.effort)
        .and_then(chat_reasoning_effort);

    ChatCompletionsApiRequest {
        model: request.model,
        messages: responses_input_to_chat_messages(&request.input, &request.instructions),
        tools,
        tool_choice,
        stream: true,
        stream_options: ChatStreamOptions {
            include_usage: true,
        },
        reasoning_effort,
        max_tokens: None,
        temperature: None,
        top_p: None,
        service_tier: request.service_tier,
        user: None,
    }
}

fn chat_reasoning_effort(effort: ReasoningEffortConfig) -> Option<ChatReasoningEffort> {
    match effort {
        ReasoningEffortConfig::None => None,
        ReasoningEffortConfig::Minimal | ReasoningEffortConfig::Low => {
            Some(ChatReasoningEffort::Low)
        }
        ReasoningEffortConfig::Medium => Some(ChatReasoningEffort::Medium),
        ReasoningEffortConfig::High
        | ReasoningEffortConfig::XHigh
        | ReasoningEffortConfig::Max
        | ReasoningEffortConfig::Ultra => Some(ChatReasoningEffort::High),
        ReasoningEffortConfig::Custom(value) => match value.as_str() {
            "low" => Some(ChatReasoningEffort::Low),
            "medium" => Some(ChatReasoningEffort::Medium),
            "high" => Some(ChatReasoningEffort::High),
            _ => None,
        },
    }
}

/// Converts a Responses-style tool spec list (`create_tools_json_for_responses_api`
/// output) into the Chat Completions `tools` array shape.
///
/// Plain functions are preserved. Namespace functions are flattened into
/// request-unique Chat function names and retain their Responses identity in
/// [`ChatTool::target`] so streamed calls can be restored before dispatch.
/// Responses-native tools without complete Chat semantics remain hidden.
pub fn responses_tools_to_chat_tools(tools: &[Value]) -> Vec<ChatTool> {
    let mut out = Vec::with_capacity(tools.len());
    for tool in tools {
        match tool.get("type").and_then(Value::as_str) {
            Some("function") => {
                if let Some(converted) = convert_function_tool(tool, None, None) {
                    out.push(converted);
                }
            }
            Some("namespace") => convert_namespace_tool(tool, &mut out),
            // These Responses-native tools do not have a complete Chat
            // Completions equivalent. Keep them hidden rather than exposing a
            // tool the provider cannot execute with the same semantics.
            Some("tool_search" | "web_search" | "image_generation" | "custom") | None => {}
            Some(_) => {}
        }
    }
    let mut seen_names = HashSet::new();
    out.retain(|tool| seen_names.insert(tool.function.name.clone()));
    out
}

fn convert_namespace_tool(tool: &Value, out: &mut Vec<ChatTool>) {
    let Some(namespace) = tool.get("name").and_then(Value::as_str) else {
        return;
    };
    let namespace_description = tool
        .get("description")
        .and_then(Value::as_str)
        .filter(|description| !description.is_empty());
    let Some(tools) = tool.get("tools").and_then(Value::as_array) else {
        return;
    };
    for nested_tool in tools {
        if nested_tool.get("type").and_then(Value::as_str) != Some("function") {
            continue;
        }
        if let Some(converted) =
            convert_function_tool(nested_tool, Some(namespace), namespace_description)
        {
            out.push(converted);
        }
    }
}

fn convert_function_tool(
    tool: &Value,
    namespace: Option<&str>,
    namespace_description: Option<&str>,
) -> Option<ChatTool> {
    let name = tool.get("name").and_then(Value::as_str)?.to_string();
    let tool_description = tool
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("");
    let description = match (namespace_description, tool_description.is_empty()) {
        (Some(namespace_description), false) => {
            format!("{namespace_description}\n\n{tool_description}")
        }
        (Some(namespace_description), true) => namespace_description.to_string(),
        (None, _) => tool_description.to_string(),
    };
    let strict = tool.get("strict").and_then(Value::as_bool).unwrap_or(false);
    let parameters = tool
        .get("parameters")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let wire_name = namespace
        .map(|namespace| format!("{namespace}__{name}"))
        .unwrap_or_else(|| name.clone());

    Some(ChatTool {
        r#type: "function".to_string(),
        function: ChatToolFunction {
            name: wire_name,
            description,
            strict,
            parameters,
        },
        target: ChatToolTarget {
            name,
            namespace: namespace.map(str::to_string),
        },
    })
}

/// Flatten a Responses `input` array into a Chat Completions `messages` array.
///
/// `instructions` (the top-level `instructions` field of a Responses request)
/// becomes a leading `system` message when non-empty, matching how most chat
/// providers expect a system/developer preamble.
///
/// Mapping rules:
/// - `ResponseItem::Message{role}` → `{role, content}`, where `developer`
///   aliases to `system` (most chat providers only know `system`). Content
///   items are concatenated into a single string (text only; images dropped
///   for now as multimodal support varies by provider).
/// - `ResponseItem::FunctionCall` → an assistant message
///   carrying `tool_calls`.
/// - `ResponseItem::FunctionCallOutput` → a `{role:"tool", tool_call_id,
///   content}` message.
/// - `ResponseItem::Reasoning` → dropped (no chat equivalent; the model cannot
///   act on encrypted/server-side reasoning from a third-party provider).
/// - `ResponseItem::LocalShellCall` and other Responses-native tool calls are
///   approximated as assistant tool calls when they carry a usable call_id.
/// - `ResponseItem::AdditionalTools` carries inline tool defs; ignored here
///   (tools are passed separately via [`responses_tools_to_chat_tools`]).
///
/// Consecutive tool calls share one assistant message (Chat Completions allows
/// multiple `tool_calls` per assistant turn), and each tool result is its own
/// `tool` message keyed by `tool_call_id`.
pub fn responses_input_to_chat_messages(
    input: &[ResponseItem],
    instructions: &str,
) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(input.len() + 1);

    if !instructions.is_empty() {
        messages.push(ChatMessage::Text {
            role: "system".to_string(),
            content: instructions.to_string(),
        });
    }

    for item in input {
        match item {
            ResponseItem::Message { role, content, .. } => {
                let normalized_role = normalize_role(role);
                let text = content
                    .iter()
                    .filter_map(content_item_to_text)
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.trim().is_empty() {
                    continue;
                }
                push_or_merge_text(&mut messages, normalized_role, text);
            }
            ResponseItem::AgentMessage { content, .. } => {
                let text = content
                    .iter()
                    .filter_map(|c| match c {
                        AgentMessageInputContent::InputText { text } => Some(text.as_str()),
                        AgentMessageInputContent::EncryptedContent { .. } => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                if text.trim().is_empty() {
                    continue;
                }
                push_or_merge_text(&mut messages, "assistant".to_string(), text);
            }
            ResponseItem::FunctionCall {
                name,
                namespace,
                arguments,
                call_id,
                ..
            } => {
                let tool_call = ChatToolCall {
                    id: call_id.clone(),
                    r#type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: namespace
                            .as_deref()
                            .map(|namespace| format!("{namespace}__{name}"))
                            .unwrap_or_else(|| name.clone()),
                        arguments: arguments.clone(),
                    },
                };
                push_assistant_tool_call(&mut messages, tool_call);
            }
            ResponseItem::FunctionCallOutput {
                call_id, output, ..
            } => {
                let content = output.body.to_text().unwrap_or_default();
                push_tool_result(&mut messages, call_id.clone(), content);
            }
            // Responses-only concepts with no chat equivalent.
            ResponseItem::Reasoning { .. }
            | ResponseItem::AdditionalTools { .. }
            | ResponseItem::CustomToolCall { .. }
            | ResponseItem::CustomToolCallOutput { .. }
            | ResponseItem::ToolSearchCall { .. }
            | ResponseItem::ToolSearchOutput { .. }
            | ResponseItem::WebSearchCall { .. }
            | ResponseItem::ImageGenerationCall { .. }
            | ResponseItem::Compaction { .. }
            | ResponseItem::CompactionTrigger {}
            | ResponseItem::ContextCompaction { .. }
            | ResponseItem::Other => {}
            // Best-effort: model the shell call as an assistant tool call when
            // it carries a call_id the model can reference. Shell calls without
            // a call_id (legacy form) are dropped to avoid confusing the model.
            ResponseItem::LocalShellCall { call_id, .. } => {
                if let Some(call_id) = call_id.as_ref() {
                    let tool_call = ChatToolCall {
                        id: call_id.clone(),
                        r#type: "function".to_string(),
                        function: ChatToolCallFunction {
                            name: "local_shell".to_string(),
                            arguments: String::from("{}"),
                        },
                    };
                    push_assistant_tool_call(&mut messages, tool_call);
                }
            }
        }
    }

    messages
}

/// Map Responses roles onto the set most chat providers accept.
fn normalize_role(role: &str) -> String {
    match role {
        // `developer` is Responses-specific; chat providers typically only
        // recognize `system`. Aliasing keeps the preamble visible.
        "developer" | "system" => "system".to_string(),
        "user" => "user".to_string(),
        "assistant" => "assistant".to_string(),
        // Pass through anything else (some providers accept `tool`, etc.).
        other => other.to_string(),
    }
}

fn content_item_to_text(item: &ContentItem) -> Option<String> {
    match item {
        ContentItem::InputText { text } | ContentItem::OutputText { text } => {
            if text.is_empty() {
                None
            } else {
                Some(text.clone())
            }
        }
        // Multimodal content is intentionally not translated here: support for
        // image and audio inputs varies widely across chat-compatible
        // providers. A future enhancement can emit provider-compatible part
        // arrays once the transport contract supports them explicitly.
        ContentItem::InputImage { .. } | ContentItem::InputAudio { .. } => None,
    }
}

/// Append a plain text message, merging with a preceding same-role text
/// message to keep the conversation compact.
fn push_or_merge_text(messages: &mut Vec<ChatMessage>, role: String, text: String) {
    if let Some(ChatMessage::Text {
        role: last_role,
        content,
    }) = messages.last_mut()
        && *last_role == role
    {
        content.push('\n');
        content.push_str(&text);
        return;
    }
    messages.push(ChatMessage::Text {
        role,
        content: text,
    });
}

/// Append a tool call to the trailing assistant message if it is an
/// `AssistantWithToolCalls`, otherwise start a new one. Chat Completions
/// groups a turn's parallel tool calls under a single assistant message.
fn push_assistant_tool_call(messages: &mut Vec<ChatMessage>, tool_call: ChatToolCall) {
    if let Some(ChatMessage::AssistantWithToolCalls { tool_calls, .. }) = messages.last_mut() {
        tool_calls.push(tool_call);
        return;
    }
    messages.push(ChatMessage::AssistantWithToolCalls {
        role: "assistant".to_string(),
        content: None,
        tool_calls: vec![tool_call],
    });
}

fn push_tool_result(messages: &mut Vec<ChatMessage>, tool_call_id: String, content: String) {
    let insert_at = messages
        .iter()
        .rposition(|message| matches!(message, ChatMessage::AssistantWithToolCalls { .. }))
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

#[cfg(test)]
#[path = "chat_translate_tests.rs"]
mod tests;
