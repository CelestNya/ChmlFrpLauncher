//! 运行配置：数据目录、前端目录、监听地址与网关 socket。
//!
//! fnOS 环境变量约定（developer.fnnas.com）：
//! - TRIM_PKGVAR：应用运行时数据目录（重启保留）——数据落盘首选
//! - TRIM_SERVICE_PORT：manifest.service_port 注入的服务端口
//! - TRIM_APPDEST：应用 target 目录（前端 dist 与网关 socket 所在）
//! 开发模式下回退本地默认值，便于本机调试。

use std::net::SocketAddr;
use std::path::PathBuf;

/// 开发模式默认端口（避开常用端口，fnOS 下由 TRIM_SERVICE_PORT 覆盖）。
const DEV_DEFAULT_PORT: u16 = 17890;

/// 统一网关注入的 socket 文件名（ui/config 的 gatewaySocket）。
const GATEWAY_SOCKET_NAME: &str = "app.sock";

#[derive(Clone)]
pub struct DaemonConfig {
    /// 数据目录：frpc 二进制 / 隧道配置 / PID 持久化 / 自定义隧道清单
    pub data_dir: PathBuf,
    /// 前端静态资源目录（SPA dist；fnOS 下 TRIM_APPDEST/dist，开发可覆盖）
    pub web_dir: PathBuf,
    /// 应用 target 目录（fnOS 下 TRIM_APPDEST；网关 socket 在此创建）
    pub app_dest: Option<PathBuf>,
    /// 监听地址（默认仅回环，统一网关在 fnOS 侧转发）
    pub listen_addr: SocketAddr,
}

impl DaemonConfig {
    pub fn load() -> Self {
        let data_dir = env_var_path("TRIM_PKGVAR")
            .or_else(|| env_var_path("TRIM_PKGDATA"))
            .unwrap_or_else(default_data_dir);

        let app_dest = env_var_path("TRIM_APPDEST");
        let web_dir = app_dest
            .clone()
            .map(|d| d.join("dist"))
            .or_else(|| env_var_path("DAEMON_WEB_DIR"))
            .unwrap_or_else(|| default_data_dir().join("web"));

        let port = parse_service_port(std::env::var("TRIM_SERVICE_PORT").ok()).0;

        Self {
            data_dir,
            web_dir,
            app_dest,
            listen_addr: SocketAddr::from(([127, 0, 0, 1], port)),
        }
    }

    /// 统一网关 socket 路径（TRIM_APPDEST/app.sock）；开发模式无 TRIM_APPDEST 则禁用。
    pub fn gateway_socket_path(&self) -> Option<PathBuf> {
        self.app_dest
            .as_ref()
            .map(|d| d.join(GATEWAY_SOCKET_NAME))
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

/// 解析服务端口（TRIM_SERVICE_PORT）。返回 (端口, 是否告警过)：
/// 非法/越界值时回退默认端口并告警（原实现静默回退，生产端口错配无任何提示）。
fn parse_service_port(raw: Option<String>) -> (u16, bool) {
    let Some(raw) = raw else {
        return (DEV_DEFAULT_PORT, false);
    };
    match raw.trim().parse::<u16>() {
        Ok(port) => (port, false),
        Err(_) => {
            tracing::warn!(
                "TRIM_SERVICE_PORT 非法（{raw}），回退默认端口 {DEV_DEFAULT_PORT}"
            );
            (DEV_DEFAULT_PORT, true)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 端口解析() {
        assert_eq!(parse_service_port(Some("17890".to_string())), (17890, false));
        assert_eq!(parse_service_port(Some(" 8080 ".to_string())), (8080, false));
        assert_eq!(parse_service_port(None), (DEV_DEFAULT_PORT, false));
        let (port, warned) = parse_service_port(Some("not-a-port".to_string()));
        assert_eq!(port, DEV_DEFAULT_PORT);
        assert!(warned, "非法端口应触发告警");
        let (port, warned) = parse_service_port(Some("99999".to_string()));
        assert_eq!(port, DEV_DEFAULT_PORT);
        assert!(warned, "越界端口应触发告警");
    }
}
