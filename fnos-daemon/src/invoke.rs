//! /api/invoke 透传分发（Q3 决议：通用透传契约）。
//!
//! 契约（与桌面版 Tauri IPC 同语义）：
//! ```json
//! POST /api/invoke
//! { "cmd": "start_frpc", "args": { "config": { ... } } }
//! → 200 { "ok": true, "data": ... }
//! → 200 { "ok": false, "error": "..." }
//! ```
//! 参数结构体统一 `#[serde(rename_all = "camelCase")]`（与 Tauri IPC 参数名一致）；
//! 事件（frpc-log 等）经 /ws/logs 推送（C3 接入）。

use crate::frpc::TunnelConfig;
use crate::AppState;
use axum::extract::State;
use axum::Json;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
pub struct InvokeRequest {
    pub cmd: String,
    #[serde(default)]
    pub args: Option<Value>,
}

#[derive(serde::Serialize)]
pub struct InvokeResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl InvokeResponse {
    fn ok(data: impl serde::Serialize) -> Self {
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
pub async fn bootstrap() -> Json<Value> {
    Json(json!({
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

fn parse_args<T: DeserializeOwned>(args: Option<Value>) -> Result<T, String> {
    let value = args.unwrap_or(Value::Null);
    serde_json::from_value(value).map_err(|e| format!("参数解析失败: {}", e))
}

/// 命令分发表：C2 登记进程管理；后续批次陆续登记守护/下载/网络/自定义隧道。
async fn dispatch(state: &AppState, cmd: &str, args: Option<Value>) -> Result<Value, String> {
    match cmd {
        // ---- frpc 进程管理（C2） ----
        "start_frpc" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct StartFrpcArgs {
                config: TunnelConfig,
            }
            let args: StartFrpcArgs = parse_args(args)?;
            let data = state.frpc.start_frpc(args.config).await?;
            Ok(json!(data))
        }
        "stop_frpc" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: i32,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            let data = state.frpc.stop_frpc(args.tunnel_id).await?;
            Ok(json!(data))
        }
        "is_frpc_running" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: i32,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            let data = state.frpc.is_frpc_running(args.tunnel_id)?;
            Ok(json!(data))
        }
        "get_running_tunnels" => {
            let data = state.frpc.get_running_tunnels()?;
            Ok(json!(data))
        }
        "get_persisted_running_tunnels" => {
            let data = state.frpc.persistence.get_running_tunnels();
            Ok(json!(data))
        }
        "stop_orphan_process" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: i32,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            let info = state
                .frpc
                .persistence
                .get_running_tunnels()
                .into_iter()
                .find(|t| t.tunnel_id == args.tunnel_id);
            match info {
                Some(info) => {
                    let data = state
                        .frpc
                        .persistence
                        .kill_orphan(args.tunnel_id, info.pid)?;
                    Ok(json!(data))
                }
                None => Err("未找到该隧道的运行记录".to_string()),
            }
        }
        "is_tunnel_process_alive" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: i32,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            let data = state.frpc.is_frpc_running(args.tunnel_id)?;
            Ok(json!(data))
        }

        // ---- NO_OP：fnOS 版显式不可用 ----
        "fix_frpc_ini_tls" => Err("该功能在 fnOS 版不可用: fix_frpc_ini_tls".to_string()),

        _ => Err(format!("未知或不可用的命令: {cmd}")),
    }
}