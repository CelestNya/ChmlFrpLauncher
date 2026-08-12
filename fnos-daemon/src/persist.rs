//! PID 持久化与孤儿进程回收。
//!
//! 规格源：src-tauri/src/commands/process_persistence.rs（平台分支照抄）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const RUNNING_TUNNELS_FILE: &str = "running_tunnels.json";

/// 保存隧道进程信息（与桌面版 PersistedTunnelInfo 一致 + fnOS 守护恢复扩展）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PersistedTunnelInfo {
    pub tunnel_id: i32,
    pub pid: u32,
    pub tunnel_type: String,
    pub original_id: Option<String>,
    pub started_at: String,
    /// fnOS（daemon 中-8）：完整隧道配置快照，daemon 重启后据此重新注册守护。
    /// 旧格式文件无此字段，反序列化容错为 None（旧记录不恢复守护）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<crate::frpc::TunnelConfig>,
    /// fnOS（daemon 中-4）：进程身份（unix 下为 /proc/<pid>/stat 的 start_time），
    /// PID 复用后凭此识别「PID 还在但已不是 frpc」。旧格式/Windows 为 None（回退
    /// 仅按 PID 存活判断）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<u64>,
}

pub struct Persistence {
    data_dir: std::path::PathBuf,
}

impl Persistence {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
        }
    }

    fn path(&self) -> std::path::PathBuf {
        self.data_dir.join(RUNNING_TUNNELS_FILE)
    }

    /// 保存运行中的隧道信息。
    #[allow(clippy::too_many_arguments)]
    pub fn save_running_tunnel(
        &self,
        tunnel_id: i32,
        pid: u32,
        tunnel_type: &str,
        original_id: Option<String>,
        config: Option<crate::frpc::TunnelConfig>,
    ) -> Result<(), String> {
        let path = self.path();
        let mut tunnels = load_persisted_tunnels_from_file(&path);

        // daemon 中-4：记录进程身份，PID 复用后能识别「已不是 frpc」
        #[cfg(unix)]
        let start_time = process_start_time(pid);
        #[cfg(not(unix))]
        let start_time: Option<u64> = None;

        tunnels.insert(
            tunnel_id,
            PersistedTunnelInfo {
                tunnel_id,
                pid,
                tunnel_type: tunnel_type.to_string(),
                original_id,
                started_at: chrono::Local::now().to_rfc3339(),
                config,
                start_time,
            },
        );

        write_persisted_tunnels(&path, &tunnels)
    }

    /// 移除隧道信息。
    pub fn remove_running_tunnel(&self, tunnel_id: i32) -> Result<(), String> {
        let path = self.path();
        let mut tunnels = load_persisted_tunnels_from_file(&path);
        tunnels.remove(&tunnel_id);
        write_persisted_tunnels(&path, &tunnels)
    }

    /// 恢复进程状态：剔除已停止的 PID，返回仍在运行的记录。
    pub fn recover_running_tunnels(&self) -> Vec<PersistedTunnelInfo> {
        let path = self.path();
        let tunnels = load_persisted_tunnels_from_file(&path);
        let mut still_running = Vec::new();
        let mut updated = HashMap::new();

        for (tunnel_id, info) in tunnels {
            if is_process_alive(info.pid, info.start_time) {
                still_running.push(info.clone());
                updated.insert(tunnel_id, info);
            }
        }

        let _ = write_persisted_tunnels(&path, &updated);
        still_running
    }

    /// 获取仍在运行的隧道列表（仅存活 PID，不清理）。
    pub fn get_running_tunnels(&self) -> Vec<PersistedTunnelInfo> {
        let path = self.path();
        let tunnels = load_persisted_tunnels_from_file(&path);
        tunnels
            .values()
            .filter(|info| is_process_alive(info.pid, info.start_time))
            .cloned()
            .collect()
    }

    /// 获取全部隧道记录（不过滤存活状态——供孤儿清理等场景使用）。
    pub fn get_all_records(&self) -> Vec<PersistedTunnelInfo> {
        let path = self.path();
        load_persisted_tunnels_from_file(&path)
            .values()
            .cloned()
            .collect()
    }

    /// 按 PID 终止孤儿进程（进程管理器之外的残留）。
    pub fn kill_orphan(&self, tunnel_id: i32, pid: u32) -> Result<String, String> {
        // 孤儿回收不校验进程身份：调用点（stop/delete）已确认隧道上下文，
        // 无 start_time 记录时按 PID 直接杀（桌面版同语义）
        if is_process_alive(pid, None) {
            kill_process_by_pid(pid)?;
        }
        let _ = self.remove_running_tunnel(tunnel_id);
        Ok(format!("已终止进程 (PID: {})", pid))
    }
}

fn load_persisted_tunnels_from_file(path: &Path) -> HashMap<i32, PersistedTunnelInfo> {
    if !path.exists() {
        return HashMap::new();
    }
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str(&content).ok())
        .unwrap_or_default()
}

fn write_persisted_tunnels(
    path: &Path,
    tunnels: &HashMap<i32, PersistedTunnelInfo>,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let content = serde_json::to_string_pretty(tunnels).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(path, content).map_err(|e| format!("写入文件失败: {}", e))
}

/// 检查进程是否在运行（平台分支照抄桌面版 + fnOS 进程身份校验，daemon 中-4）。
///
/// `expected_start_time`：unix 下与 /proc/<pid>/stat 的 start_time 比对——PID 被
/// 系统回收复用后（原 frpc 已死、新进程占同 PID），仅凭 kill(pid, 0) 会误判
/// 「仍运行」甚至误杀无辜进程。None（旧记录/Windows）回退为仅 PID 存活判断。
fn is_process_alive(pid: u32, expected_start_time: Option<u64>) -> bool {
    #[cfg(target_os = "windows")]
    {
        // Windows 无 start_time 身份（OpenProcess 已足够），参数仅 unix 分支使用
        let _ = expected_start_time;
        unsafe {
            let handle = windows_open_process(pid);
            if handle.is_null() {
                return false;
            }
            let mut exit_code: u32 = 0;
            let result = windows_get_exit_code(handle, &mut exit_code);
            windows_close_handle(handle);
            result != 0 && exit_code == 259
        }
    }

    #[cfg(unix)]
    {
        if unsafe { libc::kill(pid as i32, 0) } != 0 {
            return false;
        }
        match (expected_start_time, process_start_time(pid)) {
            // 无记录（旧格式）或读不到 /proc → 仅按 PID 存活
            (None, _) => true,
            (Some(_), None) => true,
            (Some(expected), Some(actual)) => expected == actual,
        }
    }
}

/// 读取 /proc/<pid>/stat 的 start_time（字段 22，进程启动时刻）。进程不存在
/// 或解析失败返回 None。
#[cfg(unix)]
fn process_start_time(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm 可能含空格/括号：从最后一个 ')' 之后解析，字段 3（state）起算，
    // start_time = 字段 22 → 后续切分 index 19
    let rest = stat.rsplit_once(')')?.1;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    fields.get(19)?.parse().ok()
}

fn kill_process_by_pid(pid: u32) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        Command::new("taskkill")
            .args(["/F", "/PID", &pid.to_string()])
            .creation_flags(0x08000000)
            .output()
            .map_err(|e| format!("终止进程失败: {}", e))?;
        Ok(())
    }

    #[cfg(unix)]
    {
        unsafe {
            if libc::kill(pid as i32, libc::SIGTERM) != 0 {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
        Ok(())
    }
}

#[cfg(target_os = "windows")]
unsafe fn windows_open_process(pid: u32) -> *mut std::ffi::c_void {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(
            dwDesiredAccess: u32,
            bInheritHandle: i32,
            dwProcessId: u32,
        ) -> *mut std::ffi::c_void;
    }
    OpenProcess(0x1000, 0, pid)
}

#[cfg(target_os = "windows")]
unsafe fn windows_get_exit_code(handle: *mut std::ffi::c_void, exit_code: &mut u32) -> i32 {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetExitCodeProcess(hProcess: *mut std::ffi::c_void, lpExitCode: *mut u32) -> i32;
    }
    GetExitCodeProcess(handle, exit_code)
}

#[cfg(target_os = "windows")]
unsafe fn windows_close_handle(handle: *mut std::ffi::c_void) {
    #[link(name = "kernel32")]
    extern "system" {
        fn CloseHandle(hObject: *mut std::ffi::c_void) -> i32;
    }
    CloseHandle(handle);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frpc::TunnelConfig;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fnos-persist-{}-{}",
            name,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_config() -> TunnelConfig {
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

    #[test]
    fn 配置快照roundtrip() {
        // daemon 中-8：重启后守护恢复需要完整 TunnelConfig，持久化时必须带快照
        let dir = temp_dir("roundtrip");
        let p = Persistence::new(&dir);
        p.save_running_tunnel(1, std::process::id(), "api", None, Some(sample_config()))
            .unwrap();
        let records = p.get_all_records();
        assert_eq!(records.len(), 1);
        let config = records[0].config.as_ref().expect("缺少 config 快照");
        assert_eq!(config.tunnel_id, 1);
        assert_eq!(config.server_addr, "test.example.com");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 旧格式文件兼容_缺config字段() {
        let dir = temp_dir("legacy");
        std::fs::write(
            dir.join("running_tunnels.json"),
            r#"{"1":{"tunnel_id":1,"pid":1,"tunnel_type":"api","original_id":null,"started_at":"2026-08-12T00:00:00Z"}}"#,
        )
        .unwrap();
        let p = Persistence::new(&dir);
        let records = p.get_all_records();
        assert_eq!(records.len(), 1);
        assert!(records[0].config.is_none(), "旧文件缺 config 应容错为 None");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recover剔除死亡pid() {
        let dir = temp_dir("recover");
        let p = Persistence::new(&dir);
        // 起一个短命进程并 reap，拿到确定已死的 pid
        let dead_pid = {
            #[cfg(unix)]
            let mut c = std::process::Command::new("sh")
                .args(["-c", "exit 0"])
                .spawn()
                .unwrap();
            #[cfg(windows)]
            let mut c = std::process::Command::new("cmd")
                .args(["/C", "exit 0"])
                .spawn()
                .unwrap();
            let pid = c.id();
            c.wait().unwrap();
            pid
        };
        p.save_running_tunnel(2, dead_pid, "api", None, None).unwrap();
        p.save_running_tunnel(3, std::process::id(), "api", None, None)
            .unwrap();
        let recovered = p.recover_running_tunnels();
        assert_eq!(recovered.len(), 1, "死亡 pid 未剔除");
        assert_eq!(recovered[0].tunnel_id, 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn 进程身份校验_识别pid复用() {
        // daemon 中-4：PID 被复用后 start_time 不匹配 → 判死，不误杀新进程
        let dir = temp_dir("identity");
        let p = Persistence::new(&dir);

        // 起一个长命进程并记录其真实 start_time
        let mut child = std::process::Command::new("sleep")
            .arg("300")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        p.save_running_tunnel(1, pid, "api", None, None).unwrap();
        let saved = p.get_all_records();
        let start_time = saved[0].start_time.expect("应记录 start_time");

        // 真实身份 → 存活
        assert!(is_process_alive(pid, Some(start_time)));
        // 身份不匹配（模拟复用：原进程已死、同 PID 被新进程占用）→ 判死
        assert!(!is_process_alive(pid, Some(start_time + 1)));
        // 无身份记录 → 回退仅 PID 存活
        assert!(is_process_alive(pid, None));

        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn 进程身份校验_死进程判死() {
        let dir = temp_dir("identity-dead");
        let p = Persistence::new(&dir);
        let mut child = std::process::Command::new("sleep")
            .arg("0.1")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        p.save_running_tunnel(1, pid, "api", None, None).unwrap();
        let start_time = p.get_all_records()[0].start_time.unwrap();
        let _ = child.wait();
        assert!(!is_process_alive(pid, Some(start_time)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}