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
use crate::guard;
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
                .get_all_records()
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

        // ---- 进程守护（C3） ----
        "set_process_guard_enabled" => {
            #[derive(Deserialize)]
            struct GuardArgs {
                enabled: bool,
            }
            let args: GuardArgs = parse_args(args)?;
            let data = guard::set_process_guard_enabled(&state.guard, args.enabled);
            Ok(json!(data))
        }
        "get_process_guard_enabled" => {
            let data = guard::get_process_guard_enabled(&state.guard);
            Ok(json!(data))
        }
        "add_guarded_process" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct AddGuardedArgs {
                tunnel_id: i32,
                config: TunnelConfig,
            }
            let args: AddGuardedArgs = parse_args(args)?;
            guard::add_guarded_process(&state.guard, args.tunnel_id, args.config);
            Ok(json!(null))
        }
        "add_guarded_custom_tunnel" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct AddCustomGuardedArgs {
                tunnel_id: i32,
                original_id: String,
            }
            let args: AddCustomGuardedArgs = parse_args(args)?;
            guard::add_guarded_custom_tunnel(&state.guard, args.tunnel_id, args.original_id);
            Ok(json!(null))
        }
        "remove_guarded_process" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct RemoveGuardedArgs {
                tunnel_id: i32,
                #[serde(default)]
                is_manual_stop: bool,
            }
            let args: RemoveGuardedArgs = parse_args(args)?;
            guard::remove_guarded_process(&state.guard, args.tunnel_id, args.is_manual_stop);
            Ok(json!(null))
        }
        "check_log_and_stop_guard" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct CheckLogArgs {
                tunnel_id: i32,
                log_message: String,
            }
            let args: CheckLogArgs = parse_args(args)?;
            let data = check_log_with_emit(&state, args.tunnel_id, args.log_message).await?;
            Ok(json!(data))
        }

        // ---- NO_OP：fnOS 版显式不可用 ----
        "fix_frpc_ini_tls" => Err("该功能在 fnOS 版不可用: fix_frpc_ini_tls".to_string()),

        _ => Err(format!("未知或不可用的命令: {cmd}")),
    }
}

/// check_log_and_stop_guard 的 daemon 实现：命中模式则移除守护并广播日志。
async fn check_log_with_emit(
    state: &AppState,
    tunnel_id: i32,
    log_message: String,
) -> Result<String, String> {
    use crate::events::{Event, LogMessage};

    let Some(pattern) = guard::should_stop_guard_by_log(&log_message) else {
        return Ok("无需停止守护".to_string());
    };

    guard::remove_guarded_process(&state.guard, tunnel_id, false);

    let timestamp = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
    let _ = state.events.send(Event::log(LogMessage {
        tunnel_id,
        message: format!(
            "[W] [ChmlFrpLauncher] 检测到错误 \"{}\"，已停止守护进程",
            pattern
        ),
        timestamp,
    }));

    Ok("已停止守护进程".to_string())
}