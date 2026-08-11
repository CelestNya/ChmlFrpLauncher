//! /ws/logs：事件订阅推送。
//!
//! 订阅 broadcast 通道，将 frpc-log / download-progress / tunnel-auto-restarted
//! 事件帧转发给浏览器（shim 的 listen 桥接目标）。事件名与载荷与桌面版一致。

use crate::events::Event;
use crate::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use tokio::sync::broadcast;

pub async fn ws_logs(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state.events.subscribe()))
}

async fn handle_socket(mut socket: WebSocket, mut rx: broadcast::Receiver<Event>) {
    loop {
        tokio::select! {
            // 客户端断开 / 关闭帧
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
            // 事件推送；发送失败视为连接已断开
            Ok(event) = rx.recv() => {
                let Ok(text) = serde_json::to_string(&event) else { continue };
                if socket.send(Message::Text(text.into())).await.is_err() {
                    break;
                }
            }
        }
    }
}