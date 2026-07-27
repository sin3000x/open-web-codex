use crate::chat_translate::ChatToolTarget;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ContentItem;
use codex_protocol::models::MessagePhase;
use codex_protocol::models::ResponseItem;
use codex_protocol::protocol::TokenUsage;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;
use tracing::trace;

const REQUEST_ID_HEADER: &str = "x-request-id";
const OPENAI_MODEL_HEADER: &str = "openai-model";

pub fn spawn_chat_response_stream(
    stream_response: StreamResponse,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    tool_targets: HashMap<String, ChatToolTarget>,
) -> ResponseStream {
    let upstream_request_id = stream_response
        .headers
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let server_model = stream_response
        .headers
        .get(OPENAI_MODEL_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let (tx_event, rx_event) = mpsc::channel::<Result<ResponseEvent, ApiError>>(1600);
    tokio::spawn(async move {
        if let Some(model) = server_model {
            let _ = tx_event.send(Ok(ResponseEvent::ServerModel(model))).await;
        }
        process_chat_sse(
            stream_response.bytes,
            tx_event,
            idle_timeout,
            telemetry,
            tool_targets,
        )
        .await;
    });

    ResponseStream {
        rx_event,
        upstream_request_id,
    }
}

pub async fn process_chat_sse(
    stream: ByteStream,
    tx_event: mpsc::Sender<Result<ResponseEvent, ApiError>>,
    idle_timeout: Duration,
    telemetry: Option<Arc<dyn SseTelemetry>>,
    tool_targets: HashMap<String, ChatToolTarget>,
) {
    let mut stream = stream.eventsource();
    let mut state = ChatStreamState::default();

    loop {
        let start = Instant::now();
        let response = timeout(idle_timeout, stream.next()).await;
        if let Some(t) = telemetry.as_ref() {
            t.on_sse_poll(&response, start.elapsed());
        }
        let sse = match response {
            Ok(Some(Ok(sse))) => sse,
            Ok(Some(Err(e))) => {
                debug!("Chat SSE error: {e:#}");
                let _ = tx_event.send(Err(ApiError::Stream(e.to_string()))).await;
                return;
            }
            Ok(None) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream(
                        "stream closed before chat completion finished".to_string(),
                    )))
                    .await;
                return;
            }
            Err(_) => {
                let _ = tx_event
                    .send(Err(ApiError::Stream("idle timeout waiting for SSE".into())))
                    .await;
                return;
            }
        };

        trace!("Chat SSE event: {}", &sse.data);
        if sse.data.trim() == "[DONE]" {
            finish_chat_stream(&tx_event, state, &tool_targets).await;
            return;
        }

        let event: ChatCompletionChunk = match serde_json::from_str(&sse.data) {
            Ok(event) => event,
            Err(e) => {
                debug!("Failed to parse chat SSE event: {e}, data: {}", &sse.data);
                continue;
            }
        };
        state.response_id = state.response_id.or(event.id);
        if let Some(model) = event.model {
            let _ = tx_event.send(Ok(ResponseEvent::ServerModel(model))).await;
        }
        if let Some(usage) = event.usage {
            state.token_usage = Some(usage.into());
        }

        for choice in event.choices {
            if let Some(delta) = choice.delta.content
                && !delta.is_empty()
            {
                if !state.assistant_item_started {
                    let item = state.assistant_message_item(String::new());
                    if tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(item)))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    state.assistant_item_started = true;
                }
                state.assistant_text.push_str(&delta);
                if tx_event
                    .send(Ok(ResponseEvent::OutputTextDelta(delta)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            for tool_call in choice.delta.tool_calls {
                state.merge_tool_call(tool_call);
            }
        }
    }
}

async fn finish_chat_stream(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    state: ChatStreamState,
    tool_targets: &HashMap<String, ChatToolTarget>,
) {
    let ChatStreamState {
        response_id,
        assistant_text,
        tool_calls,
        token_usage,
        ..
    } = state;

    let assistant_phase = if tool_calls.is_empty() {
        MessagePhase::FinalAnswer
    } else {
        MessagePhase::Commentary
    };
    for tool_call in tool_calls {
        let wire_name = tool_call.function.name;
        let target = tool_targets.get(&wire_name);
        let item = ResponseItem::FunctionCall {
            id: Some(codex_protocol::ResponseItemId::from_server(
                tool_call.id.clone(),
            )),
            name: target
                .map(|target| target.name.clone())
                .unwrap_or_else(|| wire_name.clone()),
            namespace: target.and_then(|target| target.namespace.clone()),
            arguments: tool_call.function.arguments,
            call_id: tool_call.id,
            internal_chat_message_metadata_passthrough: None,
        };
        if tx_event
            .send(Ok(ResponseEvent::OutputItemAdded(item.clone())))
            .await
            .is_err()
        {
            return;
        }
        if tx_event
            .send(Ok(ResponseEvent::OutputItemDone(item)))
            .await
            .is_err()
        {
            return;
        }
    }
    if !assistant_text.is_empty() {
        let item = assistant_message_item(
            response_id.as_deref(),
            assistant_text,
            Some(assistant_phase),
        );
        if tx_event
            .send(Ok(ResponseEvent::OutputItemDone(item)))
            .await
            .is_err()
        {
            return;
        }
    }
    let _ = tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id: response_id.unwrap_or_else(|| "chatcmpl".to_string()),
            token_usage,
            end_turn: None,
        }))
        .await;
}

fn assistant_message_item(
    response_id: Option<&str>,
    text: String,
    phase: Option<MessagePhase>,
) -> ResponseItem {
    let id = response_id.unwrap_or("chatcmpl").to_string() + "-message";
    ResponseItem::Message {
        id: Some(codex_protocol::ResponseItemId::from_server(id)),
        role: "assistant".to_string(),
        content: if text.is_empty() {
            Vec::new()
        } else {
            vec![ContentItem::OutputText { text }]
        },
        phase,
        internal_chat_message_metadata_passthrough: None,
    }
}

#[derive(Default)]
struct ChatStreamState {
    response_id: Option<String>,
    assistant_text: String,
    assistant_item_started: bool,
    tool_calls: Vec<AccumulatedToolCall>,
    token_usage: Option<TokenUsage>,
}

impl ChatStreamState {
    fn assistant_message_item(&self, text: String) -> ResponseItem {
        assistant_message_item(self.response_id.as_deref(), text, None)
    }

    fn merge_tool_call(&mut self, delta: ChatToolCallDelta) {
        let index = delta.index.unwrap_or(self.tool_calls.len());
        while self.tool_calls.len() <= index {
            self.tool_calls.push(AccumulatedToolCall::default());
        }
        let tool_call = &mut self.tool_calls[index];
        if let Some(id) = delta.id {
            tool_call.id = id;
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name {
                tool_call.function.name = name;
            }
            if let Some(arguments) = function.arguments {
                tool_call.function.arguments.push_str(&arguments);
            }
        }
    }
}

#[derive(Default)]
struct AccumulatedToolCall {
    id: String,
    function: AccumulatedToolCallFunction,
}

#[derive(Default)]
struct AccumulatedToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChunk {
    id: Option<String>,
    model: Option<String>,
    #[serde(default)]
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    delta: ChatDelta,
}

#[derive(Debug, Default, Deserialize)]
struct ChatDelta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ChatToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCallDelta {
    index: Option<usize>,
    id: Option<String>,
    function: Option<ChatToolCallFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct ChatToolCallFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatUsage {
    #[serde(alias = "prompt_tokens")]
    input_tokens: i64,
    #[serde(default)]
    input_tokens_details: Option<ChatInputTokensDetails>,
    #[serde(alias = "completion_tokens")]
    output_tokens: i64,
    #[serde(default)]
    output_tokens_details: Option<ChatOutputTokensDetails>,
    #[serde(alias = "total_tokens")]
    total_tokens: i64,
}

impl From<ChatUsage> for TokenUsage {
    fn from(value: ChatUsage) -> Self {
        Self {
            input_tokens: value.input_tokens,
            cached_input_tokens: value
                .input_tokens_details
                .map(|details| details.cached_tokens)
                .unwrap_or(0),
            cache_write_input_tokens: 0,
            output_tokens: value.output_tokens,
            reasoning_output_tokens: value
                .output_tokens_details
                .map(|details| details.reasoning_tokens)
                .unwrap_or(0),
            total_tokens: value.total_tokens,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ChatInputTokensDetails {
    #[serde(default)]
    cached_tokens: i64,
}

#[derive(Debug, Deserialize)]
struct ChatOutputTokensDetails {
    #[serde(default)]
    reasoning_tokens: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use codex_client::TransportError;
    use futures::TryStreamExt;
    use pretty_assertions::assert_eq;
    use tokio_test::io::Builder as IoBuilder;
    use tokio_util::io::ReaderStream;

    async fn collect_events(body: &str) -> Vec<ResponseEvent> {
        collect_events_with_tool_targets(body, HashMap::new()).await
    }

    async fn collect_events_with_tool_targets(
        body: &str,
        tool_targets: HashMap<String, ChatToolTarget>,
    ) -> Vec<ResponseEvent> {
        let mut builder = IoBuilder::new();
        builder.read(body.as_bytes());
        let reader = builder.build();
        let stream =
            ReaderStream::new(reader).map_err(|err| TransportError::Network(err.to_string()));
        let (tx, mut rx) = mpsc::channel::<Result<ResponseEvent, ApiError>>(16);
        tokio::spawn(process_chat_sse(
            Box::pin(stream),
            tx,
            Duration::from_secs(1),
            /*telemetry*/ None,
            tool_targets,
        ));

        let mut events = Vec::new();
        while let Some(event) = rx.recv().await {
            events.push(event.expect("chat event should parse"));
        }
        events
    }

    #[tokio::test]
    async fn chat_sse_emits_text_tool_call_and_usage() {
        let body = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"cmd\\\":\"}}]}}]}\n\n",
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"pwd\\\"}\"}}]}}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );

        let events = collect_events(body).await;
        assert!(matches!(
            events[0],
            ResponseEvent::ServerModel(ref model) if model == "deepseek-chat"
        ));
        assert!(matches!(
            events[1],
            ResponseEvent::OutputItemAdded(ResponseItem::Message { .. })
        ));
        assert!(matches!(
            events[2],
            ResponseEvent::OutputTextDelta(ref delta) if delta == "hel"
        ));
        assert!(matches!(
            events[3],
            ResponseEvent::OutputTextDelta(ref delta) if delta == "lo"
        ));
        assert!(matches!(
            events[4],
            ResponseEvent::OutputItemAdded(ResponseItem::FunctionCall { .. })
        ));
        let ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
            name,
            arguments,
            call_id,
            ..
        }) = &events[5]
        else {
            panic!("expected function call, got {:?}", events[5]);
        };
        assert_eq!(
            (name.as_str(), arguments.as_str(), call_id.as_str()),
            ("shell", "{\"cmd\":\"pwd\"}", "call_1")
        );
        let ResponseEvent::OutputItemDone(ResponseItem::Message { content, phase, .. }) =
            &events[6]
        else {
            panic!("expected assistant message, got {:?}", events[6]);
        };
        assert_eq!(
            content,
            &vec![ContentItem::OutputText {
                text: "hello".to_string()
            }]
        );
        assert_eq!(phase, &Some(MessagePhase::Commentary));
        let ResponseEvent::Completed {
            response_id,
            token_usage,
            ..
        } = &events[7]
        else {
            panic!("expected completed, got {:?}", events[7]);
        };
        assert_eq!(response_id, "chatcmpl-1");
        assert_eq!(
            token_usage.as_ref(),
            Some(&TokenUsage {
                input_tokens: 2,
                cached_input_tokens: 0,
                cache_write_input_tokens: 0,
                output_tokens: 3,
                reasoning_output_tokens: 0,
                total_tokens: 5,
            })
        );
    }

    #[tokio::test]
    async fn chat_sse_restores_namespaced_tool_target() {
        let body = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"mcp__map_cards__create_map_card\",\"arguments\":\"{}\"}}]}}]}\n\n",
            "data: [DONE]\n\n",
        );
        let tool_targets = HashMap::from([(
            "mcp__map_cards__create_map_card".to_string(),
            ChatToolTarget {
                name: "create_map_card".to_string(),
                namespace: Some("mcp__map_cards".to_string()),
            },
        )]);

        let events = collect_events_with_tool_targets(body, tool_targets).await;

        let ResponseEvent::OutputItemDone(ResponseItem::FunctionCall {
            name,
            namespace,
            arguments,
            call_id,
            ..
        }) = &events[1]
        else {
            panic!("expected namespaced function call, got {:?}", events[1]);
        };
        assert_eq!(name, "create_map_card");
        assert_eq!(namespace.as_deref(), Some("mcp__map_cards"));
        assert_eq!(arguments, "{}");
        assert_eq!(call_id, "call_1");
    }

    #[tokio::test]
    async fn chat_sse_marks_text_only_completion_as_final_answer() {
        let body = concat!(
            "data: {\"id\":\"chatcmpl-1\",\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\n",
            "data: [DONE]\n\n",
        );

        let events = collect_events(body).await;

        let ResponseEvent::OutputItemDone(ResponseItem::Message { content, phase, .. }) =
            &events[2]
        else {
            panic!("expected assistant message, got {:?}", events[2]);
        };
        assert_eq!(
            (content, phase),
            (
                &vec![ContentItem::OutputText {
                    text: "done".to_string(),
                }],
                &Some(MessagePhase::FinalAnswer),
            )
        );
    }
}
