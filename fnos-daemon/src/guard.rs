//! 进程守护：3s 轮询 + 错误日志模式智能停止 + 自动重启。
//!
//! 规格源：src-tauri/src/commands/process_guard.rs。
//! 差异：桌面版用 std::thread + block_on；daemon 全 tokio（select 合并
//! 3s tick 与事件监听两个职责）；**D1：守护默认开启**（桌面版默认关）。
//!
//! fnOS 修复（2026-08-13 批 A9，daemon 中-1/2/3/12）：
//! - 手动停止 vs 在途重启竞态：restart 的延时 sleep 之后二次检查 manually_stopped
//! - 重启失败退避：连续 3 次失败才移除守护（瞬时错误不再静默失联守护），
//!   失败计数驱动退避延时（1s→2s→4s→8s）
//! - broadcast Lagged：重新订阅并告警（原实现 Ok(event) 模式不匹配，
//!   事件分支永久卡在 Lagged，STOP 模式日志全部漏掉）
//! - 守护经 GuardOps trait 抽象，可 mock 单测（tick/restart 状态机）

use crate::custom::CustomManager;
use crate::events::{AutoRestartedPayload, Event, LogMessage};
use crate::frpc::{FrpcManager, TunnelConfig};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
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

/// 连续重启失败达到该次数才移除守护（瞬时错误不立即失联守护）。
const MAX_RESTART_FAILURES: u32 = 3;

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
    /// 重启失败计数（daemon 中-2：驱动退避与 3 次移除判定）
    pub restart_failures: Mutex<HashMap<i32, u32>>,
}

impl GuardState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            enabled: AtomicBool::new(true),
            guarded_processes: Mutex::new(HashMap::new()),
            manually_stopped: Mutex::new(HashSet::new()),
            restart_failures: Mutex::new(HashMap::new()),
        })
    }
}

/// 守护对隧道运行状态 / 启动 / 日志事件的依赖面（trait 抽象供 mock 单测）。
/// 方法返回 Send future：monitor 运行于 tokio::spawn（多线程 runtime）。
pub trait GuardOps: Send + Sync {
    fn is_api_running(
        &self,
        tunnel_id: i32,
    ) -> impl std::future::Future<Output = bool> + Send;
    fn is_custom_running(
        &self,
        original_id: &str,
    ) -> impl std::future::Future<Output = bool> + Send;
    fn start_api(
        &self,
        config: TunnelConfig,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn start_custom(
        &self,
        original_id: String,
    ) -> impl std::future::Future<Output = Result<String, String>> + Send;
    fn emit_log(&self, msg: LogMessage);
    fn emit_event(&self, event: Event);
}

/// 生产实现：桥接 FrpcManager + CustomManager。
pub struct GuardOpsImpl {
    pub frpc: Arc<FrpcManager>,
    pub custom: Arc<CustomManager>,
}

impl GuardOps for GuardOpsImpl {
    async fn is_api_running(&self, tunnel_id: i32) -> bool {
        self.frpc.is_frpc_running(tunnel_id).unwrap_or(false)
    }
    async fn is_custom_running(&self, original_id: &str) -> bool {
        self.custom
            .is_custom_tunnel_running(original_id.to_string())
            .unwrap_or(false)
    }
    async fn start_api(&self, config: TunnelConfig) -> Result<String, String> {
        self.frpc.start_frpc(config).await
    }
    async fn start_custom(&self, original_id: String) -> Result<String, String> {
        self.custom.start_custom_tunnel(original_id).await
    }
    fn emit_log(&self, msg: LogMessage) {
        self.frpc.emit_log(msg);
    }
    fn emit_event(&self, event: Event) {
        let _ = self.frpc.events.send(event);
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
pub fn start_guard_monitor<T: GuardOps + 'static>(
    guard: Arc<GuardState>,
    ops: Arc<T>,
    events: broadcast::Sender<Event>,
) {
    tokio::spawn(async move {
        let mut rx = events.subscribe();

        loop {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(3)) => {
                    tick(&guard, &ops).await;
                }
                event = rx.recv() => {
                    match event {
                        Ok(Event { event_type: "frpc-log", payload }) => {
                            if let Ok(log) = serde_json::from_value::<LogMessage>(payload) {
                                check_log_and_stop_guard(&guard, &ops, log).await;
                            }
                        }
                        Ok(_) => {}
                        // daemon 中-3：Lagged 后 resubscribe 并告警；原实现 Ok(event)
                        // 模式不匹配，事件分支永久卡在 Lagged，STOP 模式日志全部漏掉
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!("[守护进程] 事件订阅滞后 {n} 条，已重新订阅");
                            rx = events.subscribe();
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        }
    });
}

async fn tick<T: GuardOps>(guard: &Arc<GuardState>, ops: &Arc<T>) {
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
            GuardTunnelType::Api { .. } => ops.is_api_running(tunnel_id).await,
            GuardTunnelType::Custom { original_id } => {
                ops.is_custom_running(original_id).await
            }
        };
        if running {
            // 恢复运行：清零失败计数（此前失败过但最终成功）
            reset_restart_failures(guard, tunnel_id);
            continue;
        }

        // 走 emit_log（缓冲+广播）：若用裸 events.send，断线窗口内（无 WS 订阅者）
        // 守护消息会丢失，且不进补发缓冲。与 frpc 日志行为保持一致。
        ops.emit_log(LogMessage {
            tunnel_id,
            message: "[W] [ChmlFrpLauncher] 检测到进程离线，触发守护进程，自动重启中".to_string(),
            timestamp: get_timestamp(),
        });

        restart_tunnel(guard, ops, info).await;
    }
}

async fn restart_tunnel<T: GuardOps>(
    guard: &Arc<GuardState>,
    ops: &Arc<T>,
    info: ProcessGuardInfo,
) {
    let tunnel_id = info.tunnel_id;

    // 失败退避延时：1s、2s、4s、8s（封顶）；首次为 1s，与桌面版一致
    let failures = current_restart_failures(guard, tunnel_id);
    let delay_secs = (1u64 << failures.min(3)).min(8);
    tokio::time::sleep(Duration::from_secs(delay_secs)).await;

    // daemon 中-1：TOCTOU 二次检查——延时 sleep 期间用户可能手动停止，
    // 醒来后若已标记手动停止则放弃重启（尊重用户意图，避免计费流量）
    if is_manually_stopped(guard, tunnel_id) {
        return;
    }

    let result = match &info.tunnel_type {
        GuardTunnelType::Api { config } => ops.start_api(config.clone()).await,
        GuardTunnelType::Custom { original_id } => ops.start_custom(original_id.clone()).await,
    };

    match result {
        Ok(_) => {
            reset_restart_failures(guard, tunnel_id);
            ops.emit_event(Event::auto_restarted(AutoRestartedPayload {
                tunnel_id,
                timestamp: get_timestamp(),
            }));
        }
        Err(e) => {
            ops.emit_log(LogMessage {
                tunnel_id,
                message: format!("[E] [ChmlFrpLauncher] 守护进程重启失败: {}", e),
                timestamp: get_timestamp(),
            });

            // daemon 中-2：连续 MAX 次失败才移除守护（瞬时错误不再静默失联）；
            // 崩溃风暴由失败计数驱动的退避延时抑制
            let failures = bump_restart_failures(guard, tunnel_id);
            if failures >= MAX_RESTART_FAILURES {
                if let Ok(mut guarded) = guard.guarded_processes.lock() {
                    guarded.remove(&tunnel_id);
                }
                reset_restart_failures(guard, tunnel_id);
                ops.emit_log(LogMessage {
                    tunnel_id,
                    message: format!(
                        "[E] [ChmlFrpLauncher] 连续 {MAX_RESTART_FAILURES} 次重启失败，已移除守护"
                    ),
                    timestamp: get_timestamp(),
                });
            }
        }
    }
}

fn current_restart_failures(guard: &GuardState, tunnel_id: i32) -> u32 {
    guard
        .restart_failures
        .lock()
        .ok()
        .map(|m| m.get(&tunnel_id).copied().unwrap_or(0))
        .unwrap_or(0)
}

fn bump_restart_failures(guard: &GuardState, tunnel_id: i32) -> u32 {
    let mut map = match guard.restart_failures.lock() {
        Ok(m) => m,
        Err(_) => return u32::MAX,
    };
    let n = map.get(&tunnel_id).copied().unwrap_or(0) + 1;
    map.insert(tunnel_id, n);
    n
}

fn reset_restart_failures(guard: &GuardState, tunnel_id: i32) {
    if let Ok(mut map) = guard.restart_failures.lock() {
        map.remove(&tunnel_id);
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
async fn check_log_and_stop_guard<T: GuardOps>(
    guard: &Arc<GuardState>,
    ops: &Arc<T>,
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

    ops.emit_log(LogMessage {
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
        if let Ok(mut failures) = guard.restart_failures.lock() {
            failures.clear();
        }
    }
    format!(
        "守护进程已{}",
        if enabled { "启用" } else { "禁用" }
    )
}

/// daemon 中-12：重新启用守护时，把仍在运行的隧道重新登记（disable 清空了
/// guarded 集合，原实现 enable 后存量隧道失去自动重启覆盖）。
pub fn set_process_guard_enabled_with_recovery(
    guard: &GuardState,
    enabled: bool,
    running: &[crate::persist::PersistedTunnelInfo],
) -> String {
    let msg = set_process_guard_enabled(guard, enabled);
    if enabled {
        re_register_guarded(guard, running);
    }
    msg
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

/// daemon 中-8：重启后把存量运行隧道重新登记进守护集合，恢复自动重启覆盖
/// （原实现只打印日志，重启后守护对存量隧道失联）。返回重新登记数量。
pub fn re_register_guarded(
    guard: &GuardState,
    recovered: &[crate::persist::PersistedTunnelInfo],
) -> usize {
    if !guard.enabled.load(Ordering::SeqCst) {
        return 0;
    }
    let mut count = 0;
    for info in recovered {
        match info.tunnel_type.as_str() {
            "api" => {
                if let Some(config) = &info.config {
                    add_guarded_process(guard, info.tunnel_id, config.clone());
                    count += 1;
                }
            }
            "custom" => {
                if let Some(original_id) = &info.original_id {
                    add_guarded_custom_tunnel(guard, info.tunnel_id, original_id.clone());
                    count += 1;
                }
            }
            _ => {}
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frpc::TunnelConfig;
    use crate::persist::PersistedTunnelInfo;
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;

    fn sample_config(tunnel_id: i32) -> TunnelConfig {
        TunnelConfig {
            tunnel_id,
            tunnel_name: "test".to_string(),
            user_token: "user".to_string(),
            server_addr: "test.example.com".to_string(),
            server_port: 7000,
            node_token: "node".to_string(),
            tunnel_type: "tcp".to_string(),
            local_ip: "127.0.0.1".to_string(),
            local_port: 80,
            remote_port: Some(1234),
            custom_domains: None,
            http_proxy: None,
            log_level: "info".to_string(),
            force_tls: false,
            kcp_optimization: false,
        }
    }

    fn persisted(tunnel_id: i32, tunnel_type: &str, config: Option<TunnelConfig>) -> PersistedTunnelInfo {
        PersistedTunnelInfo {
            tunnel_id,
            pid: 100 + tunnel_id as u32,
            tunnel_type: tunnel_type.to_string(),
            original_id: if tunnel_type == "custom" {
                Some(format!("custom-{tunnel_id}"))
            } else {
                None
            },
            started_at: "".to_string(),
            config,
            start_time: None,
        }
    }

    /// 可编程 mock：记录 start 调用、按队列返回结果、记录日志/事件。
    struct MockOps {
        running: StdMutex<HashSet<i32>>,
        custom_running: StdMutex<HashSet<String>>,
        starts: StdMutex<Vec<String>>,
        start_results: StdMutex<VecDeque<Result<String, String>>>,
        logs: StdMutex<Vec<LogMessage>>,
        events: StdMutex<Vec<String>>,
    }

    impl MockOps {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                running: StdMutex::new(HashSet::new()),
                custom_running: StdMutex::new(HashSet::new()),
                starts: StdMutex::new(Vec::new()),
                start_results: StdMutex::new(VecDeque::new()),
                logs: StdMutex::new(Vec::new()),
                events: StdMutex::new(Vec::new()),
            })
        }
        fn push_result(&self, r: Result<String, String>) {
            self.start_results.lock().unwrap().push_back(r);
        }
        fn take_starts(&self) -> Vec<String> {
            std::mem::take(&mut *self.starts.lock().unwrap())
        }
        fn take_logs(&self) -> Vec<String> {
            std::mem::take(&mut *self.logs.lock().unwrap())
                .into_iter()
                .map(|l| l.message)
                .collect()
        }
    }

    impl GuardOps for MockOps {
        async fn is_api_running(&self, tunnel_id: i32) -> bool {
            self.running.lock().unwrap().contains(&tunnel_id)
        }
        async fn is_custom_running(&self, original_id: &str) -> bool {
            self.custom_running.lock().unwrap().contains(original_id)
        }
        async fn start_api(&self, config: TunnelConfig) -> Result<String, String> {
            self.starts
                .lock()
                .unwrap()
                .push(format!("api:{}", config.tunnel_id));
            self.start_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok("ok".to_string()))
        }
        async fn start_custom(&self, original_id: String) -> Result<String, String> {
            self.starts
                .lock()
                .unwrap()
                .push(format!("custom:{original_id}"));
            self.start_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Ok("ok".to_string()))
        }
        fn emit_log(&self, msg: LogMessage) {
            self.logs.lock().unwrap().push(msg);
        }
        fn emit_event(&self, event: Event) {
            let Event { event_type, .. } = event;
            self.events.lock().unwrap().push(event_type.to_string());
        }
    }

    // ---- 中-1：手动停止 vs 在途重启 TOCTOU ----

    #[tokio::test(start_paused = true)]
    async fn 手动停止取消在途重启() {
        let guard = GuardState::new();
        let ops = MockOps::new();
        add_guarded_process(&guard, 1, sample_config(1));

        let g = guard.clone();
        let o = ops.clone();
        let tick_task = tokio::spawn(async move { tick(&g, &o).await });
        // tick 已进入 restart 的延时 sleep（1s），此期间用户手动停止
        tokio::time::sleep(Duration::from_millis(500)).await;
        remove_guarded_process(&guard, 1, true);

        tick_task.await.unwrap();
        assert!(
            ops.take_starts().is_empty(),
            "手动停止后不应再重启: {:?}",
            ops.starts
        );
    }

    // ---- 中-2：重启失败退避 + 3 次移除 ----

    #[tokio::test(start_paused = true)]
    async fn 瞬时失败不立即移除守护() {
        let guard = GuardState::new();
        let ops = MockOps::new();
        add_guarded_process(&guard, 1, sample_config(1));

        // 第一次重启失败
        ops.push_result(Err("瞬时错误".to_string()));
        tick(&guard, &ops).await;
        assert!(guard.guarded_processes.lock().unwrap().contains_key(&1), "瞬时失败不应移除守护");
        assert!(ops.take_starts().len() == 1);

        // 第二次成功 → 清零
        tick(&guard, &ops).await;
        assert!(guard.guarded_processes.lock().unwrap().contains_key(&1));
    }

    #[tokio::test(start_paused = true)]
    async fn 连续三次失败移除守护() {
        let guard = GuardState::new();
        let ops = MockOps::new();
        add_guarded_process(&guard, 1, sample_config(1));

        for _ in 0..3 {
            ops.push_result(Err("崩溃".to_string()));
            tick(&guard, &ops).await;
        }
        assert!(
            !guard.guarded_processes.lock().unwrap().contains_key(&1),
            "连续 3 次失败应移除守护"
        );
        let logs = ops.take_logs();
        assert!(
            logs.iter().any(|m| m.contains("连续 3 次重启失败")),
            "应有移除告警日志: {logs:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn 成功重启清零失败计数() {
        let guard = GuardState::new();
        let ops = MockOps::new();
        add_guarded_process(&guard, 1, sample_config(1));

        // 失败 2 次（未达移除阈值）→ 成功 1 次 → 再失败 2 次：计数已清零，不应移除
        ops.push_result(Err("e".to_string()));
        tick(&guard, &ops).await;
        ops.push_result(Err("e".to_string()));
        tick(&guard, &ops).await;
        tick(&guard, &ops).await; // 成功（默认 Ok）
        ops.push_result(Err("e".to_string()));
        tick(&guard, &ops).await;
        ops.push_result(Err("e".to_string()));
        tick(&guard, &ops).await;

        assert!(
            guard.guarded_processes.lock().unwrap().contains_key(&1),
            "成功清零后，非连续的 2 次失败不应移除守护"
        );
    }

    // ---- STOP 模式 ----

    #[test]
    fn 错误模式命中与豁免() {
        assert_eq!(
            should_stop_guard_by_log("login failed: invalid token"),
            Some("invalid token")
        );
        assert_eq!(should_stop_guard_by_log("normal log line"), None);
        // [ChmlFrpLauncher] 内部日志豁免在 check_log_and_stop_guard 内检查
    }

    #[tokio::test]
    async fn check_log_豁免内部日志并命中外部错误() {
        let guard = GuardState::new();
        let ops = MockOps::new();
        add_guarded_process(&guard, 1, sample_config(1));

        // 内部日志：不触发移除
        check_log_and_stop_guard(
            &guard,
            &ops,
            LogMessage {
                tunnel_id: 1,
                message: "[W] [ChmlFrpLauncher] 检测到错误".to_string(),
                timestamp: "".to_string(),
            },
        )
        .await;
        assert!(guard.guarded_processes.lock().unwrap().contains_key(&1));

        // 外部错误：命中模式移除守护 + 告警日志
        check_log_and_stop_guard(
            &guard,
            &ops,
            LogMessage {
                tunnel_id: 1,
                message: "invalid token".to_string(),
                timestamp: "".to_string(),
            },
        )
        .await;
        assert!(!guard.guarded_processes.lock().unwrap().contains_key(&1));
        assert!(ops.take_logs().iter().any(|m| m.contains("已停止守护进程")));
    }

    // ---- 中-3：Lagged 后仍能处理 STOP 事件（monitor 集成） ----

    #[tokio::test]
    async fn 事件滞后后重新订阅并继续处理() {
        let guard = GuardState::new();
        let ops = MockOps::new();
        let (tx, _) = broadcast::channel::<Event>(64);
        add_guarded_process(&guard, 1, sample_config(1));

        start_guard_monitor(guard.clone(), ops.clone(), tx.clone());
        // 先让 monitor 任务运行起来完成订阅（current_thread：await 前 spawn 不执行，
        // 否则后续同步发送时零订阅者，消息全部丢弃、Lagged 路径根本测不到）
        tokio::task::yield_now().await;

        // 同步灌满 500 条（monitor 任务不会在同步循环中运行），必然触发 monitor 的 rx Lagged
        for i in 0..500 {
            let _ = tx.send(Event::log(LogMessage {
                tunnel_id: 1,
                message: format!("flood {i}"),
                timestamp: "".to_string(),
            }));
        }

        // 循环重发 STOP 模式日志直到被处理：broadcast 的新订阅者不重放已发送
        // 消息（resubscribe 前的帧收不到），重发保证命中 resubscribe 之后。
        // 若事件分支死在 Lagged（原 bug：Ok(event) 模式不匹配），重发永远不会被
        // 处理 → 超时失败。
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if !guard.guarded_processes.lock().unwrap().contains_key(&1) {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "Lagged 后 STOP 事件未被处理（守护未移除）：guarded={} logs={}",
                    guard.guarded_processes.lock().unwrap().len(),
                    ops.logs.lock().unwrap().len(),
                );
            }
            let _ = tx.send(Event::log(LogMessage {
                tunnel_id: 1,
                message: "invalid token".to_string(),
                timestamp: "".to_string(),
            }));
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    // ---- 中-8 / 中-12：恢复注册与 enable 重注册 ----

    #[test]
    fn 恢复记录重新注册守护() {
        let guard = GuardState::new();
        let recovered = vec![
            persisted(1, "api", Some(sample_config(1))),
            persisted(2, "custom", None),
            // 旧格式记录：无 config 快照 → 不注册（无法重启恢复）
            persisted(3, "api", None),
        ];
        let n = re_register_guarded(&guard, &recovered);
        assert_eq!(n, 2, "只有带快照/原 id 的记录可注册");
        let guarded = guard.guarded_processes.lock().unwrap();
        assert_eq!(guarded.len(), 2);
        assert!(guarded.contains_key(&1));
        assert!(guarded.contains_key(&2));
    }

    #[test]
    fn 守护禁用时注册不生效() {
        let guard = GuardState::new();
        guard.enabled.store(false, Ordering::SeqCst);
        let recovered = vec![persisted(1, "api", Some(sample_config(1)))];
        assert_eq!(re_register_guarded(&guard, &recovered), 0);
        assert!(guard.guarded_processes.lock().unwrap().is_empty());
    }

    #[test]
    fn 重新启用时恢复存量隧道守护() {
        let guard = GuardState::new();
        add_guarded_process(&guard, 1, sample_config(1));

        // 禁用 → guarded 清空
        set_process_guard_enabled(&guard, false);
        assert!(guard.guarded_processes.lock().unwrap().is_empty());

        // 启用 + 存量运行隧道 → 重新注册
        let running = vec![persisted(1, "api", Some(sample_config(1)))];
        let msg = set_process_guard_enabled_with_recovery(&guard, true, &running);
        assert!(msg.contains("启用"));
        assert!(guard.guarded_processes.lock().unwrap().contains_key(&1));
    }
}
