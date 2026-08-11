//! 运行配置：数据目录与监听地址。
//!
//! fnOS 环境变量约定（developer.fnnas.com）：
//! - TRIM_PKGVAR：应用运行时数据目录（重启保留）——数据落盘首选
//! - TRIM_SERVICE_PORT：manifest.service_port 注入的服务端口
//! 开发模式下回退本地默认值，便于本机调试。

use std::net::SocketAddr;
use std::path::PathBuf;

/// 开发模式默认端口（避开常用端口，fnOS 下由 TRIM_SERVICE_PORT 覆盖）。
const DEV_DEFAULT_PORT: u16 = 17890;

#[derive(Clone)]
pub struct DaemonConfig {
    /// 数据目录：frpc 二进制 / 隧道配置 / PID 持久化 / 自定义隧道清单
    pub data_dir: PathBuf,
    /// 监听地址（默认仅回环，统一网关在 fnOS 侧转发）
    pub listen_addr: SocketAddr,
}

impl DaemonConfig {
    pub fn load() -> Self {
        let data_dir = env_var_path("TRIM_PKGVAR")
            .or_else(|| env_var_path("TRIM_PKGDATA"))
            .unwrap_or_else(default_data_dir);

        let port = std::env::var("TRIM_SERVICE_PORT")
            .ok()
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(DEV_DEFAULT_PORT);

        Self {
            data_dir,
            listen_addr: SocketAddr::from(([127, 0, 0, 1], port)),
        }
    }

    /// 确保数据目录存在。
    pub fn ensure_dirs(&self) {
        if let Err(e) = std::fs::create_dir_all(&self.data_dir) {
            tracing::warn!("创建数据目录失败: {e}");
        }
    }
}

fn env_var_path(name: &str) -> Option<PathBuf> {
    let v = std::env::var(name).ok()?;
    if v.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(v))
}

fn default_data_dir() -> PathBuf {
    // 开发模式：可被 DAEMON_DATA_DIR 覆盖，否则用系统数据目录
    if let Some(dir) = env_var_path("DAEMON_DATA_DIR") {
        return dir;
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("data")
}
