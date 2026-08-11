//! 事件总线：frpc 日志 / 下载进度 / 守护重启事件。
//!
//! 桌面版通过 tauri `emit` 推事件；daemon 用 broadcast channel，
//! C3 由 /ws/logs 订阅转发（事件名与载荷与桌面版一致，shim 透传无感）。

use serde::{Deserialize, Serialize};

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