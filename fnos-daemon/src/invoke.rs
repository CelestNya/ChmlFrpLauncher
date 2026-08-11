//! /api/invoke 透传分发（Q3 决议：通用透传契约）。
//!
//! 契约（与桌面版 Tauri IPC 同语义）：
//! ```json
//! POST /api/invoke
//! { "cmd": "start_frpc", "args": { "tunnelId": 3, "config": { ... } } }
//! → 200 { "ok": true, "data": ... }
//! → 200 { "ok": false, "error": "..." }
//! ```
//! 参数结构体统一 `#[serde(rename_all = "camelCase")]`；
//! 事件（frpc-log 等）经 /ws/logs 推送，见 ws.rs。

use crate::AppState;
use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Deserialize)]
pub struct InvokeRequest {
    pub cmd: String,
    #[serde(default)]
    pub args: Option<Value>,
}

#[derive(Serialize)]
pub struct InvokeResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl InvokeResponse {
    fn ok(data: impl Serialize) -> Self {
        Self {
            ok: true,
            data: serde_json::to_value(data).ok(),
            error: None,
        }
    }

    fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            data: None,
            error: Some(error.into()),
        }
    }
}

/// GET /api/bootstrap —— shim 获取版本信息。
pub async fn bootstrap() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "chmlfrp-daemon",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// POST /api/invoke —— 命令透传分发。
pub async fn handle_invoke(
    State(state): State<AppState>,
    Json(req): Json<InvokeRequest>,
) -> Json<InvokeResponse> {
    match dispatch(&state, &req.cmd, req.args).await {
        Ok(data) => Json(InvokeResponse::ok(data)),
        Err(e) => Json(InvokeResponse::err(e)),
    }
}

/// 命令分发表：C2 起逐步登记 frpc 管理、守护、下载、网络探测等实现。
/// 未登记命令返回明确错误（前端不应触达；NO_OP 命令在此登记为显式不可用）。
async fn dispatch(
    _state: &AppState,
    cmd: &str,
    _args: Option<Value>,
) -> Result<Value, String> {
    Err(format!("未知或不可用的命令: {cmd}"))
}
