//! /ws/logs：事件订阅推送。
//!
//! 订阅 broadcast 通道，将 frpc-log / download-progress / tunnel-auto-restarted
//! 事件帧转发给浏览器（shim 的 listen 桥接目标）。事件名与载荷与桌面版一致。
//!
//! 连接建立后先补发日志历史（frpc.rs 环形缓冲）：网关空闲超时会周期断开 WS
//! （fnOS 实测约 60s），断线窗口内产生的事件 broadcast 不重放，靠补发兜底。
//! 补发帧带 "replay": true 标记，前端据此**不触发通知**（toast/音效）——
//! 页面重载后补发历史日志会重新显示（特性），但旧事件不应重复打扰用户。

use crate::events::{Event, LogHistory};
use crate::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use serde_json::json;
use tokio::sync::broadcast;

pub async fn ws_logs(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| {
        handle_socket(socket, state.events.subscribe(), state.log_history.clone())
    })
}

async fn handle_socket(
    mut socket: WebSocket,
    mut rx: broadcast::Receiver<Event>,
    log_history: LogHistory,
) {
    // 先按序补发历史帧（收到即视为新订阅者，历史仅此一次）
    let snapshot = log_history
        .lock()
        .map(|h| h.iter().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    for frame in snapshot {
        // 补发帧加 replay 标记（实时帧无此字段）
        let marked = serde_json::from_str::<serde_json::Value>(&frame)
            .map(|mut v| {
                v["replay"] = json!(true);
                serde_json::to_string(&v).unwrap_or_else(|_| frame.clone())
            })
            .unwrap_or(frame);
        if socket.send(Message::Text(marked.into())).await.is_err() {
            return;
        }
    }

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