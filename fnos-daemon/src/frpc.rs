//! frpc 进程管理：配置生成 / spawn / 日志管道 / token 脱敏。
//!
//! 规格源：src-tauri/src/commands/process.rs + utils.rs（函数逻辑照抄，
//! 仅将 tauri 类型（AppHandle / State / Emitter）替换为 daemon 自有依赖：
//! 数据目录 / 进程表 / broadcast 事件通道）。

use crate::events::{Event, LogMessage};
use crate::persist::Persistence;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt::Write;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command as StdCommand, Stdio};
use std::sync::Mutex;
use std::thread;
use tokio::sync::broadcast;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// 隧道配置（与桌面版 models::TunnelConfig 一致，camelCase 反序列化）。
#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
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

/// frpc 进程管理器。
pub struct FrpcManager {
    pub data_dir: PathBuf,
    pub processes: Mutex<HashMap<i32, Child>>,
    pub persistence: Persistence,
    pub events: broadcast::Sender<Event>,
}

impl FrpcManager {
    pub fn new(data_dir: PathBuf, events: broadcast::Sender<Event>) -> Self {
        Self {
            data_dir: data_dir.clone(),
            processes: Mutex::new(HashMap::new()),
            persistence: Persistence::new(&data_dir),
            events,
        }
    }

    fn emit(&self, event: Event) {
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

        {
            let procs = self
                .processes
                .lock()
                .map_err(|e| format!("获取进程锁失败: {}", e))?;
            if procs.contains_key(&tunnel_id) {
                return Err("该隧道已在运行中".to_string());
            }
        }

        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| format!("创建应用目录失败: {}", e))?;

        let config_path = self.data_dir.join(format!("g_{}.ini", tunnel_id));
        let config_content = generate_frpc_config(&config)?;
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

        let mut child = cmd.spawn().map_err(|e| format!("启动 frpc 失败: {}", e))?;
        let pid = child.id();

        let timestamp = chrono::Local::now().format("%Y/%m/%d %H:%M:%S").to_string();
        self.emit(Event::log(LogMessage {
            tunnel_id,
            message: format!(
                "[I] [ChmlFrpLauncher] frpc 进程已启动 (PID: {}), 开始连接服务器...",
                pid
            ),
            timestamp,
        }));

        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(
                self.events.clone(),
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
                tunnel_id,
                user_token,
                node_token,
                stderr,
                true,
            );
        }

        {
            let mut procs = self
                .processes
                .lock()
                .map_err(|e| format!("获取进程锁失败: {}", e))?;
            procs.insert(tunnel_id, child);
        }

        let _ = self
            .persistence
            .save_running_tunnel(tunnel_id, pid, "api", None);

        Ok(format!("frpc 已启动 (PID: {})", pid))
    }

    /// 停止官方隧道 frpc（MutexGuard 先 drop 再走孤儿回收，顺序约束照桌面版）。
    pub async fn stop_frpc(&self, tunnel_id: i32) -> Result<String, String> {
        let found_in_manager = {
            let mut procs = self
                .processes
                .lock()
                .map_err(|e| format!("获取进程锁失败: {}", e))?;

            if let Some(mut child) = procs.remove(&tunnel_id) {
                let result = match child.kill() {
                    Ok(_) => {
                        let _ = child.wait();
                        Ok("frpc 已停止".to_string())
                    }
                    Err(e) => {
                        let _ = child.wait();
                        Err(format!("停止进程失败: {}", e))
                    }
                };
                let _ = self.persistence.remove_running_tunnel(tunnel_id);
                let config_path = self.data_dir.join(format!("g_{}.ini", tunnel_id));
                if config_path.exists() {
                    let _ = std::fs::remove_file(&config_path);
                }
                return result;
            }
            false
        };

        if !found_in_manager {
            let app_dir = self.data_dir.clone();
            if let Some(info) = self
                .persistence
                .get_running_tunnels()
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
        } else {
            Ok("frpc 已停止".to_string())
        }
    }

    /// 查询隧道是否运行（进程表优先，持久化 PID 兜底）。
    pub fn is_frpc_running(&self, tunnel_id: i32) -> Result<bool, String> {
        let in_process_manager = {
            let mut procs = self
                .processes
                .lock()
                .map_err(|e| format!("获取进程锁失败: {}", e))?;

            if let Some(child) = procs.get_mut(&tunnel_id) {
                match child.try_wait() {
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

        for (tunnel_id, child) in procs.iter_mut() {
            match child.try_wait() {
                Ok(None) => running_tunnels.push(*tunnel_id),
                _ => stopped_tunnels.push(*tunnel_id),
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

/// 日志管道：逐行剥离 ANSI → token 脱敏 → 打时间戳 → 推事件。
fn spawn_log_reader(
    events: broadcast::Sender<Event>,
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

                if events
                    .send(Event::log(LogMessage {
                        tunnel_id,
                        message,
                        timestamp,
                    }))
                    .is_err()
                {
                    break;
                }
            }
        })
    {
        eprintln!("[错误] 创建 {} 监听线程失败: {}", thread_name_log, e);
    }
}

/// 生成 frpc 配置文件（照桌面版 generate_frpc_config）。
fn generate_frpc_config(config: &TunnelConfig) -> Result<String, String> {
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

    result = result.replace(&format!("{}.", token), "");
    result = result.replace(&format!("{}-", token), "");
    result = result.replace(token, "");

    if let Some(dot_pos) = token.find('.') {
        let first_part = &token[..dot_pos];
        let second_part = &token[dot_pos + 1..];

        if first_part.len() >= 6 {
            result = result.replace(first_part, "***");
        }
        if second_part.len() >= 6 {
            result = result.replace(second_part, "***");
        }
    }

    if token.len() >= 10 {
        for window_size in (8..=token.len()).rev() {
            if window_size <= token.len() {
                let substr = &token[..window_size];
                if result.contains(substr) && substr.len() >= 8 {
                    result = result.replace(substr, "***");
                }
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::sanitize_log;

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

    #[test]
    fn 空token与正常文本不受影响() {
        let msg = "normal log line: connecting to server 127.0.0.1:7000";
        let out = sanitize_log(msg, &[""]);
        assert_eq!(out, msg);
    }
}