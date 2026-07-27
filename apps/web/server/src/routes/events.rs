use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use open_web_codex_platform_store::AppState;
use serde::Deserialize;
use serde_json::json;

use crate::middleware::auth::authenticate_token;

#[derive(Deserialize)]
struct AuthenticateMessage {
    #[serde(rename = "type")]
    kind: String,
    token: String,
}

/// Authenticated, tenant-filtered live event channel. Authentication is the
/// first WebSocket message so credentials never appear in URLs or access logs.
pub async fn websocket(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| serve(socket, state))
}

async fn serve(mut socket: WebSocket, state: AppState) {
    let authenticated = tokio::time::timeout(Duration::from_secs(5), socket.next()).await;
    let request = match authenticated {
        Ok(Some(Ok(Message::Text(text)))) => {
            serde_json::from_str::<AuthenticateMessage>(&text).ok()
        }
        _ => None,
    };
    let Some(request) = request.filter(|request| request.kind == "authenticate") else {
        reject(&mut socket, "authentication_required").await;
        return;
    };
    let auth = match authenticate_token(&state.db, &request.token).await {
        Ok(auth) => auth,
        Err(_) => {
            reject(&mut socket, "authentication_failed").await;
            return;
        }
    };
    // Subscribe before acknowledging readiness so events emitted during the
    // handshake remain queued for this connection.
    let mut events = state.event_bus.subscribe();
    let ready = json!({ "type": "ready", "version": 1 }).to_string();
    if socket.send(Message::Text(ready.into())).await.is_err() {
        return;
    }

    loop {
        tokio::select! {
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                _ => {}
            },
            event = events.recv() => match event {
                Ok(event) if event.organization_id == auth.organization_id => {
                    let Ok(text) = String::from_utf8(event.payload) else {
                        continue;
                    };
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        return;
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let resync = json!({ "type": "resyncRequired", "version": 1 }).to_string();
                    if socket.send(Message::Text(resync.into())).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

async fn reject(socket: &mut WebSocket, code: &str) {
    let payload = json!({ "type": "error", "version": 1, "code": code }).to_string();
    let _ = socket.send(Message::Text(payload.into())).await;
    let _ = socket.close().await;
}
