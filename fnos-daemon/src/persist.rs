//! PID 持久化与孤儿进程回收。
//!
//! 规格源：src-tauri/src/commands/process_persistence.rs（平台分支照抄）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const RUNNING_TUNNELS_FILE: &str = "running_tunnels.json";

/// 保存隧道进程信息（与桌面版 PersistedTunnelInfo 一致）。
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PersistedTunnelInfo {
    pub tunnel_id: i32,
    pub pid: u32,
    pub tunnel_type: String,
    pub original_id: Option<String>,
    pub started_at: String,
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
    pub fn save_running_tunnel(
        &self,
        tunnel_id: i32,
        pid: u32,
        tunnel_type: &str,
        original_id: Option<String>,
    ) -> Result<(), String> {
        let path = self.path();
        let mut tunnels = load_persisted_tunnels_from_file(&path);

        tunnels.insert(
            tunnel_id,
            PersistedTunnelInfo {
                tunnel_id,
                pid,
                tunnel_type: tunnel_type.to_string(),
                original_id,
                started_at: chrono::Local::now().to_rfc3339(),
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
            if is_process_alive(info.pid) {
                still_running.push(info.clone());
                updated.insert(tunnel_id, info);
            }
        }

        let _ = write_persisted_tunnels(&path, &updated);
        still_running
    }

    /// 获取仍在运行的隧道列表（不清理）。
    pub fn get_running_tunnels(&self) -> Vec<PersistedTunnelInfo> {
        let path = self.path();
        let tunnels = load_persisted_tunnels_from_file(&path);
        tunnels
            .values()
            .filter(|info| is_process_alive(info.pid))
            .cloned()
            .collect()
    }

    /// 按 PID 终止孤儿进程（进程管理器之外的残留）。
    pub fn kill_orphan(&self, tunnel_id: i32, pid: u32) -> Result<String, String> {
        if is_process_alive(pid) {
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

/// 检查进程是否在运行（平台分支照抄桌面版）。
fn is_process_alive(pid: u32) -> bool {
    #[cfg(target_os = "windows")]
    {
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
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
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