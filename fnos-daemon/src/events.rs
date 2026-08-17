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
/// 否则会冲掉日志帧）。同时持久化到日志文件（2026-08-17 用户反馈：daemon 更新
/// 重启后内存缓冲清空导致日志全丢——文件兜底，重启后从文件回灌）。
pub fn push_log_history(history: &LogHistory, logfile: Option<&crate::logfile::LogFile>, event: &Event) {
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
        h.push_back(json.clone());
    }
    if let Some(lf) = logfile {
        let _ = lf.append(&json);
    }
}

/// 日志消息（与桌面版 models::LogMessage 载荷一致）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LogMessage {
    pub tunnel_id: i32,
    pub message: String,
    pub timestamp: String,
}

/// 下载进度（与桌面版 DownloadProgress 一致；fnOS 扩展 stage 字段，
/// 前端据此展示「连接中/下载中/校验中/应用完成」阶段）。
#[derive(Serialize, Clone, Debug)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: u64,
    pub percentage: f64,
    /// 阶段：connecting / downloading / verifying / applying
    pub stage: String,
}

/// 守护自动重启事件载荷。
#[derive(Serialize, Clone, Debug)]
pub struct AutoRestartedPayload {
    pub tunnel_id: i32,
    pub timestamp: String,
}

/// 异步下载结果（2026-08-18：网关转发超时实测 504，下载改后台任务，
/// 完成/失败经本事件推送；shim 收到 ok 后再触发 apply）。
#[derive(Serialize, Clone, Debug)]
pub struct DownloadResult {
    pub ok: bool,
    pub staged: Option<String>,
    pub error: Option<String>,
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

    pub fn download_result(r: DownloadResult) -> Self {
        Self {
            event_type: "download-result",
            payload: serde_json::to_value(r).unwrap_or_default(),
        }
    }

    pub fn auto_restarted(p: AutoRestartedPayload) -> Self {
        Self {
            event_type: "tunnel-auto-restarted",
            payload: serde_json::to_value(p).unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn new_history() -> LogHistory {
        Arc::new(Mutex::new(VecDeque::new()))
    }

    fn log_event(i: usize) -> Event {
        Event::log(LogMessage {
            tunnel_id: 1,
            message: format!("m{i}"),
            timestamp: "t".to_string(),
        })
    }

    #[test]
    fn 环形缓冲满时逐出最旧() {
        let history = new_history();
        for i in 0..(LOG_HISTORY_CAP + 10) {
            push_log_history(&history, None, &log_event(i));
        }
        let h = history.lock().unwrap();
        assert_eq!(h.len(), LOG_HISTORY_CAP, "缓冲不应超过容量");
        let oldest: serde_json::Value = serde_json::from_str(h.front().unwrap()).unwrap();
        assert_eq!(oldest["payload"]["message"], "m10", "最旧 10 条应被逐出");
        let newest: serde_json::Value = serde_json::from_str(h.back().unwrap()).unwrap();
        assert_eq!(newest["payload"]["message"], "m521", "最新帧应保留");
    }

    #[test]
    fn 非frpc日志不入缓冲() {
        let history = new_history();
        push_log_history(
            &history,
            None,
            &Event::download_progress(DownloadProgress {
                downloaded: 5,
                total: 100,
                percentage: 5.0,
                stage: "downloading".to_string(),
            }),
        );
        push_log_history(
            &history,
            None,
            &Event::auto_restarted(AutoRestartedPayload {
                tunnel_id: 1,
                timestamp: "t".to_string(),
            }),
        );
        assert!(
            history.lock().unwrap().is_empty(),
            "高频/守护事件不应冲掉日志缓冲"
        );
    }

    #[test]
    fn 帧序列化含type字段_ws直接透发() {
        // ws.rs 直接透发缓冲里的 JSON 字符串，帧必须自包含 type 字段
        let event = log_event(0);
        let json = serde_json::to_string(&event).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["type"], "frpc-log");
        assert_eq!(v["payload"]["tunnel_id"], 1);
    }
}