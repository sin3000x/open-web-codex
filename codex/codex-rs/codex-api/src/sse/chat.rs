use crate::chat_translate::ChatToolTarget;
use crate::common::ResponseEvent;
use crate::common::ResponseStream;
use crate::error::ApiError;
use crate::telemetry::SseTelemetry;
use codex_client::ByteStream;
use codex_client::StreamResponse;
use codex_protocol::models::ResponseItem;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio::time::timeout;
use tracing::debug;

const REQUEST_ID_HEADER: &str = "x-request-id";
const OPENAI_MODEL_HEADER: &str = "openai-model";

#[path = "chat_state.rs"]
mod chat_state;
#[path = "chat_wire.rs"]
mod chat_wire;

use chat_state::ChatStreamState;
use chat_state::assistant_message_item;
use chat_wire::ChatCompletionChunk;
use chat_wire::ChatDelta;
use chat_wire::ChatFinishReason;

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
    let (tx_event, rx_event) = mpsc::channel(1600);
    tokio::spawn(async move {
        if let Some(model) = server_model
            && tx_event
                .send(Ok(ResponseEvent::ServerModel(model)))
                .await
                .is_err()
        {
            return;
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
    let mut state = ChatStreamState::new();
    loop {
        let started = Instant::now();
        let next_event = timeout(idle_timeout, stream.next()).await;
        if let Some(telemetry) = &telemetry {
            telemetry.on_sse_poll(&next_event, started.elapsed());
        }
        let event = match next_event {
            Ok(Some(Ok(event))) => event,
            Ok(Some(Err(error))) => {
                send_error(
                    &tx_event,
                    ApiError::Stream(format!("chat SSE error: {error}")),
                )
                .await;
                return;
            }
            Ok(None) => {
                send_error(
                    &tx_event,
                    ApiError::Stream("chat stream closed before [DONE]".to_string()),
                )
                .await;
                return;
            }
            Err(_) => {
                send_error(
                    &tx_event,
                    ApiError::Stream("idle timeout waiting for chat SSE".to_string()),
                )
                .await;
                return;
            }
        };

        if event.data.trim() == "[DONE]" {
            if let Err(error) = finish_chat_stream(&tx_event, state, &tool_targets).await {
                send_error(&tx_event, error).await;
            }
            return;
        }
        let chunk: ChatCompletionChunk = match serde_json::from_str(&event.data) {
            Ok(chunk) => chunk,
            Err(error) => {
                debug!("invalid chat SSE payload: {error}");
                send_error(
                    &tx_event,
                    ApiError::Stream(format!("invalid chat completion SSE payload: {error}")),
                )
                .await;
                return;
            }
        };
        if let Some(model) = chunk.model
            && tx_event
                .send(Ok(ResponseEvent::ServerModel(model)))
                .await
                .is_err()
        {
            return;
        }
        if let Some(usage) = chunk.usage {
            state.token_usage = Some(usage.into());
        }
        if chunk.choices.len() > 1 {
            send_error(
                &tx_event,
                ApiError::Stream(
                    "chat completion returned multiple choices; only choice index 0 is supported"
                        .to_string(),
                ),
            )
            .await;
            return;
        }
        if let Some(choice) = chunk.choices.into_iter().next() {
            if choice.index != 0 {
                send_error(
                    &tx_event,
                    ApiError::Stream(format!(
                        "chat completion returned unsupported choice index {}",
                        choice.index
                    )),
                )
                .await;
                return;
            }
            if let Some(finish_reason) = choice.finish_reason
                && let Err(error) = state.record_finish_reason(&finish_reason)
            {
                send_error(&tx_event, error).await;
                return;
            }
            let ChatDelta {
                content,
                refusal,
                reasoning_content,
                tool_calls,
            } = choice.delta;
            if refusal
                .as_deref()
                .is_some_and(|refusal| !refusal.is_empty())
            {
                send_error(
                    &tx_event,
                    ApiError::Stream("chat completion returned a refusal".to_string()),
                )
                .await;
                return;
            }
            if let Some(reasoning_content) = reasoning_content
                && !reasoning_content.is_empty()
                && let Err(error) = state
                    .append_reasoning_content(&tx_event, reasoning_content)
                    .await
            {
                send_error(&tx_event, error).await;
                return;
            }
            if let Some(content) = content
                && !content.is_empty()
            {
                if let Err(error) = state.finish_reasoning_item(&tx_event).await {
                    send_error(&tx_event, error).await;
                    return;
                }
                if !state.assistant_item_started {
                    if tx_event
                        .send(Ok(ResponseEvent::OutputItemAdded(
                            state.assistant_message_item(String::new()),
                        )))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    state.assistant_item_started = true;
                }
                state.output_started = true;
                state.assistant_text.push_str(&content);
                if tx_event
                    .send(Ok(ResponseEvent::OutputTextDelta(content)))
                    .await
                    .is_err()
                {
                    return;
                }
            }
            if !tool_calls.is_empty() {
                if let Err(error) = state.finish_reasoning_item(&tx_event).await {
                    send_error(&tx_event, error).await;
                    return;
                }
                state.output_started = true;
            }
            for tool_call in tool_calls {
                if let Err(error) = state.merge_tool_call(tool_call) {
                    send_error(&tx_event, error).await;
                    return;
                }
            }
        }
    }
}

async fn send_error(tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>, error: ApiError) {
    let _ = tx_event.send(Err(error)).await;
}

async fn finish_chat_stream(
    tx_event: &mpsc::Sender<Result<ResponseEvent, ApiError>>,
    mut state: ChatStreamState,
    tool_targets: &HashMap<String, ChatToolTarget>,
) -> Result<(), ApiError> {
    state.finish_reasoning_item(tx_event).await?;
    let ChatStreamState {
        response_id,
        assistant_item_id,
        assistant_text,
        tool_calls,
        token_usage,
        finish_reason,
        ..
    } = state;
    match finish_reason {
        None => {
            return Err(ApiError::Stream(
                "chat stream ended with [DONE] before a supported finish_reason".to_string(),
            ));
        }
        Some(ChatFinishReason::Stop) if !tool_calls.is_empty() => {
            return Err(ApiError::Stream(
                "chat completion ended with finish_reason `stop` after tool calls".to_string(),
            ));
        }
        Some(ChatFinishReason::Stop) if assistant_text.is_empty() => {
            return Err(ApiError::Stream(
                "chat completion ended with finish_reason `stop` without assistant text"
                    .to_string(),
            ));
        }
        Some(ChatFinishReason::ToolCalls) if tool_calls.is_empty() => {
            return Err(ApiError::Stream(
                "chat completion ended with finish_reason `tool_calls` without tool calls"
                    .to_string(),
            ));
        }
        Some(ChatFinishReason::Stop | ChatFinishReason::ToolCalls) => {}
    };
    let tool_items = tool_calls
        .into_iter()
        .map(|tool_call| {
            if tool_call.id.is_empty() || tool_call.function.name.is_empty() {
                return Err(ApiError::InvalidRequest {
                    message: "provider protocol violation: chat completion returned an incomplete tool call"
                        .to_string(),
                });
            }
            if tool_call.function.arguments.is_empty() {
                return Err(ApiError::InvalidRequest {
                    message: "provider protocol violation: chat completion returned a tool call without arguments"
                        .to_string(),
                });
            }
            // Unparsable function arguments are not a transport failure: Core's
            // tool handlers already return the parse error to the model as the
            // call's output, exactly as for a Responses-native FunctionCall.
            let target = tool_targets.get(&tool_call.function.name).ok_or_else(|| {
                ApiError::InvalidRequest {
                    message: format!(
                        "provider protocol violation: chat completion called unavailable tool `{}`; the tool was not offered in this request",
                        tool_call.function.name
                    ),
                }
            })?;
            if target.name == "tool_search" && target.namespace.is_none() {
                Ok(ResponseItem::ToolSearchCall {
                    id: Some(codex_protocol::ResponseItemId::from_server(
                        tool_call.id.clone(),
                    )),
                    call_id: Some(tool_call.id),
                    status: None,
                    execution: "client".to_string(),
                    arguments: serde_json::from_str(&tool_call.function.arguments).map_err(
                        |error| ApiError::InvalidRequest {
                            message: format!(
                                "provider protocol violation: chat completion returned invalid tool_search arguments: {error}"
                            ),
                        },
                    )?,
                    internal_chat_message_metadata_passthrough: None,
                })
            } else {
                Ok(ResponseItem::FunctionCall {
                    id: Some(codex_protocol::ResponseItemId::from_server(
                        tool_call.id.clone(),
                    )),
                    name: target.name.clone(),
                    namespace: target.namespace.clone(),
                    arguments: tool_call.function.arguments,
                    encrypted_function_args: collaboration_plaintext_marker(target),
                    call_id: tool_call.id,
                    internal_chat_message_metadata_passthrough: None,
                })
            }
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    // Core's native reducer has one active item. A streamed text Message must
    // therefore be completed before buffered FunctionCall items begin.
    if !assistant_text.is_empty() {
        let item = assistant_message_item(assistant_item_id, assistant_text);
        if tx_event
            .send(Ok(ResponseEvent::OutputItemDone(item)))
            .await
            .is_err()
        {
            return Ok(());
        }
    }
    for item in tool_items {
        if tx_event
            .send(Ok(ResponseEvent::OutputItemAdded(item.clone())))
            .await
            .is_err()
        {
            return Ok(());
        }
        if tx_event
            .send(Ok(ResponseEvent::OutputItemDone(item)))
            .await
            .is_err()
        {
            return Ok(());
        }
    }
    let _ = tx_event
        .send(Ok(ResponseEvent::Completed {
            response_id,
            token_usage,
            end_turn: None,
        }))
        .await;
    Ok(())
}

fn collaboration_plaintext_marker(target: &ChatToolTarget) -> Option<Vec<String>> {
    (target.namespace.as_deref() == Some("collaboration")
        && matches!(
            target.name.as_str(),
            "spawn_agent" | "send_message" | "followup_task"
        ))
    .then(Vec::new)
}

#[cfg(test)]
#[path = "chat_smoke_tests.rs"]
mod smoke_tests;

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
