use super::*;
use crate::common::Reasoning;
use crate::common::ReasoningContext;
use crate::common::ResponsesApiTools;
use crate::common::TextControls;
use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::InternalChatMessageMetadataPassthrough;
use codex_protocol::models::ResponseItem;
use std::sync::Arc;

fn request(tools: Option<Vec<Value>>) -> ResponsesApiRequest {
    ResponsesApiRequest {
        model: "chat-model".to_string(),
        instructions: "be concise".to_string(),
        input: vec![text_message("user", "hello")],
        tools: tools.map(|tools| {
            ResponsesApiTools::from(Arc::from(serde_json::value::to_raw_value(&tools).unwrap()))
        }),
        tool_choice: "auto".to_string(),
        parallel_tool_calls: true,
        reasoning: None,
        store: false,
        stream: true,
        stream_options: None,
        include: Vec::new(),
        service_tier: None,
        prompt_cache_key: None,
        text: None,
        client_metadata: None,
    }
}

fn text_message(role: &str, text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: role.to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }
}

fn turn_metadata(turn_id: &str) -> InternalChatMessageMetadataPassthrough {
    InternalChatMessageMetadataPassthrough {
        turn_id: Some(turn_id.to_string()),
        ..Default::default()
    }
}

#[test]
fn translates_text_only_request_with_exact_controls() {
    let translated = responses_request_to_chat_completions_request(request(None)).unwrap();

    assert_eq!(
        translated,
        ChatCompletionsApiRequest {
            model: "chat-model".to_string(),
            messages: vec![
                ChatMessage::Text {
                    role: "system".to_string(),
                    content: "be concise".to_string(),
                },
                ChatMessage::Text {
                    role: "user".to_string(),
                    content: "hello".to_string(),
                },
            ],
            tools: Vec::new(),
            tool_choice: None,
            parallel_tool_calls: true,
            stream: true,
            stream_options: ChatStreamOptions {
                include_usage: true,
            },
            reasoning_effort: None,
            service_tier: None,
        }
    );
}

#[test]
fn maps_supported_chat_request_controls_without_approximation() {
    let tools = vec![serde_json::json!({
        "type": "function",
        "name": "route",
        "parameters": {"type": "object"}
    })];
    for (choice, expected_choice) in [
        ("auto", None),
        ("none", Some("none")),
        ("required", Some("required")),
    ] {
        let mut request = request(Some(tools.clone()));
        request.tool_choice = choice.to_string();
        request.parallel_tool_calls = false;
        request.service_tier = Some("priority".to_string());
        request.reasoning = Some(Reasoning {
            effort: Some(ReasoningEffortConfig::XHigh),
            summary: Some(ReasoningSummaryConfig::Auto),
            context: None,
        });

        let translated = responses_request_to_chat_completions_request(request).unwrap();
        assert_eq!(translated.tool_choice.as_deref(), expected_choice);
        assert!(!translated.parallel_tool_calls);
        assert_eq!(translated.service_tier.as_deref(), Some("priority"));
        assert_eq!(
            translated.reasoning_effort,
            Some(ChatReasoningEffort::XHigh)
        );
    }
}

#[test]
fn rejects_responses_controls_before_a_chat_request_is_created() {
    let mut non_streaming = request(None);
    non_streaming.stream = false;
    let mut server_store = request(None);
    server_store.store = true;
    let mut text_controls = request(None);
    text_controls.text = Some(TextControls::default());
    let mut unknown_include = request(None);
    unknown_include.include = vec!["future.include".to_string()];
    let mut reasoning_context = request(None);
    reasoning_context.reasoning = Some(Reasoning {
        effort: None,
        summary: None,
        context: Some(ReasoningContext::AllTurns),
    });

    for request in [
        non_streaming,
        server_store,
        text_controls,
        unknown_include,
        reasoning_context,
    ] {
        assert!(matches!(
            responses_request_to_chat_completions_request(request),
            Err(ApiError::InvalidRequest { .. })
        ));
    }
}

#[test]
fn preserves_only_exact_reasoning_effort_values() {
    for (effort, expected) in [
        (ReasoningEffortConfig::None, ChatReasoningEffort::None),
        (ReasoningEffortConfig::Minimal, ChatReasoningEffort::Minimal),
        (ReasoningEffortConfig::Low, ChatReasoningEffort::Low),
        (ReasoningEffortConfig::Medium, ChatReasoningEffort::Medium),
        (ReasoningEffortConfig::High, ChatReasoningEffort::High),
        (ReasoningEffortConfig::XHigh, ChatReasoningEffort::XHigh),
    ] {
        assert_eq!(chat_reasoning_effort(effort).unwrap(), expected);
    }
    for effort in [
        ReasoningEffortConfig::Max,
        ReasoningEffortConfig::Ultra,
        ReasoningEffortConfig::Custom("future".to_string()),
    ] {
        assert!(matches!(
            chat_reasoning_effort(effort),
            Err(ApiError::InvalidRequest { .. })
        ));
    }
}

#[test]
fn flattens_function_and_namespace_tools_with_reversible_targets() {
    let translated = responses_request_to_chat_completions_request(request(Some(vec![
        serde_json::json!({
            "type": "function",
            "name": "weather",
            "parameters": {"type": "object"}
        }),
        serde_json::json!({
            "type": "namespace",
            "name": "mcp__maps",
            "tools": [{
                "type": "function",
                "name": "route",
                "parameters": {"type": "object"}
            }]
        }),
    ])))
    .unwrap();

    assert_eq!(translated.tools[0].function.name, "weather");
    assert_eq!(translated.tools[1].function.name, "mcp__maps__route");
    assert_eq!(
        translated.tools[1].target,
        ChatToolTarget {
            name: "route".to_string(),
            namespace: Some("mcp__maps".to_string()),
        }
    );
}

#[test]
fn translates_history_loaded_tool_search_into_chat_tools_and_reverse_targets() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object", "properties": {"query": {"type": "string"}}}
    })]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Use the network baseline tool.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "network coverage"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "function",
                "name": "evaluate_network_baseline",
                "description": "Evaluate a network baseline.",
                "parameters": {"type": "object", "properties": {}}
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
    ];

    let translated = responses_request_to_chat_completions_request(request).unwrap();
    assert!(!translated.parallel_tool_calls);
    assert_eq!(
        translated
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>(),
        vec!["tool_search", "evaluate_network_baseline"]
    );
    assert_eq!(
        translated.messages,
        vec![
            ChatMessage::Text {
                role: "user".to_string(),
                content: "Use the network baseline tool.".to_string(),
            },
            ChatMessage::Assistant {
                role: "assistant".to_string(),
                content: String::new(),
                reasoning_content: None,
                tool_calls: Some(vec![ChatToolCall {
                    id: "search-1".to_string(),
                    r#type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: "tool_search".to_string(),
                        arguments: "{\"query\":\"network coverage\"}".to_string(),
                    },
                }]),
            },
            ChatMessage::ToolResult {
                role: "tool".to_string(),
                tool_call_id: "search-1".to_string(),
                content: "[{\"description\":\"Evaluate a network baseline.\",\"name\":\"evaluate_network_baseline\",\"parameters\":{\"properties\":{},\"type\":\"object\"},\"type\":\"function\"}]".to_string(),
            },
        ]
    );
    assert_eq!(
        translated.tools[1].target,
        ChatToolTarget {
            name: "evaluate_network_baseline".to_string(),
            namespace: None,
        }
    );
}

#[test]
fn keeps_history_loaded_deferred_target_after_inter_agent_completion_message() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object"}
    })]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Plan the network.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "spawn agent"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "namespace",
                "name": "multi_agent_v1",
                "description": "Native Agent collaboration.",
                "tools": [{
                    "type": "function",
                    "name": "spawn_agent",
                    "description": "Start a child Agent.",
                    "parameters": {"type": "object", "properties": {}}
                }]
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        // Inter-Agent completion is represented as a user-role Chat message,
        // but its typed metadata keeps it inside the same Root Turn.
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Data Agent completed the prepared input handoff.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
    ];

    let translated = responses_request_to_chat_completions_request(request).unwrap();
    assert!(
        translated
            .tools
            .iter()
            .any(|tool| tool.function.name == "multi_agent_v1__spawn_agent")
    );
    assert_eq!(
        translated
            .tools
            .iter()
            .find(|tool| tool.function.name == "multi_agent_v1__spawn_agent")
            .map(|tool| &tool.target),
        Some(&ChatToolTarget {
            name: "spawn_agent".to_string(),
            namespace: Some("multi_agent_v1".to_string()),
        })
    );
}

#[test]
fn replays_history_loaded_target_when_latest_user_message_lacks_turn_metadata() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object"}
    })]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Plan the network.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "spawn agent"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "namespace",
                "name": "multi_agent_v1",
                "tools": [{
                    "type": "function",
                    "name": "spawn_agent",
                    "parameters": {"type": "object"}
                }]
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Unidentified mailbox message.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    let translated = responses_request_to_chat_completions_request(request).unwrap();
    assert_eq!(
        translated
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>(),
        vec!["tool_search", "multi_agent_v1__spawn_agent"]
    );
}

#[test]
fn replays_prior_turn_loaded_tool_schema_for_resume_shaped_history() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object", "properties": {"query": {"type": "string"}}}
    })]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-old".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "network coverage"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-old")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-old".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "function",
                "name": "evaluate_network_baseline",
                "description": "Evaluate a network baseline.",
                "parameters": {"type": "object", "properties": {}}
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-old")),
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Use navigation distances now.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-new")),
        },
    ];

    let translated = responses_request_to_chat_completions_request(request).unwrap();
    assert_eq!(
        translated
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>(),
        vec!["tool_search", "evaluate_network_baseline"],
        "a resumed Turn keeps completed loaded Tool schemas callable"
    );
    assert_eq!(
        translated.tools[1],
        ChatTool {
            r#type: "function".to_string(),
            function: ChatToolFunction {
                name: "evaluate_network_baseline".to_string(),
                description: "Evaluate a network baseline.".to_string(),
                strict: false,
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
            target: ChatToolTarget {
                name: "evaluate_network_baseline".to_string(),
                namespace: None,
            },
        }
    );
}

#[test]
fn deduplicates_the_same_history_loaded_tool_across_turns() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object"}
    })]));
    request.instructions.clear();
    let route_tool = serde_json::json!({
        "type": "function",
        "name": "route",
        "description": "Calculate a route.",
        "parameters": {"type": "object", "properties": {}}
    });
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Find route tools.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-old")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-old".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "route"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-old")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-old".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![route_tool.clone()],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-old")),
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Resume and use route again.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-new")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-new".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "route"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-new")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-new".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![route_tool],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-new")),
        },
    ];

    let translated = responses_request_to_chat_completions_request(request).unwrap();
    assert_eq!(
        translated
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>(),
        vec!["tool_search", "route"]
    );
}

#[test]
fn does_not_project_empty_history_loaded_tool_search_output() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object"}
    })]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Search for an unavailable tool.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-old")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-empty".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "missing"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-old")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-empty".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: Vec::new(),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-old")),
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Continue.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-new")),
        },
    ];

    let translated = responses_request_to_chat_completions_request(request).unwrap();
    assert_eq!(
        translated
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>(),
        vec!["tool_search"]
    );
}

#[test]
fn rejects_orphan_history_loaded_tool_search_output_before_target_mapping() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object"}
    })]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Use the route tool.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-orphan".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "function",
                "name": "route",
                "parameters": {"type": "object"}
            })],
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert!(matches!(
        responses_request_to_chat_completions_request(request),
        Err(ApiError::InvalidRequest { message })
            if message.contains("history tool_search result without its preceding tool-call group")
    ));
}

#[test]
fn rejects_failed_tool_search_output_before_target_mapping() {
    for status in ["failed", "cancelled"] {
        let mut request = request(Some(vec![serde_json::json!({
            "type": "tool_search",
            "execution": "client",
            "description": "Search available tools.",
            "parameters": {"type": "object"}
        })]));
        request.instructions.clear();
        request.input = vec![
            ResponseItem::Message {
                id: None,
                role: "user".to_string(),
                content: vec![ContentItem::InputText {
                    text: "Use the route tool.".to_string(),
                }],
                phase: None,
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::ToolSearchCall {
                id: None,
                call_id: Some(format!("search-{status}")),
                status: None,
                execution: "client".to_string(),
                arguments: serde_json::json!({"query": "route"}),
                internal_chat_message_metadata_passthrough: None,
            },
            ResponseItem::ToolSearchOutput {
                id: None,
                call_id: Some(format!("search-{status}")),
                status: status.to_string(),
                execution: "client".to_string(),
                tools: vec![serde_json::json!({
                    "type": "function",
                    "name": "route",
                    "parameters": {"type": "object"}
                })],
                internal_chat_message_metadata_passthrough: None,
            },
        ];

        assert!(matches!(
            responses_request_to_chat_completions_request(request),
            Err(ApiError::InvalidRequest { message })
                if message.contains("non-client completed tool_search output")
        ));
    }
}

#[test]
fn encodes_a_loaded_namespace_from_prompt_once() {
    let mut request = request(Some(vec![
        serde_json::json!({
            "type": "tool_search",
            "execution": "client",
            "description": "Search available tools.",
            "parameters": {"type": "object"}
        }),
        serde_json::json!({
            "type": "namespace",
            "name": "mcp__supply_chain",
            "description": "Warehouse network tools.",
            "tools": [{
                "type": "function",
                "name": "compare_network_scenarios",
                "description": "Compare network scenarios.",
                "parameters": {"type": "object", "properties": {}}
            }]
        }),
    ]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Compare the network.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "compare network"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "namespace",
                "name": "mcp__supply_chain",
                "description": "Warehouse network tools.",
                "tools": [{
                    "type": "function",
                    "name": "compare_network_scenarios",
                    "description": "Compare network scenarios.",
                    "parameters": {"type": "object", "properties": {}}
                }]
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
    ];

    let translated = responses_request_to_chat_completions_request(request).unwrap();
    assert_eq!(
        translated
            .tools
            .iter()
            .map(|tool| tool.function.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "tool_search",
            "mcp__supply_chain__compare_network_scenarios"
        ]
    );
    assert_eq!(
        translated.tools[1].target,
        ChatToolTarget {
            name: "compare_network_scenarios".to_string(),
            namespace: Some("mcp__supply_chain".to_string()),
        }
    );
}

#[test]
fn rejects_colliding_history_loaded_deferred_tool_targets() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object"}
    })]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Use the matching tool.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "one"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "namespace",
                "name": "a__b",
                "description": "first",
                "tools": [{
                    "type": "function",
                    "name": "c",
                    "parameters": {"type": "object"}
                }]
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-2".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "two"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-2".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "namespace",
                "name": "a",
                "description": "second",
                "tools": [{
                    "type": "function",
                    "name": "b__c",
                    "parameters": {"type": "object"}
                }]
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
    ];

    assert!(matches!(
        responses_request_to_chat_completions_request(request),
        Err(ApiError::InvalidRequest { message }) if message.contains("colliding deferred tool target")
    ));
}

#[test]
fn rejects_repeated_history_loaded_deferred_tool_with_different_schema() {
    let mut request = request(Some(vec![serde_json::json!({
        "type": "tool_search",
        "execution": "client",
        "description": "Search available tools.",
        "parameters": {"type": "object"}
    })]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Use the route tool.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "route"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "function",
                "name": "route",
                "parameters": {"type": "object"}
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-2".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "route details"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-2".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "function",
                "name": "route",
                "parameters": {
                    "type": "object",
                    "properties": {"mode": {"type": "string"}}
                }
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
    ];

    assert!(matches!(
        responses_request_to_chat_completions_request(request),
        Err(ApiError::InvalidRequest { message }) if message.contains("multiple schemas")
    ));
}

#[test]
fn rejects_history_loaded_deferred_tool_schema_conflict_with_prompt_tool() {
    let mut request = request(Some(vec![
        serde_json::json!({
            "type": "tool_search",
            "execution": "client",
            "description": "Search available tools.",
            "parameters": {"type": "object"}
        }),
        serde_json::json!({
            "type": "function",
            "name": "route",
            "parameters": {"type": "object"}
        }),
    ]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Use the route tool.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "route"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "function",
                "name": "route",
                "parameters": {
                    "type": "object",
                    "properties": {"mode": {"type": "string"}}
                }
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
    ];

    assert!(matches!(
        responses_request_to_chat_completions_request(request),
        Err(ApiError::InvalidRequest { message }) if message.contains("multiple schemas")
    ));
}

#[test]
fn rejects_history_loaded_deferred_flatten_collision_with_prompt_tool() {
    let mut request = request(Some(vec![
        serde_json::json!({
            "type": "tool_search",
            "execution": "client",
            "description": "Search available tools.",
            "parameters": {"type": "object"}
        }),
        serde_json::json!({
            "type": "function",
            "name": "a__b__c",
            "parameters": {"type": "object"}
        }),
    ]));
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: "Use the matching tool.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchCall {
            id: None,
            call_id: Some("search-1".to_string()),
            status: None,
            execution: "client".to_string(),
            arguments: serde_json::json!({"query": "matching"}),
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
        ResponseItem::ToolSearchOutput {
            id: None,
            call_id: Some("search-1".to_string()),
            status: "completed".to_string(),
            execution: "client".to_string(),
            tools: vec![serde_json::json!({
                "type": "namespace",
                "name": "a__b",
                "tools": [{
                    "type": "function",
                    "name": "c",
                    "parameters": {"type": "object"}
                }]
            })],
            internal_chat_message_metadata_passthrough: Some(turn_metadata("turn-current")),
        },
    ];

    assert!(matches!(
        responses_request_to_chat_completions_request(request),
        Err(ApiError::InvalidRequest { message }) if message.contains("colliding deferred tool target")
    ));
}

#[test]
fn rejects_an_unnamespaced_function_that_collides_with_native_tool_search() {
    let request = request(Some(vec![serde_json::json!({
        "type": "function",
        "name": "tool_search",
        "parameters": {"type": "object"}
    })]));

    assert!(matches!(
        responses_request_to_chat_completions_request(request),
        Err(ApiError::InvalidRequest { .. })
    ));
}

#[test]
fn groups_raw_reasoning_assistant_text_and_tool_calls() {
    let mut request = request(None);
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Reasoning {
            id: None,
            summary: Vec::new(),
            content: Some(vec![
                codex_protocol::models::ReasoningItemContent::ReasoningText {
                    text: "check route coordinates".to_string(),
                },
            ]),
            encrypted_content: None,
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: "Checking the route.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
        function_call("route", Some("mcp__maps"), "call_1"),
        ResponseItem::FunctionCallOutput {
            id: None,
            call_id: "call_1".to_string(),
            output: FunctionCallOutputPayload::from_text("12 km".to_string()),
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert_eq!(
        responses_request_to_chat_completions_request(request)
            .unwrap()
            .messages,
        vec![
            ChatMessage::Assistant {
                role: "assistant".to_string(),
                content: "Checking the route.".to_string(),
                reasoning_content: Some("check route coordinates".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    r#type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: "mcp__maps__route".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
            },
            ChatMessage::ToolResult {
                role: "tool".to_string(),
                tool_call_id: "call_1".to_string(),
                content: "12 km".to_string(),
            },
        ]
    );
}

#[test]
fn groups_raw_reasoning_tool_only_and_output() {
    let mut request = request(None);
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Reasoning {
            id: None,
            summary: Vec::new(),
            content: Some(vec![
                codex_protocol::models::ReasoningItemContent::ReasoningText {
                    text: "check route coordinates".to_string(),
                },
            ]),
            encrypted_content: None,
            internal_chat_message_metadata_passthrough: None,
        },
        function_call("route", Some("mcp__maps"), "call_1"),
        ResponseItem::FunctionCallOutput {
            id: None,
            call_id: "call_1".to_string(),
            output: FunctionCallOutputPayload::from_text("12 km".to_string()),
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert_eq!(
        responses_request_to_chat_completions_request(request)
            .unwrap()
            .messages,
        vec![
            ChatMessage::Assistant {
                role: "assistant".to_string(),
                content: String::new(),
                reasoning_content: Some("check route coordinates".to_string()),
                tool_calls: Some(vec![ChatToolCall {
                    id: "call_1".to_string(),
                    r#type: "function".to_string(),
                    function: ChatToolCallFunction {
                        name: "mcp__maps__route".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
            },
            ChatMessage::ToolResult {
                role: "tool".to_string(),
                tool_call_id: "call_1".to_string(),
                content: "12 km".to_string(),
            },
        ]
    );
}

#[test]
fn preserves_each_supported_message_item_without_merging_or_text_rewrites() {
    let mut request = request(None);
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![
                ContentItem::InputText {
                    text: " first".to_string(),
                },
                ContentItem::InputText {
                    text: "\nsecond ".to_string(),
                },
            ],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::Message {
            id: None,
            role: "user".to_string(),
            content: vec![ContentItem::InputText {
                text: String::new(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: " answer ".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert_eq!(
        responses_request_to_chat_completions_request(request)
            .unwrap()
            .messages,
        vec![
            ChatMessage::Text {
                role: "user".to_string(),
                content: " first\nsecond ".to_string(),
            },
            ChatMessage::Text {
                role: "user".to_string(),
                content: String::new(),
            },
            ChatMessage::Assistant {
                role: "assistant".to_string(),
                content: " answer ".to_string(),
                reasoning_content: None,
                tool_calls: None,
            },
        ]
    );
}

#[test]
fn preserves_system_and_developer_sources_in_order_as_distinct_chat_messages() {
    let mut request = request(None);
    request.instructions = "top-level instructions".to_string();
    request.input = vec![
        text_message("system", "system source"),
        text_message("developer", "developer source"),
        text_message("user", "user source"),
    ];

    assert_eq!(
        responses_request_to_chat_completions_request(request)
            .unwrap()
            .messages,
        vec![
            ChatMessage::Text {
                role: "system".to_string(),
                content: "top-level instructions".to_string(),
            },
            ChatMessage::Text {
                role: "system".to_string(),
                content: "system source".to_string(),
            },
            ChatMessage::Text {
                role: "system".to_string(),
                content: "developer source".to_string(),
            },
            ChatMessage::Text {
                role: "user".to_string(),
                content: "user source".to_string(),
            },
        ]
    );
}

#[test]
fn replays_final_raw_reasoning_with_its_following_assistant_message() {
    let mut request = request(None);
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Reasoning {
            id: None,
            summary: Vec::new(),
            content: Some(vec![
                codex_protocol::models::ReasoningItemContent::ReasoningText {
                    text: "inspect the route first".to_string(),
                },
            ]),
            encrypted_content: None,
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::Message {
            id: None,
            role: "assistant".to_string(),
            content: vec![ContentItem::OutputText {
                text: "The route is safe.".to_string(),
            }],
            phase: None,
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert_eq!(
        responses_request_to_chat_completions_request(request)
            .unwrap()
            .messages,
        vec![ChatMessage::Assistant {
            role: "assistant".to_string(),
            content: "The route is safe.".to_string(),
            reasoning_content: Some("inspect the route first".to_string()),
            tool_calls: None,
        }]
    );
}

#[test]
fn preserves_interrupted_reasoning_and_its_following_plaintext_agent_message() {
    let mut request = request(None);
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Reasoning {
            id: None,
            summary: Vec::new(),
            content: Some(vec![
                codex_protocol::models::ReasoningItemContent::ReasoningText {
                    text: "check the prepared route matrix".to_string(),
                },
            ]),
            encrypted_content: None,
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::AgentMessage {
            id: None,
            author: "network_agent".to_string(),
            recipient: "root".to_string(),
            content: vec![
                codex_protocol::models::AgentMessageInputContent::InputText {
                    text: "The route matrix is ready.".to_string(),
                },
            ],
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert_eq!(
        responses_request_to_chat_completions_request(request)
            .unwrap()
            .messages,
        vec![
            ChatMessage::Assistant {
                role: "assistant".to_string(),
                content: String::new(),
                reasoning_content: Some("check the prepared route matrix".to_string()),
                tool_calls: None,
            },
            ChatMessage::Text {
                role: "assistant".to_string(),
                content: "The route matrix is ready.".to_string(),
            },
        ]
    );
}

#[test]
fn replays_interrupted_raw_reasoning_before_the_next_user_message() {
    let mut request = request(None);
    request.instructions.clear();
    request.input = vec![
        ResponseItem::Reasoning {
            id: None,
            summary: Vec::new(),
            content: Some(vec![
                codex_protocol::models::ReasoningItemContent::ReasoningText {
                    text: "wait for the network result".to_string(),
                },
            ]),
            encrypted_content: None,
            internal_chat_message_metadata_passthrough: None,
        },
        text_message("user", "Continue after the network result."),
    ];

    assert_eq!(
        responses_request_to_chat_completions_request(request)
            .unwrap()
            .messages,
        vec![
            ChatMessage::Assistant {
                role: "assistant".to_string(),
                content: String::new(),
                reasoning_content: Some("wait for the network result".to_string()),
                tool_calls: None,
            },
            ChatMessage::Text {
                role: "user".to_string(),
                content: "Continue after the network result.".to_string(),
            },
        ]
    );
}

#[test]
fn rejects_chat_history_items_without_typed_chat_equivalents() {
    let mut unsupported_role = request(None);
    unsupported_role.input = vec![text_message("tool", "result")];
    let mut mismatched_content = request(None);
    mismatched_content.input = vec![ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::InputText {
            text: "not assistant output".to_string(),
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }];
    let mut encrypted_agent_message = request(None);
    encrypted_agent_message.input = vec![ResponseItem::AgentMessage {
        id: None,
        author: "planner".to_string(),
        recipient: "reviewer".to_string(),
        content: vec![
            codex_protocol::models::AgentMessageInputContent::EncryptedContent {
                encrypted_content: "ciphertext".to_string(),
            },
        ],
        internal_chat_message_metadata_passthrough: None,
    }];

    for request in [
        unsupported_role,
        mismatched_content,
        encrypted_agent_message,
    ] {
        assert!(matches!(
            responses_request_to_chat_completions_request(request),
            Err(ApiError::InvalidRequest { .. })
        ));
    }
}

#[test]
fn maps_only_plaintext_native_agent_messages_to_assistant_history() {
    let mut request = request(None);
    request.instructions.clear();
    request.input = vec![
        ResponseItem::AgentMessage {
            id: None,
            author: "planner".to_string(),
            recipient: "reviewer".to_string(),
            content: vec![
                codex_protocol::models::AgentMessageInputContent::InputText {
                    text: "first line".to_string(),
                },
                codex_protocol::models::AgentMessageInputContent::InputText {
                    text: "second line".to_string(),
                },
            ],
            internal_chat_message_metadata_passthrough: None,
        },
        ResponseItem::AgentMessage {
            id: None,
            author: "reviewer".to_string(),
            recipient: "planner".to_string(),
            content: vec![
                codex_protocol::models::AgentMessageInputContent::InputText {
                    text: "reply".to_string(),
                },
            ],
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    assert_eq!(
        responses_request_to_chat_completions_request(request)
            .unwrap()
            .messages,
        vec![
            ChatMessage::Text {
                role: "assistant".to_string(),
                content: "first line\nsecond line".to_string(),
            },
            ChatMessage::Text {
                role: "assistant".to_string(),
                content: "reply".to_string(),
            },
        ]
    );
}

#[test]
fn rejects_non_text_or_incomplete_chat_history() {
    let mut non_text = request(None);
    non_text.input = vec![ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputImage {
            image_url: "data:image/png;base64,AA==".to_string(),
            detail: None,
        }],
        phase: None,
        internal_chat_message_metadata_passthrough: None,
    }];
    let mut reasoning = request(None);
    reasoning.input = vec![ResponseItem::Reasoning {
        id: None,
        summary: Vec::new(),
        content: None,
        encrypted_content: None,
        internal_chat_message_metadata_passthrough: None,
    }];
    let mut incomplete_tool_group = request(None);
    incomplete_tool_group.instructions.clear();
    incomplete_tool_group.input = vec![function_call("route", None, "call_1")];

    for request in [non_text, reasoning, incomplete_tool_group] {
        assert!(matches!(
            responses_request_to_chat_completions_request(request),
            Err(ApiError::InvalidRequest { .. })
        ));
    }
}

fn function_call(name: &str, namespace: Option<&str>, call_id: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: name.to_string(),
        namespace: namespace.map(str::to_string),
        arguments: "{}".to_string(),
        encrypted_function_args: None,
        call_id: call_id.to_string(),
        internal_chat_message_metadata_passthrough: None,
    }
}

#[test]
fn wraps_unparsable_tool_arguments_only_on_the_chat_wire() {
    let mut request = request(None);
    request.instructions.clear();
    let ResponseItem::FunctionCall {
        id,
        name,
        namespace,
        encrypted_function_args,
        call_id,
        internal_chat_message_metadata_passthrough,
        ..
    } = function_call("route", None, "call_1")
    else {
        unreachable!("function_call builds a FunctionCall");
    };
    request.input = vec![
        ResponseItem::FunctionCall {
            id,
            name,
            namespace,
            arguments: "{\"x\":{\"x\":".to_string(),
            encrypted_function_args,
            call_id,
            internal_chat_message_metadata_passthrough,
        },
        ResponseItem::FunctionCallOutput {
            id: None,
            call_id: "call_1".to_string(),
            output: FunctionCallOutputPayload::from_text(
                "failed to parse function arguments".to_string(),
            ),
            internal_chat_message_metadata_passthrough: None,
        },
    ];

    let messages = responses_request_to_chat_completions_request(request)
        .unwrap()
        .messages;
    let Some(ChatMessage::Assistant {
        tool_calls: Some(tool_calls),
        ..
    }) = messages.first()
    else {
        panic!("expected an assistant tool call: {messages:?}");
    };
    let wire: Value = serde_json::from_str(&tool_calls[0].function.arguments).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({ "unparsed_arguments": "{\"x\":{\"x\":" })
    );
}
