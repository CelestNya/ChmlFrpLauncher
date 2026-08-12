//! 进程守护：3s 轮询 + 错误日志模式智能停止 + 自动重启。
//!
//! 规格源：src-tauri/src/commands/process_guard.rs。
//! 差异：桌面版用 std::thread + block_on；daemon 全 tokio（select 合并
//! 3s tick 与事件监听两个职责）；**D1：守护默认开启**（桌面版默认关）。

use crate::custom::CustomManager;
use crate::events::{AutoRestartedPayload, Event, LogMessage};
use crate::frpc::{FrpcManager, TunnelConfig};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// 命中即停止守护的错误模式（照抄桌面版 STOP_GUARD_PATTERNS）。
const STOP_GUARD_PATTERNS: &[&str] = &[
    "token in login doesn't match token from configuration",
    "authorization failed",
    "invalid token",
    "read: connection reset by peer",
    "错误的用户token，此用户不存在",
    "允许的隧道数量超出上限，请删除隧道或续费vip",
    "不属于你",
    "缺少用户token或隧道id参数",
    "您目前为免费会员",
    "客户端代理参数错误，配置文件与记录不匹配。请不要随意修改配置文件！",
    "ChmlFrp API Error",
];

#[derive(Clone, Debug)]
pub enum GuardTunnelType {
    Api { config: TunnelConfig },
    Custom { original_id: String },
}

#[derive(Clone, Debug)]
pub struct ProcessGuardInfo {
    pub tunnel_id: i32,
    pub tunnel_type: GuardTunnelType,
}

/// 守护状态（照桌面版 ProcessGuardState；enabled 默认 true = D1 决议）。
pub struct GuardState {
    pub enabled: AtomicBool,
    pub guarded_processes: Mutex<HashMap<i32, ProcessGuardInfo>>,
    pub manually_stopped: Mutex<HashSet<i32>>,
}

impl GuardState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(true),
            guarded_processes: Mutex::new(HashMap::new()),
            manually_stopped: Mutex::new(HashSet::new()),
        })
    }
}

pub fn should_stop_guard_by_log(message: &str) -> Option<&'static str> {
    let message_lower = message.to_lowercase();
    STOP_GUARD_PATTERNS
        .iter()
        .find(|p| message_lower.contains(&p.to_lowercase()))
        .copied()
}

fn get_timestamp() -> String {
    chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string()
}

/// 守护监控：合并两个职责的 tokio task。
/// 1. 每 3s 轮询受守护隧道，离线则自动重启；
/// 2. 订阅日志事件，命中 STOP_GUARD_PATTERNS 则移除守护（防无限重启）。
pub fn start_guard_monitor(
    guard: Arc<GuardState>,
    frpc: Arc<FrpcManager>,
    custom: Arc<CustomManager>,
    events: broadcast::Sender<Event>,
) {
    tokio::spawn(async move {
        let mut rx = events.subscribe();

        loop {
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {
                    tick(&guard, &frpc, &custom, &events).await;
                }
                Ok(event) = rx.recv() => {
                    if let Event { event_type: "frpc-log", payload } = event {
                        if let Ok(log) = serde_json::from_value::<LogMessage>(payload) {
                            check_log_and_stop_guard(&guard, &frpc, log).await;
                        }
                    }
                }
            }
        }
    });
}

async fn tick(
    guard: &Arc<GuardState>,
    frpc: &Arc<FrpcManager>,
    custom: &Arc<CustomManager>,
    events: &broadcast::Sender<Event>,
) {
    if !guard.enabled.load(Ordering::SeqCst) {
        return;
    }

    let guarded_list: Vec<ProcessGuardInfo> = match guard.guarded_processes.lock() {
        Ok(guarded) => guarded.values().cloned().collect(),
        Err(_) => return,
    };

    if guarded_list.is_empty() {
        return;
    }

    for info in guarded_list {
        let tunnel_id = info.tunnel_id;

        if is_manually_stopped(guard, tunnel_id) {
            continue;
        }

        let running = match &info.tunnel_type {
            GuardTunnelType::Api { .. } => frpc.is_frpc_running(tunnel_id).unwrap_or(false),
            GuardTunnelType::Custom { original_id } => custom
                .is_custom_tunnel_running(original_id.clone())
                .unwrap_or(false),
        };
        if running {
            continue;
        }

        // 走 emit_log（缓冲+广播）：若用裸 events.send，断线窗口内（无 WS 订阅者）
        // 守护消息会丢失，且不进补发缓冲。与 frpc 日志行为保持一致。
        frpc.emit_log(LogMessage {
            tunnel_id,
            message: "[W] [ChmlFrpLauncher] 检测到进程离线，触发守护进程，自动重启中".to_string(),
            timestamp: get_timestamp(),
        });

        restart_tunnel(guard, frpc, custom, events, info).await;
    }
}

async fn restart_tunnel(
    guard: &Arc<GuardState>,
    frpc: &Arc<FrpcManager>,
    custom: &Arc<CustomManager>,
    events: &broadcast::Sender<Event>,
    info: ProcessGuardInfo,
) {
    let tunnel_id = info.tunnel_id;

    let result = match &info.tunnel_type {
        GuardTunnelType::Api { config } => {
            // 重启前短暂等待，避免 frpc 端口释放竞态（照桌面版 1s 延时）
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            frpc.start_frpc(config.clone()).await
        }
        GuardTunnelType::Custom { original_id } => {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            custom.start_custom_tunnel(original_id.clone()).await
        }
    };

    match result {
        Ok(_) => {
            let _ = events.send(Event::auto_restarted(AutoRestartedPayload {
                tunnel_id,
                timestamp: get_timestamp(),
            }));
        }
        Err(e) => {
            frpc.emit_log(LogMessage {
                tunnel_id,
                message: format!("[E] [ChmlFrpLauncher] 守护进程重启失败: {}", e),
                timestamp: get_timestamp(),
            });

            if let Ok(mut guarded) = guard.guarded_processes.lock() {
                guarded.remove(&tunnel_id);
            }
        }
    }
}

fn is_manually_stopped(guard: &Arc<GuardState>, tunnel_id: i32) -> bool {
    guard
        .manually_stopped
        .lock()
        .ok()
        .map(|s| s.contains(&tunnel_id))
        .unwrap_or(true)
}

/// 日志命中错误模式时停止守护（照桌面版 check_log_and_stop_guard）。
///
/// 注意：daemon 内部生成的日志（带 [ChmlFrpLauncher] 标记）不参与模式匹配——
/// 否则"已停止守护"消息自身会再次命中模式，造成 emit → 消费 → emit 的自触发循环
/// （桌面版日志管道只读子进程输出，天然无此问题；daemon 的事件消费者订阅整个广播通道）。
async fn check_log_and_stop_guard(
    guard: &Arc<GuardState>,
    frpc: &Arc<FrpcManager>,
    log: LogMessage,
) {
    if log.message.contains("[ChmlFrpLauncher]") {
        return;
    }

    let Some(pattern) = should_stop_guard_by_log(&log.message) else {
        return;
    };

    tracing::warn!("[守护进程] 检测到隧道 {} 出现错误: {}", log.tunnel_id, pattern);
    tracing::warn!("[守护进程] 停止对隧道 {} 的守护", log.tunnel_id);

    {
        let mut guarded = guard.guarded_processes.lock().ok();
        if let Some(ref mut g) = guarded {
            g.remove(&log.tunnel_id);
        }
    }

    frpc.emit_log(LogMessage {
        tunnel_id: log.tunnel_id,
        message: format!(
            "[W] [ChmlFrpLauncher] 检测到错误 \"{}\"，已停止守护进程",
            pattern
        ),
        timestamp: get_timestamp(),
    });
}

// ---- invoke 命令实现（薄胶水，操作 GuardState） ----

pub fn set_process_guard_enabled(guard: &GuardState, enabled: bool) -> String {
    guard.enabled.store(enabled, Ordering::SeqCst);
    if !enabled {
        if let Ok(mut guarded) = guard.guarded_processes.lock() {
            guarded.clear();
        }
        if let Ok(mut stopped) = guard.manually_stopped.lock() {
            stopped.clear();
        }
    }
    format!(
        "守护进程已{}",
        if enabled { "启用" } else { "禁用" }
    )
}

pub fn get_process_guard_enabled(guard: &GuardState) -> bool {
    guard.enabled.load(Ordering::SeqCst)
}

pub fn add_guarded_process(guard: &GuardState, tunnel_id: i32, config: TunnelConfig) {
    if !guard.enabled.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(mut guarded) = guard.guarded_processes.lock() {
        guarded.insert(
            tunnel_id,
            ProcessGuardInfo {
                tunnel_id,
                tunnel_type: GuardTunnelType::Api { config },
            },
        );
    }
    if let Ok(mut stopped) = guard.manually_stopped.lock() {
        stopped.remove(&tunnel_id);
    }
}

pub fn add_guarded_custom_tunnel(guard: &GuardState, tunnel_id_hash: i32, original_id: String) {
    if !guard.enabled.load(Ordering::SeqCst) {
        return;
    }
    if let Ok(mut guarded) = guard.guarded_processes.lock() {
        guarded.insert(
            tunnel_id_hash,
            ProcessGuardInfo {
                tunnel_id: tunnel_id_hash,
                tunnel_type: GuardTunnelType::Custom { original_id },
            },
        );
    }
    if let Ok(mut stopped) = guard.manually_stopped.lock() {
        stopped.remove(&tunnel_id_hash);
    }
}

pub fn remove_guarded_process(guard: &GuardState, tunnel_id: i32, is_manual_stop: bool) {
    if let Ok(mut guarded) = guard.guarded_processes.lock() {
        guarded.remove(&tunnel_id);
    }
    if is_manual_stop {
        if let Ok(mut stopped) = guard.manually_stopped.lock() {
            stopped.insert(tunnel_id);
        }
    }
}