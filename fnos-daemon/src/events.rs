//! 事件总线：frpc 日志 / 下载进度 / 守护重启事件。
//!
//! 桌面版通过 tauri `emit` 推事件；daemon 用 broadcast channel，
//! C3 由 /ws/logs 订阅转发（事件名与载荷与桌面版一致，shim 透传无感）。

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// frpc 日志环形缓冲：WS 断线（网关空闲超时）重连后补发，避免断线窗口日志丢失。
/// 元素为已序列化的 Event JSON 字符串（ws.rs 直接透发，零重复序列化）。
pub type LogHistory = Arc<Mutex<VecDeque<String>>>;
/// 缓冲容量：断线窗口最长约 60s，日志密集时保留最近的即可。
pub const LOG_HISTORY_CAP: usize = 512;

/// 将事件帧写入历史缓冲（只缓存 frpc-log；download-progress 等高频事件不缓存，
/// 否则会冲掉日志帧）。
pub fn push_log_history(history: &LogHistory, event: &Event) {
    if event.event_type != "frpc-log" {
        return;
    }
    let Ok(json) = serde_json::to_string(event) else {
        return;
    };
    if let Ok(mut h) = history.lock() {
        if h.len() >= LOG_HISTORY_CAP {
            h.pop_front();
        }
        h.push_back(json);
    }
}

/// 日志消息（与桌面版 models::LogMessage 载荷一致）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LogMessage {
    pub tunnel_id: i32,
    pub message: String,
    pub timestamp: String,
}

/// 下载进度（与桌面版 DownloadProgress 一致）。
#[derive(Serialize, Clone, Debug)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub percentage: f64,
}

/// 守护自动重启事件载荷。
#[derive(Serialize, Clone, Debug)]
pub struct AutoRestartedPayload {
    pub tunnel_id: i32,
    pub timestamp: String,
}

/// 统一事件帧（与桌面版事件名对应）。
#[derive(Serialize, Clone, Debug)]
pub struct Event {
    #[serde(rename = "type")]
    pub event_type: &'static str,
    pub payload: serde_json::Value,
}

impl Event {
    pub fn log(msg: LogMessage) -> Self {
        Self {
            event_type: "frpc-log",
            payload: serde_json::to_value(msg).unwrap_or_default(),
        }
    }

    pub fn download_progress(p: DownloadProgress) -> Self {
        Self {
            event_type: "download-progress",
            payload: serde_json::to_value(p).unwrap_or_default(),
        }
    }

    pub fn auto_restarted(p: AutoRestartedPayload) -> Self {
        Self {
            event_type: "tunnel-auto-restarted",
            payload: serde_json::to_value(p).unwrap_or_default(),
        }
    }
}