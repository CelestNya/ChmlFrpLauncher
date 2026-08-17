//! frpc 进程管理：配置生成 / spawn / 日志管道 / token 脱敏。
//!
//! 规格源：src-tauri/src/commands/process.rs + utils.rs（函数逻辑照抄，
//! 仅将 tauri 类型（AppHandle / State / Emitter）替换为 daemon 自有依赖：
//! 数据目录 / 进程表 / broadcast 事件通道）。

use crate::events::{push_log_history, Event, LogHistory, LogMessage};
use crate::guard::{self, GuardState};
use crate::persist::Persistence;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Write;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// 隧道配置（与桌面版 models::TunnelConfig 一致）。
///
/// ⚠️ 字段保持 snake_case：Tauri 2 只对 invoke **顶层参数键**做 camelCase→snake_case
/// 转换，嵌套结构体按 Rust 字段名原样反序列化——前端 frpcManager 构造的 config
/// 就是 snake_case（tunnel_id / server_addr / ...），daemon 必须用默认字段名匹配。
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TunnelConfig {
    pub tunnel_id: i32,
    pub tunnel_name: String,
    pub user_token: String,
    pub server_addr: String,
    pub server_port: u16,
    pub node_token: String,
    pub tunnel_type: String,
    pub local_ip: String,
    pub local_port: u16,
    pub remote_port: Option<u16>,
    pub custom_domains: Option<String>,
    pub http_proxy: Option<String>,
    pub log_level: String,
    pub force_tls: bool,
    pub kcp_optimization: bool,
}

/// 进程表条目状态（daemon 高-3 修复）。
///
/// `Starting`：并发 start 的原子占位——防止两个并发 start_frpc 都通过
/// `contains_key` 检查后各自 spawn，后 insert 覆盖前一个 `Child` 导致孤儿进程
/// （无人追踪、无法 stop、不参与守护、unix 下僵尸）。
pub enum ProcessEntry {
    Starting,
    Running(Child),
}

/// 带超时的子进程回收（daemon 中-5 修复）。
///
/// 原实现 `Child::wait()` 无限期阻塞且在进程表锁内调用：kill 失败时锁被
/// 永久持有，所有隧道的 start/stop/query 全部挂起。本函数最多等 `timeout`
/// 后放弃，调用方决定是否继续持有资源。
pub fn wait_child_timeout(child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) => {
                if Instant::now() >= deadline {
                    return false;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return false,
        }
    }
}

/// frpc 进程管理器。
pub struct FrpcManager {
    pub data_dir: PathBuf,
    pub processes: Mutex<HashMap<i32, ProcessEntry>>,
    pub persistence: Persistence,
    pub events: broadcast::Sender<Event>,
    pub log_history: LogHistory,
    /// 日志持久化文件（2026-08-17：daemon 重启后日志不丢；None = 未初始化）
    pub logfile: Option<Arc<crate::logfile::LogFile>>,
    pub guard: Arc<GuardState>,
}

impl FrpcManager {
    pub fn new(
        data_dir: PathBuf,
        events: broadcast::Sender<Event>,
        log_history: LogHistory,
        logfile: Option<Arc<crate::logfile::LogFile>>,
        guard: Arc<GuardState>,
    ) -> Self {
        Self {
            data_dir: data_dir.clone(),
            processes: Mutex::new(HashMap::new()),
            persistence: Persistence::new(&data_dir),
            events,
            log_history,
            logfile,
            guard,
        }
    }

    /// 发布日志事件：写历史缓冲（WS 断线补发）+ 持久化文件 + 广播实时推送。
    pub fn emit_log(&self, msg: LogMessage) {
        let event = Event::log(msg);
        push_log_history(&self.log_history, self.logfile.as_deref(), &event);
        let _ = self.events.send(event);
    }

    /// 定位 frpc 可执行文件（D2：数据目录优先，FRPC_PATH 内置兜底，缺省报错待下载）。
    pub fn resolve_frpc_path(&self) -> Result<PathBuf, String> {
        let in_data = self.data_dir.join(frpc_file_name());
        if in_data.exists() {
            return Ok(in_data);
        }

        if let Ok(bundled) = std::env::var("FRPC_PATH") {
            let bundled_path = PathBuf::from(bundled);
            if bundled_path.exists() {
                if let Some(parent) = in_data.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if std::fs::copy(&bundled_path, &in_data).is_ok() {
                    set_executable(&in_data);
                    return Ok(in_data);
                }
                return Ok(bundled_path);
            }
        }

        Err("frpc 未找到，请先下载".to_string())
    }

    /// 启动官方隧道 frpc。
    pub async fn start_frpc(&self, config: TunnelConfig) -> Result<String, String> {
        let tunnel_id = config.tunnel_id;
        let user_token = config.user_token.clone();
        let node_token = config.node_token.clone();

        // 原子检查 + 占位（daemon 高-3）：并发 start 同 id 只能有一个通过；
        // 占位后任一步失败必须回滚，否则该 id 永远无法再启动。
        {
            let mut procs = self
                .processes
                .lock()
                .map_err(|e| format!("获取进程锁失败: {}", e))?;
            if procs.contains_key(&tunnel_id) {
                return Err("该隧道已在运行中或正在启动".to_string());
            }
            procs.insert(tunnel_id, ProcessEntry::Starting);
        }

        let (mut child, pid) = match self.spawn_frpc(&config) {
            Ok(v) => v,
            Err(e) => {
                // 回滚占位（锁中毒时占位残留可接受——进程表已处于异常状态）
                if let Ok(mut procs) = self.processes.lock() {
                    procs.remove(&tunnel_id);
                }
                return Err(e);
            }
        };

        let timestamp = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
        self.emit_log(LogMessage {
            tunnel_id,
            message: format!(
                "[I] [ChmlFrpLauncher] frpc 进程已启动 (PID: {}), 开始连接服务器...",
                pid
            ),
            timestamp,
        });

        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(
                self.events.clone(),
                self.log_history.clone(),
                self.logfile.clone(),
                tunnel_id,
                user_token.clone(),
                node_token.clone(),
                stdout,
                false,
            );
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(
                self.events.clone(),
                self.log_history.clone(),
                self.logfile.clone(),
                tunnel_id,
                user_token,
                node_token,
                stderr,
                true,
            );
        }

        // 占位 → Running；若占位已被并发 stop 移除，取回 child 并杀掉（尊重用户停止）
        let orphan = {
            let mut procs = self
                .processes
                .lock()
                .map_err(|e| format!("获取进程锁失败: {}", e))?;
            match procs.get_mut(&tunnel_id) {
                Some(entry) if matches!(entry, ProcessEntry::Starting) => {
                    *entry = ProcessEntry::Running(child);
                    None
                }
                _ => Some(child),
            }
        };
        if let Some(mut orphan) = orphan {
            let _ = orphan.kill();
            let _ = wait_child_timeout(&mut orphan, Duration::from_secs(5));
            return Ok("frpc 已停止".to_string());
        }

        let _ = self.persistence.save_running_tunnel(
            tunnel_id,
            pid,
            "api",
            None,
            Some(config.clone()),
        );

        // 加入守护集合（守护启用时生效）
        guard::add_guarded_process(&self.guard, tunnel_id, config);

        Ok(format!("frpc 已启动 (PID: {})", pid))
    }

    /// spawn frpc 子进程（配置落盘 + spawn；不含进程表操作，供 start_frpc 占位后调用）。
    fn spawn_frpc(&self, config: &TunnelConfig) -> Result<(Child, u32), String> {
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| format!("创建应用目录失败: {}", e))?;

        let config_path = self.data_dir.join(format!("g_{}.ini", config.tunnel_id));
        let config_content = generate_frpc_config(config)?;
        std::fs::write(&config_path, config_content)
            .map_err(|e| format!("写入配置文件失败: {}", e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::metadata(&config_path)
                .map_err(|e| format!("获取配置文件权限失败: {}", e))?
                .permissions();
            let mut perms = perms;
            perms.set_mode(0o600);
            std::fs::set_permissions(&config_path, perms)
                .map_err(|e| format!("设置配置文件权限失败: {}", e))?;
        }

        let frpc_path = self.resolve_frpc_path()?;

        let mut cmd = StdCommand::new(&frpc_path);
        cmd.current_dir(&self.data_dir)
            .arg("-c")
            .arg(&config_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            cmd.creation_flags(0x08000000);
        }

        let child = cmd.spawn().map_err(|e| format!("启动 frpc 失败: {}", e))?;
        let pid = child.id();
        Ok((child, pid))
    }

    /// 停止官方隧道 frpc（daemon 中-5：先取出进程表条目、释放锁，再 kill + 带超时回收）。
    pub async fn stop_frpc(&self, tunnel_id: i32) -> Result<String, String> {
        // 先移出守护集合并标记手动停止（照桌面版 stop_frpc 首步）
        guard::remove_guarded_process(&self.guard, tunnel_id, true);

        let removed = {
            let mut procs = self
                .processes
                .lock()
                .map_err(|e| format!("获取进程锁失败: {}", e))?;
            procs.remove(&tunnel_id)
        };

        if let Some(entry) = removed {
            if let ProcessEntry::Running(mut child) = entry {
                // 锁外 kill；回收阻塞移入 spawn_blocking（async 上下文不持锁阻塞）
                child
                    .kill()
                    .map_err(|e| format!("停止进程失败: {}", e))?;
                let waited = tokio::task::spawn_blocking(move || {
                    wait_child_timeout(&mut child, Duration::from_secs(5))
                })
                .await
                .unwrap_or(false);
                if !waited {
                    let timestamp =
                        chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
                    self.emit_log(LogMessage {
                        tunnel_id,
                        message: "[W] [ChmlFrpLauncher] frpc 进程 5 秒内未退出，放弃等待"
                            .to_string(),
                        timestamp,
                    });
                }
            }
            // Starting 占位：spawn 可能尚未发生；移除占位即停止
            //（start 侧升级失败时会自行杀掉刚 spawn 的进程）
            let _ = self.persistence.remove_running_tunnel(tunnel_id);
            let config_path = self.data_dir.join(format!("g_{}.ini", tunnel_id));
            if config_path.exists() {
                let _ = std::fs::remove_file(&config_path);
            }
            return Ok("frpc 已停止".to_string());
        }

        // 不在进程管理器：孤儿回收（持久化 PID 兜底）
        let app_dir = self.data_dir.clone();
        if let Some(info) = self
            .persistence
            .get_all_records()
            .into_iter()
            .find(|t| t.tunnel_id == tunnel_id)
        {
            let _ = self.persistence.kill_orphan(tunnel_id, info.pid);
        }
        let config_path = app_dir.join(format!("g_{}.ini", tunnel_id));
        if config_path.exists() {
            let _ = std::fs::remove_file(&config_path);
        }
        Ok("frpc 已停止".to_string())
    }

    /// 查询隧道是否运行（进程表优先，持久化 PID 兜底）。
    pub fn is_frpc_running(&self, tunnel_id: i32) -> Result<bool, String> {
        let in_process_manager = {
            let mut procs = self
                .processes
                .lock()
                .map_err(|e| format!("获取进程锁失败: {}", e))?;

            if let Some(entry) = procs.get_mut(&tunnel_id) {
                match entry {
                    // 占位态：启动中视为运行（守护不会误判离线而重复拉起）
                    ProcessEntry::Starting => Some(true),
                    ProcessEntry::Running(child) => match child.try_wait() {
                        Ok(Some(_)) => {
                            procs.remove(&tunnel_id);
                            let _ = self.persistence.remove_running_tunnel(tunnel_id);
                            Some(false)
                        }
                        Ok(None) => Some(true),
                        Err(_) => {
                            procs.remove(&tunnel_id);
                            let _ = self.persistence.remove_running_tunnel(tunnel_id);
                            Some(false)
                        }
                    },
                }
            } else {
                None
            }
        };

        if let Some(running) = in_process_manager {
            return Ok(running);
        }

        // 不在进程管理器中，检查持久化的 PID
        Ok(self
            .persistence
            .get_running_tunnels()
            .iter()
            .any(|t| t.tunnel_id == tunnel_id))
    }

    /// 获取运行中的隧道列表（进程表 + 持久化 PID 兜底）。
    pub fn get_running_tunnels(&self) -> Result<Vec<i32>, String> {
        let mut procs = self
            .processes
            .lock()
            .map_err(|e| format!("获取进程锁失败: {}", e))?;

        let mut running_tunnels = Vec::new();
        let mut stopped_tunnels = Vec::new();

        for (tunnel_id, entry) in procs.iter_mut() {
            match entry {
                ProcessEntry::Starting => running_tunnels.push(*tunnel_id),
                ProcessEntry::Running(child) => match child.try_wait() {
                    Ok(None) => running_tunnels.push(*tunnel_id),
                    _ => stopped_tunnels.push(*tunnel_id),
                },
            }
        }
        for tunnel_id in &stopped_tunnels {
            procs.remove(tunnel_id);
            let _ = self.persistence.remove_running_tunnel(*tunnel_id);
        }

        for info in self.persistence.recover_running_tunnels() {
            if !running_tunnels.contains(&info.tunnel_id) {
                running_tunnels.push(info.tunnel_id);
            }
        }

        Ok(running_tunnels)
    }
}

/// 日志管道：逐行剥离 ANSI → token 脱敏 → 打时间戳 → 写历史缓冲 → 推事件。
///
/// ⚠️ send 失败**不能**退出循环：网关空闲超时会周期性断开 WS（fnOS 实测约 60s），
/// 断线窗口内 broadcast 无订阅者、send 返回 Err；若此时退出，日志线程将永久死亡，
/// 重连后 frpc 输出也永远不会再推送。事件丢弃可接受，线程必须存活。
/// （pub(crate)：custom.rs 复用，传空 secret 即不脱敏。）
#[allow(clippy::too_many_arguments)] // 日志线程参数均为独立配置项，成簇收益低
pub(crate) fn spawn_log_reader(
    events: broadcast::Sender<Event>,
    log_history: LogHistory,
    logfile: Option<Arc<crate::logfile::LogFile>>,
    tunnel_id: i32,
    user_token: String,
    node_token: String,
    reader: impl std::io::Read + Send + 'static,
    is_stderr: bool,
) {
    let thread_name = if is_stderr {
        format!("frpc-stderr-{}", tunnel_id)
    } else {
        format!("frpc-stdout-{}", tunnel_id)
    };
    let thread_name_log = thread_name.clone();

    if let Err(e) = thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            let reader = BufReader::new(reader);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                let clean_line = strip_ansi_escapes::strip_str(&line);
                let sanitized_line =
                    sanitize_log(&clean_line, &[user_token.as_str(), node_token.as_str()]);
                let timestamp = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();

                let message = if is_stderr {
                    format!("[ERR] {}", sanitized_line)
                } else {
                    sanitized_line
                };

                let event = Event::log(LogMessage {
                    tunnel_id,
                    message,
                    timestamp,
                });
                push_log_history(&log_history, logfile.as_deref(), &event);
                let _ = events.send(event);
            }
        })
    {
        eprintln!("[错误] 创建 {} 监听线程失败: {}", thread_name_log, e);
    }
}

/// 生成 frpc 配置文件（照桌面版 generate_frpc_config）。
/// ini 值合法性：拒绝换行/NUL 等控制字符——前端传入的 server_addr/token 若含
/// 换行可注入 ini 行（daemon 低-配置注入面修复）。
fn valid_ini_value(v: &str) -> bool {
    !v.chars().any(|c| c == '\n' || c == '\r' || c == '\0')
}

fn generate_frpc_config(config: &TunnelConfig) -> Result<String, String> {
    // 关键字段字符集校验（防 ini 行注入）
    for (field, value) in [
        ("server_addr", config.server_addr.as_str()),
        ("user_token", config.user_token.as_str()),
        ("node_token", config.node_token.as_str()),
        ("tunnel_name", config.tunnel_name.as_str()),
        ("local_ip", config.local_ip.as_str()),
        ("log_level", config.log_level.as_str()),
    ] {
        if !valid_ini_value(value) {
            return Err(format!("{field} 包含非法字符"));
        }
    }
    if let Some(ref proxy) = config.http_proxy {
        if !valid_ini_value(proxy) {
            return Err("http_proxy 包含非法字符".to_string());
        }
    }

    let mut content = String::new();

    writeln!(content, "[common]").unwrap();
    writeln!(content, "server_addr = {}", config.server_addr).unwrap();
    writeln!(content, "server_port = {}", config.server_port).unwrap();

    if let Some(ref proxy_url) = config.http_proxy {
        writeln!(content, "http_proxy = {}", proxy_url).unwrap();
    }

    writeln!(content, "log_level = {}", config.log_level).unwrap();
    writeln!(content, "tls_enable = {}", config.force_tls).unwrap();
    writeln!(content, "tcp_mux = true").unwrap();
    writeln!(content, "pool_count = 5").unwrap();

    if config.kcp_optimization && (config.tunnel_type == "tcp" || config.tunnel_type == "udp") {
        writeln!(content, "protocol = kcp").unwrap();
    }

    writeln!(content, "user = {}", config.user_token).unwrap();
    writeln!(content, "token = {}", config.node_token).unwrap();
    writeln!(content).unwrap();

    writeln!(content, "[{}]", config.tunnel_name).unwrap();
    writeln!(content, "type = {}", config.tunnel_type).unwrap();
    writeln!(content, "local_ip = {}", config.local_ip).unwrap();
    writeln!(content, "local_port = {}", config.local_port).unwrap();

    match config.tunnel_type.as_str() {
        "tcp" | "udp" => {
            if let Some(remote_port) = config.remote_port {
                writeln!(content, "remote_port = {}", remote_port).unwrap();
            } else {
                return Err("TCP/UDP 隧道缺少 remote_port 参数".to_string());
            }
        }
        "http" | "https" => {
            if let Some(ref custom_domains) = config.custom_domains {
                writeln!(content, "custom_domains = {}", custom_domains).unwrap();
            } else {
                return Err("HTTP/HTTPS 隧道缺少 custom_domains 参数".to_string());
            }
        }
        _ => {
            return Err(format!("不支持的隧道类型: {}", config.tunnel_type));
        }
    }

    Ok(content)
}

fn frpc_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "frpc.exe"
    } else {
        "frpc"
    }
}

fn set_executable(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mut perms = metadata.permissions();
            if perms.mode() & 0o111 == 0 {
                perms.set_mode(0o755);
                let _ = std::fs::set_permissions(path, perms);
            }
        }
    }
    let _ = path;
}

/// 隐藏用户日志里的 token（照抄桌面版 utils::sanitize_log）。
pub fn sanitize_log(message: &str, secrets: &[&str]) -> String {
    let mut result = message.to_string();
    for secret in secrets {
        if secret.is_empty() {
            continue;
        }
        result = sanitize_token(&result, secret);
    }
    result
}

fn sanitize_token(message: &str, token: &str) -> String {
    let mut result = message.to_string();

    // 1. 完整 token 及其常见粘连形式（token. / token-）直接移除
    result = result.replace(&format!("{}.", token), "");
    result = result.replace(&format!("{}-", token), "");
    result = result.replace(token, "");

    // 2. 逐段掩码：token 的每个 `.` 分割段（长度 ≥6）单独替换。
    //    覆盖「日志只含某一段（尾段/中段单独出现、与上下文粘连）」的场景；
    //    短段（如 "token"）不掩码——是常见词，掩码会误伤正常日志。
    for seg in token.split('.') {
        if seg.len() >= 6 {
            result = result.replace(seg, "***");
        }
    }

    // 3. 滑动窗口兜底：token 的任意 ≥8 连续子串（含后缀窗口，旧实现只扫前缀）。
    //    覆盖无 `.` 分隔的长 token 被截断打印的场景。窗口从大到小，
    //    避免短窗口先替换破坏长窗口的匹配。
    if token.len() >= 8 {
        for window_size in (8..=token.len()).rev() {
            for start in 0..=token.len() - window_size {
                let substr = &token[start..start + window_size];
                if result.contains(substr) {
                    result = result.replace(substr, "***");
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_TOKEN: &str = "user.token.abc123.ABC789xyz";
    const NODE_TOKEN: &str = "node.token.xyz789.NOP456abc";

    #[test]
    fn 完整token被移除() {
        let msg = format!("user={} node={} hello", USER_TOKEN, NODE_TOKEN);
        let out = sanitize_log(&msg, &[USER_TOKEN, NODE_TOKEN]);
        assert!(!out.contains(USER_TOKEN), "完整 user token 泄漏: {out}");
        assert!(!out.contains(NODE_TOKEN), "完整 node token 泄漏: {out}");
    }

    #[test]
    fn token分片被替换() {
        let msg = format!("error in {} at {}", USER_TOKEN, NODE_TOKEN);
        let out = sanitize_log(&msg, &[USER_TOKEN, NODE_TOKEN]);
        assert!(!out.contains("abc123"), "user token 分片泄漏: {out}");
        assert!(!out.contains("xyz789"), "node token 分片泄漏: {out}");
    }

    #[test]
    fn 长尾token片段不泄漏() {
        let msg = format!("prefix blocks: {} suffix", NODE_TOKEN);
        let out = sanitize_log(&msg, &[NODE_TOKEN]);
        assert!(!out.contains("NOP456abc"), "token 长片段泄漏: {out}");
        assert!(out.contains("suffix"), "正常内容被误伤: {out}");
    }

    // —— 以下为「真实分片场景」测试：消息只含 token 的某一段，不含完整 token ——
    // （旧测试的消息含完整 token，整串先被移除，从未覆盖分片单独出现的场景）

    #[test]
    fn 中段分片单独出现不泄漏() {
        let out = sanitize_log("error in abc123 end", &[USER_TOKEN]);
        assert!(!out.contains("abc123"), "中段分片泄漏: {out}");
        assert!(out.contains("error in") && out.contains("end"), "正常内容被误伤: {out}");
    }

    #[test]
    fn 尾段分片单独出现不泄漏() {
        let out = sanitize_log("tail ABC789xyz", &[USER_TOKEN]);
        assert!(!out.contains("ABC789xyz"), "尾段分片泄漏: {out}");
        assert!(out.contains("tail"), "正常内容被误伤: {out}");
    }

    #[test]
    fn 分片与上下文粘连不泄漏() {
        let out = sanitize_log("xxabc123yy", &[USER_TOKEN]);
        assert!(!out.contains("abc123"), "粘连分片泄漏: {out}");
    }

    #[test]
    fn 短分片不掩码_避免误伤常见词() {
        // "token"（5 字符）是常见词，掩码会误伤正常日志——只掩码 ≥6 的分段
        let out = sanitize_log("the token field", &[USER_TOKEN]);
        assert!(out.contains("token"), "短分片不应被掩码: {out}");
    }

    // ---- ini 注入防护（daemon 低-配置注入面） ----

    #[test]
    fn ini值校验_拒绝控制字符() {
        assert!(valid_ini_value("1.2.3.4"));
        assert!(valid_ini_value(""));
        assert!(!valid_ini_value("1.2.3.4\nproxy=evil"), "换行可注入 ini 行");
        assert!(!valid_ini_value("a\rb"));
        assert!(!valid_ini_value("a\0b"));
    }

    #[test]
    fn 配置生成拒绝注入的server_addr() {
        let mut config = test_config();
        config.server_addr = "x\ninjected = 1".to_string();
        let err = generate_frpc_config(&config).unwrap_err();
        assert!(err.contains("非法字符"), "err: {err}");
    }

    // —— A5/A6：进程表占位状态 + 锁外 kill/回收 ——

    use crate::events::LogHistory;
    use crate::guard::GuardState;
    use std::collections::VecDeque;
    use std::sync::Arc;

    fn test_manager(data_dir: std::path::PathBuf) -> Arc<FrpcManager> {
        let (tx, _) = broadcast::channel(16);
        let history: LogHistory = Arc::new(Mutex::new(VecDeque::new()));
        Arc::new(FrpcManager::new(
            data_dir,
            tx,
            history,
            None,
            GuardState::new(),
        ))
    }

    fn test_config() -> TunnelConfig {
        TunnelConfig {
            tunnel_id: 1,
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

    fn temp_data_dir(suffix: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fnos-frpc-{}-{}-{}",
            suffix,
            std::process::id(),
            std::thread::current().name().unwrap_or("t").replace(':', "_")
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[tokio::test]
    async fn start失败时回滚占位() {
        // 无 frpc 二进制且无 FRPC_PATH → spawn 前报错，Starting 占位必须回滚，
        // 否则该 tunnel_id 永远无法再启动（daemon 高-3）
        let dir = temp_data_dir("rollback");
        let mgr = test_manager(dir.clone());
        let err = mgr.start_frpc(test_config()).await.unwrap_err();
        assert!(err.contains("frpc 未找到"), "err: {err}");
        assert!(mgr.processes.lock().unwrap().is_empty(), "占位未回滚");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 并发start同id被占位拒绝() {
        let dir = temp_data_dir("double-start");
        let mgr = test_manager(dir.clone());
        mgr.processes
            .lock()
            .unwrap()
            .insert(1, ProcessEntry::Starting);
        let err = mgr.start_frpc(test_config()).await.unwrap_err();
        assert!(err.contains("已在运行中或正在启动"), "err: {err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 停止占位态_干净移除() {
        let dir = temp_data_dir("stop-starting");
        let mgr = test_manager(dir.clone());
        mgr.processes
            .lock()
            .unwrap()
            .insert(1, ProcessEntry::Starting);
        let res = mgr.stop_frpc(1).await.unwrap();
        assert!(res.contains("已停止"), "res: {res}");
        assert!(mgr.processes.lock().unwrap().is_empty(), "占位未移除");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn stop_锁外kill并回收子进程() {
        // daemon 中-5：kill/wait 不得在进程表锁内无限阻塞——用 sleep 假进程验证
        let dir = temp_data_dir("kill-reap");
        let mgr = test_manager(dir.clone());
        let child = StdCommand::new("sleep")
            .arg("300")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        mgr.processes
            .lock()
            .unwrap()
            .insert(1, ProcessEntry::Running(child));

        let res = mgr.stop_frpc(1).await.unwrap();
        assert!(res.contains("已停止"), "res: {res}");

        // 子进程已被 kill 并回收（kill(pid, 0) 返回 ESRCH）
        let alive = unsafe { libc::kill(pid as i32, 0) } == 0;
        assert!(!alive, "子进程仍存活（pid {pid}）");
        // 锁可再次获取（未被死锁持有）
        assert!(mgr.processes.lock().is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 空token与正常文本不受影响() {
        let msg = "normal log line: connecting to server 127.0.0.1:7000";
        let out = sanitize_log(msg, &[""]);
        assert_eq!(out, msg);
    }
}