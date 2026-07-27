use super::*;
use crate::common::Reasoning;
use crate::common::ResponsesApiRequest;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;
use codex_protocol::openai_models::ReasoningEffort as ReasoningEffortConfig;
use serde_json::json;

fn user_msg(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn assistant_msg(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn developer_msg(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "developer".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn function_call(name: &str, args: &str, call_id: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: name.to_string(),
        namespace: None,
        arguments: args.to_string(),
        call_id: call_id.to_string(),
        internal_chat_message_metadata_passthrough: None,
    }
}

fn namespaced_function_call(
    namespace: &str,
    name: &str,
    args: &str,
    call_id: &str,
) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: name.to_string(),
        namespace: Some(namespace.to_string()),
        arguments: args.to_string(),
        call_id: call_id.to_string(),
        internal_chat_message_metadata_passthrough: None,
    }
}

#[test]
fn instructions_become_leading_system_message() {
    let messages = responses_input_to_chat_messages(&[], "you are helpful");
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ChatMessage::Text { role, content } => {
            assert_eq!(role, "system");
            assert_eq!(content, "you are helpful");
        }
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn no_instructions_means_no_system_message() {
    let messages = responses_input_to_chat_messages(&[user_msg("hi")], "");
    assert!(
        messages
            .iter()
            .all(|m| !matches!(m, ChatMessage::Text { role, .. } if role == "system"))
    );
}

#[test]
fn developer_role_aliases_to_system() {
    let messages = responses_input_to_chat_messages(&[developer_msg("rules")], "");
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ChatMessage::Text { role, .. } => assert_eq!(role, "system"),
        other => panic!("expected system, got {other:?}"),
    }
}

#[test]
fn consecutive_same_role_text_messages_are_merged() {
    let messages = responses_input_to_chat_messages(&[user_msg("hello"), user_msg("world")], "");
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ChatMessage::Text { role, content } => {
            assert_eq!(role, "user");
            assert_eq!(content, "hello\nworld");
        }
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn assistant_then_user_keeps_separate_messages() {
    let messages = responses_input_to_chat_messages(&[assistant_msg("hi"), user_msg("bye")], "");
    assert_eq!(messages.len(), 2);
}

#[test]
fn parallel_function_calls_group_into_one_assistant_message() {
    let messages = responses_input_to_chat_messages(
        &[
            function_call("read", "{}", "call_1"),
            function_call("write", "{}", "call_2"),
        ],
        "",
    );
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ChatMessage::AssistantWithToolCalls {
            tool_calls,
            content,
            role,
        } => {
            assert_eq!(role, "assistant");
            assert!(content.is_none());
            assert_eq!(tool_calls.len(), 2);
            assert_eq!(tool_calls[0].function.name, "read");
            assert_eq!(tool_calls[0].id, "call_1");
            assert_eq!(tool_calls[1].function.name, "write");
        }
        other => panic!("expected AssistantWithToolCalls, got {other:?}"),
    }
}

#[test]
fn namespaced_function_call_history_uses_flattened_chat_name() {
    let messages = responses_input_to_chat_messages(
        &[namespaced_function_call(
            "mcp__map_cards",
            "create_map_card",
            "{}",
            "call_1",
        )],
        "",
    );

    let ChatMessage::AssistantWithToolCalls { tool_calls, .. } = &messages[0] else {
        panic!("expected AssistantWithToolCalls, got {:?}", messages[0]);
    };
    assert_eq!(
        tool_calls[0].function.name,
        "mcp__map_cards__create_map_card"
    );
}

#[test]
fn function_call_output_becomes_tool_role_message() {
    let messages = responses_input_to_chat_messages(
        &[ResponseItem::FunctionCallOutput {
            id: None,
            call_id: "call_1".to_string(),
            output: FunctionCallOutputPayload::from_text("result data".to_string()),
            internal_chat_message_metadata_passthrough: None,
        }],
        "",
    );
    assert_eq!(messages.len(), 1);
    match &messages[0] {
        ChatMessage::ToolResult {
            role,
            tool_call_id,
            content,
        } => {
            assert_eq!(role, "tool");
            assert_eq!(tool_call_id, "call_1");
            assert_eq!(content, "result data");
        }
        other => panic!("expected ToolResult, got {other:?}"),
    }
}

#[test]
fn reasoning_items_are_dropped() {
    let messages = responses_input_to_chat_messages(
        &[
            user_msg("hi"),
            ResponseItem::Reasoning {
                id: None,
                summary: vec![],
                content: None,
                encrypted_content: None,
                internal_chat_message_metadata_passthrough: None,
            },
        ],
        "",
    );
    assert_eq!(messages.len(), 1);
}

#[test]
fn unsupported_audio_input_is_not_emitted_as_chat_text() {
    let messages = responses_input_to_chat_messages(
        &[ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputAudio {
                audio_url: "data:audio/wav;base64,AA==".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        }],
        "",
    );

    assert!(messages.is_empty());
}

#[test]
fn full_conversation_round_trip() {
    // A representative agent loop fragment: assistant calls a tool, tool
    // returns a result, then assistant replies with text.
    let input = vec![
        user_msg("what's the weather?"),
        function_call("get_weather", r#"{"city":"SF"}"#, "call_1"),
        ResponseItem::FunctionCallOutput {
            id: None,
            call_id: "call_1".to_string(),
            output: FunctionCallOutputPayload::from_text("sunny".to_string()),
            internal_chat_message_metadata_passthrough: None,
        },
        assistant_msg("It's sunny in SF."),
    ];
    let messages = responses_input_to_chat_messages(&input, "be concise");

    // system, user, assistant(tool_calls), tool, assistant(text)
    assert_eq!(messages.len(), 5);
    assert!(matches!(messages[0], ChatMessage::Text { ref role, .. } if role == "system"));
    assert!(matches!(messages[3], ChatMessage::ToolResult { .. }));
}

#[test]
fn tool_result_is_emitted_immediately_after_tool_calls_when_text_intervenes() {
    let input = vec![
        user_msg("what's the weather?"),
        function_call("get_weather", r#"{"city":"SF"}"#, "call_1"),
        assistant_msg("Checking the forecast now."),
        ResponseItem::FunctionCallOutput {
            id: None,
            call_id: "call_1".to_string(),
            output: FunctionCallOutputPayload::from_text("sunny".to_string()),
            internal_chat_message_metadata_passthrough: None,
        },
        assistant_msg("It's sunny in SF."),
    ];
    let messages = responses_input_to_chat_messages(&input, "");

    let tool_call_idx = messages
        .iter()
        .position(|message| matches!(message, ChatMessage::AssistantWithToolCalls { .. }))
        .unwrap();
    let tool_result_idx = messages
        .iter()
        .position(|message| matches!(message, ChatMessage::ToolResult { .. }))
        .unwrap();
    assert_eq!(tool_result_idx, tool_call_idx + 1);
}

#[test]
fn convert_function_tool_preserves_schema() {
    let tools = vec![json!({
        "type": "function",
        "name": "get_weather",
        "description": "Get weather for a city",
        "strict": true,
        "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}
    })];
    let chat_tools = responses_tools_to_chat_tools(&tools);
    assert_eq!(chat_tools.len(), 1);
    let value = serde_json::to_value(&chat_tools[0]).unwrap();
    assert_eq!(value["type"], "function");
    assert_eq!(value["function"]["name"], "get_weather");
    assert_eq!(value["function"]["strict"], true);
    assert_eq!(value["function"]["parameters"]["type"], "object");
    assert_eq!(
        chat_tools[0].target,
        ChatToolTarget {
            name: "get_weather".to_string(),
            namespace: None,
        }
    );
}

#[test]
fn convert_namespace_functions_for_chat_and_preserves_dispatch_target() {
    let tools = vec![json!({
        "type": "namespace",
        "name": "mcp__map_cards",
        "description": "Map card tools.",
        "tools": [{
            "type": "function",
            "name": "create_map_card",
            "description": "Create a map card.",
            "strict": false,
            "parameters": {"type": "object", "properties": {"title": {"type": "string"}}}
        }]
    })];

    let chat_tools = responses_tools_to_chat_tools(&tools);

    assert_eq!(chat_tools.len(), 1);
    assert_eq!(
        chat_tools[0].function.name,
        "mcp__map_cards__create_map_card"
    );
    assert_eq!(
        chat_tools[0].function.description,
        "Map card tools.\n\nCreate a map card."
    );
    assert_eq!(
        chat_tools[0].target,
        ChatToolTarget {
            name: "create_map_card".to_string(),
            namespace: Some("mcp__map_cards".to_string()),
        }
    );
}

#[test]
fn convert_keeps_responses_only_tools_hidden() {
    let tools = vec![
        json!({"type": "web_search"}),
        json!({"type": "tool_search", "execution": "client", "parameters": {}}),
        json!({"type": "custom", "name": "freeform", "format": {"type": "grammar"}}),
        json!({"type": "function", "name": "ok", "description": "", "strict": false, "parameters": {}}),
        json!({"type": "image_generation", "output_format": "png"}),
    ];
    let chat_tools = responses_tools_to_chat_tools(&tools);
    assert_eq!(chat_tools.len(), 1);
    let value = serde_json::to_value(&chat_tools[0]).unwrap();
    assert_eq!(value["function"]["name"], "ok");
    // strict=false should be omitted (skip_serializing_if).
    assert!(value["function"].get("strict").is_none());
}

#[test]
fn convert_drops_duplicate_flattened_chat_names() {
    let tools = vec![
        json!({
            "type": "namespace",
            "name": "mcp__demo",
            "description": "",
            "tools": [{"type": "function", "name": "lookup", "parameters": {}}]
        }),
        json!({
            "type": "function",
            "name": "mcp__demo__lookup",
            "parameters": {}
        }),
    ];

    let chat_tools = responses_tools_to_chat_tools(&tools);

    assert_eq!(chat_tools.len(), 1);
    assert_eq!(
        chat_tools[0].target,
        ChatToolTarget {
            name: "lookup".to_string(),
            namespace: Some("mcp__demo".to_string()),
        }
    );
}

#[test]
fn strict_false_is_omitted_from_serialized_tool() {
    let value = json!({
        "type": "function",
        "name": "n",
        "description": "",
        "strict": false,
        "parameters": {}
    });
    let chat_tools = responses_tools_to_chat_tools(&[value]);
    let serialized = serde_json::to_value(&chat_tools[0]).unwrap();
    assert!(serialized["function"].get("strict").is_none());
}

fn responses_request_with_tools(tools: Option<Vec<serde_json::Value>>) -> ResponsesApiRequest {
    ResponsesApiRequest {
        model: "third-party-reasoning-model".to_string(),
        instructions: "be concise".to_string(),
        input: vec![user_msg("hello")],
        tools,
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: Some(Reasoning {
            effort: Some(ReasoningEffortConfig::Ultra),
            summary: None,
            context: None,
        }),
        store: false,
        stream: true,
        stream_options: None,
        include: Vec::new(),
        service_tier: Some("priority".to_string()),
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
    }
}

#[test]
fn responses_request_conversion_owns_chat_wire_policy() {
    let request = responses_request_with_tools(Some(vec![json!({
        "type": "function",
        "name": "lookup",
        "description": "Look something up",
        "parameters": {"type": "object"}
    })]));

    let chat = responses_request_to_chat_completions_request(request);

    assert_eq!(chat.model, "third-party-reasoning-model");
    assert_eq!(chat.tool_choice, "auto");
    assert_eq!(chat.tools.len(), 1);
    assert_eq!(chat.reasoning_effort, Some(ChatReasoningEffort::High));
    assert_eq!(chat.service_tier.as_deref(), Some("priority"));
    assert!(chat.stream);
    assert!(chat.stream_options.include_usage);
    assert!(matches!(
        chat.messages.first(),
        Some(ChatMessage::Text { role, content })
            if role == "system" && content == "be concise"
    ));
}

#[test]
fn responses_request_conversion_disables_tool_choice_without_chat_safe_tools() {
    let request = responses_request_with_tools(Some(vec![json!({
        "type": "custom",
        "name": "freeform"
    })]));

    let chat = responses_request_to_chat_completions_request(request);

    assert!(chat.tools.is_empty());
    assert_eq!(chat.tool_choice, "none");
}
