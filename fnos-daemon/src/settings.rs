//! 凭据与节点设置的后端持久化（前后端分离重构·阶段1）。
//!
//! 目标（ADR-0002）：token/明文代理密码物理离开浏览器 localStorage，落到
//! daemon 文件（0600），shim 重定向转发。本模块是 daemon 侧权威存储：
//! - `credential.json`：登录态（accessToken/refreshToken/usertoken）
//! - `node_settings.json`：影响 frpc 启动/后端出网的配置（代理/日志级别/bypass/重启行为）
//!
//! 持久化模式照抄 persist.rs：损坏文件备份后重建、文件权限收紧、中文测试。
//! 登出顺序（ADR-0002 定死）：先停隧道清 g_*.ini → 再 clear_credential。
//! 该顺序由前端登出编排（logout.ts：先 stopAllRunningTunnels 再 clearStoredUser）
//! 保证；daemon 侧只提供 clear_credential 存储原语。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const CREDENTIAL_FILE: &str = "credential.json";
const NODE_SETTINGS_FILE: &str = "node_settings.json";

/// 登录态集合（Credential）。与前端 StoredUser 的 token 字段对应。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Credential {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usertoken: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token_expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_type: Option<String>,
    // 用户画像字段（前端 StoredUser 完整往返，review P2：丢失会破坏会员门控）
    // usergroup 是会员等级判断的关键（NodeSelector/EditTunnelDialog 用
    // user?.usergroup 决定免费/会员），丢字段 = 免费用户可绕过 VIP 节点拦截。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usergroup: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub userimg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel: Option<i32>,
}

impl Credential {
    /// 是否有可用登录态（任一 token 存在即视为已登录）。
    pub fn is_logged_in(&self) -> bool {
        self.usertoken.is_some()
            || self.access_token.is_some()
            || self.refresh_token.is_some()
    }
}

/// 影响 frpc 启动行为/后端出网的配置（NodeSetting）。
/// 与前端 localStorage 的 `frpc_proxy_config`/`frpcLogLevel`/`bypassProxy`/
/// `restartOnEdit` 对应；proxy 字段保留明文密码（0600 保护），不落浏览器。
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NodeSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_password: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_force_tls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_kcp_optimization: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_level: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bypass_proxy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_on_edit: Option<bool>,
}

/// 凭据与节点设置的持久化存储（data_dir 下两个文件，均 0600）。
pub struct SettingsStore {
    data_dir: PathBuf,
}

impl SettingsStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            data_dir: data_dir.to_path_buf(),
        }
    }

    fn credential_path(&self) -> PathBuf {
        self.data_dir.join(CREDENTIAL_FILE)
    }

    fn node_settings_path(&self) -> PathBuf {
        self.data_dir.join(NODE_SETTINGS_FILE)
    }

    // ---- Credential ----

    /// 保存登录态（0600 写入）。
    pub fn save_credential(&self, cred: &Credential) -> Result<(), String> {
        write_json_0600(&self.credential_path(), cred)
    }

    /// 读取登录态。损坏文件备份后返回空（后续 save 重建）。
    pub fn get_credential(&self) -> Credential {
        load_json(&self.credential_path()).unwrap_or_default()
    }

    /// 清空登录态。
    pub fn clear_credential(&self) -> Result<(), String> {
        match std::fs::remove_file(self.credential_path()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删除 credential.json 失败: {}", e)),
        }
    }

    /// 登录态只读视图（不含 token 本身）。
    pub fn credential_status(&self) -> CredentialStatus {
        CredentialStatus {
            logged_in: self.get_credential().is_logged_in(),
        }
    }

    // ---- NodeSettings ----

    pub fn save_node_settings(&self, settings: &NodeSettings) -> Result<(), String> {
        write_json_0600(&self.node_settings_path(), settings)
    }

    pub fn get_node_settings(&self) -> NodeSettings {
        load_json(&self.node_settings_path()).unwrap_or_default()
    }
}

/// 登录态只读视图（CredentialStatus）：前端判断登录态用，绝不含 token。
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CredentialStatus {
    pub logged_in: bool,
}

/// 以 0600 权限写 JSON（unix）。防其他本地用户读 token/明文密码。
fn write_json_0600<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let content =
        serde_json::to_string_pretty(value).map_err(|e| format!("序列化失败: {}", e))?;
    std::fs::write(path, content).map_err(|e| format!("写入文件失败: {}", e))?;
    set_0600(path);
    Ok(())
}

/// 读取 JSON；文件缺失返回 None，损坏时备份后返回 None（与 persist.rs 同语义）。
fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    if !path.exists() {
        return None;
    }
    let content = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&content) {
        Ok(v) => Some(v),
        Err(e) => {
            let bak = path.with_extension(format!(
                "corrupt-{}",
                chrono::Local::now().format("%Y%m%d%H%M%S")
            ));
            let _ = std::fs::rename(path, &bak);
            tracing::warn!("{} 损坏，已备份到 {}: {e}", path.display(), bak.display());
            None
        }
    }
}

#[cfg(unix)]
fn set_0600(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_0600(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fnos-settings-{}-{}",
            name,
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_credential() -> Credential {
        Credential {
            username: Some("test_user".to_string()),
            usertoken: Some("legacy_token".to_string()),
            access_token: Some("access".to_string()),
            refresh_token: Some("refresh".to_string()),
            access_token_expires_at: Some(1750000000),
            token_type: Some("Bearer".to_string()),
            usergroup: Some("free".to_string()),
            userimg: Some("img.png".to_string()),
            tunnel_count: Some(3),
            tunnel: Some(2),
        }
    }

    fn sample_node_settings() -> NodeSettings {
        NodeSettings {
            proxy_enabled: Some(true),
            proxy_type: Some("http".to_string()),
            proxy_host: Some("proxy.example.com".to_string()),
            proxy_port: Some(8080),
            proxy_username: Some("user".to_string()),
            proxy_password: Some("pass".to_string()),
            proxy_force_tls: Some(false),
            proxy_kcp_optimization: Some(true),
            log_level: Some("debug".to_string()),
            bypass_proxy: Some(false),
            restart_on_edit: Some(true),
        }
    }

    #[test]
    fn credential_roundtrip() {
        let dir = temp_dir("cred-rw");
        let store = SettingsStore::new(&dir);
        store.save_credential(&sample_credential()).unwrap();
        let loaded = store.get_credential();
        assert_eq!(loaded, sample_credential());
        assert!(loaded.is_logged_in());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn credential_status_不含token() {
        let dir = temp_dir("cred-status");
        let store = SettingsStore::new(&dir);
        store.save_credential(&sample_credential()).unwrap();
        let status = store.credential_status();
        assert!(status.logged_in);
        // 只读视图只有 logged_in，不含 token 字段
        assert_eq!(
            serde_json::to_value(&status).unwrap(),
            serde_json::json!({"logged_in": true})
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn credential_clear_和未登录() {
        let dir = temp_dir("cred-clear");
        let store = SettingsStore::new(&dir);
        store.save_credential(&sample_credential()).unwrap();
        store.clear_credential().unwrap();
        assert!(!store.get_credential().is_logged_in());
        assert!(!store.credential_status().logged_in);
        // 重复 clear 幂等
        store.clear_credential().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn node_settings_roundtrip() {
        let dir = temp_dir("node-rw");
        let store = SettingsStore::new(&dir);
        store.save_node_settings(&sample_node_settings()).unwrap();
        assert_eq!(store.get_node_settings(), sample_node_settings());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 缺失文件返回默认() {
        let dir = temp_dir("missing");
        let store = SettingsStore::new(&dir);
        assert_eq!(store.get_credential(), Credential::default());
        assert_eq!(store.get_node_settings(), NodeSettings::default());
        assert!(!store.credential_status().logged_in);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 损坏文件备份后重建() {
        let dir = temp_dir("corrupt");
        let store = SettingsStore::new(&dir);
        std::fs::write(dir.join(CREDENTIAL_FILE), "{ not valid").unwrap();
        assert_eq!(store.get_credential(), Credential::default());
        assert!(!dir.join(CREDENTIAL_FILE).exists(), "损坏文件应被改名备份");
        let has_backup = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("corrupt-"));
        assert!(has_backup);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn 文件权限收紧为0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perm");
        let store = SettingsStore::new(&dir);
        store.save_credential(&sample_credential()).unwrap();
        store.save_node_settings(&sample_node_settings()).unwrap();
        let cred_mode = std::fs::metadata(dir.join(CREDENTIAL_FILE))
            .unwrap()
            .permissions()
            .mode();
        let node_mode = std::fs::metadata(dir.join(NODE_SETTINGS_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(cred_mode & 0o777, 0o600, "credential.json 权限未收紧");
        assert_eq!(node_mode & 0o777, 0o600, "node_settings.json 权限未收紧");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
