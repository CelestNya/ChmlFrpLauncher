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

use crate::download;
use crate::frpc::TunnelConfig;
use crate::guard;
use crate::net;
use crate::proxy::{self, HttpRequestOptions};
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
        // fnOS 内部命令：清空 WS 补发缓冲（前端"清空日志"联动，避免刷新后补发旧日志）
        "clear_log_history" => {
            if let Ok(mut history) = state.log_history.lock() {
                history.clear();
            }
            Ok(json!(true))
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
            // daemon 中-12：启用时把存量运行隧道重新登记守护
            let running = state.frpc.persistence.get_running_tunnels();
            let data = guard::set_process_guard_enabled_with_recovery(
                &state.guard,
                args.enabled,
                &running,
            );
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

        // ---- 自定义隧道（C4） ----
        "save_custom_tunnel" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct SaveCustomArgs {
                tunnel_name: String,
                config_content: String,
            }
            let args: SaveCustomArgs = parse_args(args)?;
            let data = state.custom.save_custom_tunnel(args.tunnel_name, args.config_content)?;
            Ok(json!(data))
        }
        "get_custom_tunnels" => {
            let data = state.custom.get_custom_tunnels()?;
            Ok(json!(data))
        }
        "get_custom_tunnel_config" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: String,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            let data = state.custom.get_custom_tunnel_config(args.tunnel_id)?;
            Ok(json!(data))
        }
        "update_custom_tunnel" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct UpdateCustomArgs {
                tunnel_id: String,
                config_content: String,
            }
            let args: UpdateCustomArgs = parse_args(args)?;
            let data = state
                .custom
                .update_custom_tunnel(args.tunnel_id, args.config_content)?;
            Ok(json!(data))
        }
        "delete_custom_tunnel" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: String,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            state.custom.delete_custom_tunnel(args.tunnel_id)?;
            Ok(json!(null))
        }
        "start_custom_tunnel" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: String,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            let data = state.custom.start_custom_tunnel(args.tunnel_id).await?;
            Ok(json!(data))
        }
        "stop_custom_tunnel" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: String,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            let data = state.custom.stop_custom_tunnel(args.tunnel_id).await?;
            Ok(json!(data))
        }
        "is_custom_tunnel_running" => {
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct TunnelIdArgs {
                tunnel_id: String,
            }
            let args: TunnelIdArgs = parse_args(args)?;
            let data = state.custom.is_custom_tunnel_running(args.tunnel_id)?;
            Ok(json!(data))
        }

        // ---- frpc 下载（C4） ----
        "download_frpc" => {
            let data = download::download_frpc(&state.frpc).await?;
            Ok(json!(data))
        }
        "check_frpc_exists" => {
            let data = download::check_frpc_exists(&state.frpc);
            Ok(json!(data))
        }
        "get_frpc_directory" => {
            let data = download::get_frpc_directory(&state.frpc);
            Ok(json!(data))
        }
        "get_download_url" => {
            let data = download::get_download_url().await?;
            Ok(json!(data))
        }

        // ---- 网络探测与 HTTP 代理（C5） ----
        "ping_host" => {
            #[derive(Deserialize)]
            struct PingArgs {
                host: String,
            }
            let args: PingArgs = parse_args(args)?;
            let data = net::ping_host(args.host).await?;
            Ok(json!(data))
        }
        "get_ports" => {
            let data = net::get_ports().await;
            Ok(json!(data))
        }
        "check_local_port" => {
            #[derive(Deserialize)]
            struct PortArgs {
                port: String,
            }
            let args: PortArgs = parse_args(args)?;
            let data = net::check_local_port(args.port).await;
            Ok(json!(data))
        }
        "resolve_domain_to_ip" => {
            #[derive(Deserialize)]
            struct DomainArgs {
                domain: String,
            }
            let args: DomainArgs = parse_args(args)?;
            let data = net::resolve_domain_to_ip(args.domain).await?;
            Ok(json!(data))
        }
        "http_request" => {
            #[derive(Deserialize)]
            struct ProxyArgs {
                options: HttpRequestOptions,
            }
            let args: ProxyArgs = parse_args(args)?;
            let data = proxy::http_request(args.options).await?;
            Ok(json!(data))
        }
        "http_request_raw" => {
            #[derive(Deserialize)]
            struct ProxyArgs {
                options: HttpRequestOptions,
            }
            let args: ProxyArgs = parse_args(args)?;
            let data = proxy::http_request_raw(args.options).await?;
            Ok(json!(data))
        }

        // ---- NO_OP：fnOS 版显式不可用（桌面专属能力，patch 删 UI 后前端不调用） ----
        "fix_frpc_ini_tls"
        | "is_autostart_enabled"
        | "set_autostart"
        | "get_tunnel_auto_start"
        | "set_tunnel_auto_start"
        | "hide_window"
        | "show_window"
        | "quit_app"
        | "read_image_folder"
        | "copy_background_video"
        | "copy_background_image"
        | "import_background_image_folder"
        | "get_background_video_path" => {
            Err(format!("该功能在 fnOS 版不可用: {cmd}"))
        }

        _ => Err(format!("未知或不可用的命令: {cmd}")),
    }
}

/// check_log_and_stop_guard 的 daemon 实现：命中模式则移除守护并广播日志。
async fn check_log_with_emit(
    state: &AppState,
    tunnel_id: i32,
    log_message: String,
) -> Result<String, String> {
    use crate::events::LogMessage;

    let Some(pattern) = guard::should_stop_guard_by_log(&log_message) else {
        return Ok("无需停止守护".to_string());
    };

    guard::remove_guarded_process(&state.guard, tunnel_id, false);

    let timestamp = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
    // daemon 中-12：走 emit_log（进补发缓冲）而非裸 events.send——WS 断线窗口内
    // 该消息会丢失且重连后不补发
    state.frpc.emit_log(LogMessage {
        tunnel_id,
        message: format!(
            "[W] [ChmlFrpLauncher] 检测到错误 \"{}\"，已停止守护进程",
            pattern
        ),
        timestamp,
    });

    Ok("已停止守护进程".to_string())
}