//! 自更新（B5）：GitHub Releases 源 → 下载更新 bundle → sha256 校验 → 原子替换 → 重启。
//!
//! 更新源：`https://api.github.com/repos/CelestNya/ChmlFrpLauncher/releases/latest`
//! 更新包（release asset）：`chmlfrp-fnos-{version}-{platform}.tar.gz`，平台 x86/arm。
//! bundle 内部：
//! ```text
//! manifest.json   # { version, platform, files: { "chmlfrp-daemon": "<sha256>", "dist/...": "<sha256>", ... } }
//! chmlfrp-daemon
//! dist/...
//! ```
//! 流程：check（版本比对 + 5 分钟缓存，规避 GitHub API 60 次/时限流）→
//! download（流式下载到 TRIM_PKGVAR/update/staged，逐文件 sha256 校验）→
//! apply（备份 → 原子替换 target 下 daemon 与 dist/ → spawn 新进程 → 通知优雅关闭）。
//!
//! 替换位置：target 目录（TRIM_APPDEST）。run-as=package 对 target 写权限若受限，
//! 本模块报明确错误并提示手动更新（plan 待验证清单第 4 条）。

use crate::config::DaemonConfig;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::broadcast;
use tracing::{info, warn};

const GITHUB_API_LATEST: &str =
    "https://api.github.com/repos/CelestNya/ChmlFrpLauncher/releases/latest";
const UPDATE_CACHE_SECS: u64 = 300;
const BUNDLE_PREFIX: &str = "chmlfrp-fnos-";

/// 平台后缀（daemon 运行架构 → bundle 平台名，与 manifest platform 一致）。
fn platform_suffix() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm",
        _ => "x86",
    }
}

/// GitHub API 限流规避：进程内缓存最近一次检查结果。
#[derive(Clone)]
pub struct UpdateChecker {
    inner: Arc<Mutex<Option<(Instant, UpdateInfo)>>>,
}

impl Default for UpdateChecker {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
        }
    }
}

/// 可用更新信息（与前端 UpdateInfo 对齐）。
#[derive(Clone, serde::Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub version: Option<String>,
    pub url: Option<String>,
    pub size: Option<u64>,
}

/// 更新 bundle 元数据（manifest.json）。
#[derive(serde::Deserialize)]
struct BundleManifest {
    version: String,
    platform: String,
    /// 相对路径 → sha256（含 daemon 二进制与 dist/ 全部文件）
    files: HashMap<String, String>,
}

fn build_http_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .user_agent("ChmlFrpLauncher-fnos-updater/1.0")
        .no_proxy()
        .build()
        .map_err(|e| format!("创建 HTTP 客户端失败: {e}"))
}

fn version_compare(current: &str, remote: &str) -> bool {
    fn parts(v: &str) -> Vec<u64> {
        v.trim_start_matches('v')
            .split(['.', '-'])
            .filter_map(|s| s.parse::<u64>().ok())
            .collect()
    }
    let a = parts(current);
    let b = parts(remote);
    for (x, y) in a.iter().zip(b.iter()) {
        if x != y {
            return x < y;
        }
    }
    a.len() < b.len()
}

/// 查询最新 release 并匹配本平台 bundle。
/// `UPDATE_API_URL` 环境变量可覆盖 GitHub API 地址（测试 / 自托管源 / 规避限流）。
async fn fetch_latest(checker: &UpdateChecker, force: bool) -> Result<UpdateInfo, String> {
    // 缓存命中（5 分钟内）
    if !force {
        if let Ok(guard) = checker.inner.lock() {
            if let Some((at, info)) = guard.as_ref() {
                if at.elapsed().as_secs() < UPDATE_CACHE_SECS {
                    return Ok(info.clone());
                }
            }
        }
    }

    let client = build_http_client(15)?;
    let api_url = std::env::var("UPDATE_API_URL").unwrap_or_else(|_| GITHUB_API_LATEST.to_string());
    let resp = client
        .get(&api_url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("请求更新源失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API 返回 {}", resp.status()));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("解析 GitHub 响应失败: {e}"))?;

    let tag = json["tag_name"].as_str().unwrap_or_default().to_string();
    let version = tag.trim_start_matches('v').to_string();
    let platform = platform_suffix();

    // 匹配本平台 bundle asset：chmlfrp-fnos-{version}-{platform}.tar.gz
    let mut matched: Option<(String, u64)> = None;
    if let Some(assets) = json["assets"].as_array() {
        for asset in assets {
            let name = asset["name"].as_str().unwrap_or_default();
            let target = format!("{BUNDLE_PREFIX}{version}-{platform}.tar.gz");
            if name == target {
                matched = Some((
                    asset["browser_download_url"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string(),
                    asset["size"].as_u64().unwrap_or(0),
                ));
                break;
            }
        }
    }

    let current = env!("CARGO_PKG_VERSION");
    let info = match matched {
        Some((url, size)) => UpdateInfo {
            available: version_compare(current, &version),
            version: Some(version),
            url: Some(url),
            size: Some(size),
        },
        None => UpdateInfo {
            available: false,
            version: None,
            url: None,
            size: None,
        },
    };

    if let Ok(mut guard) = checker.inner.lock() {
        *guard = Some((Instant::now(), info.clone()));
    }
    Ok(info)
}

/// GET /api/update/check —— 检查是否有可用更新。
pub async fn handle_check(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
) -> axum::Json<serde_json::Value> {
    match fetch_latest(&state.update, false).await {
        Ok(info) => axum::Json(serde_json::json!({ "ok": true, "data": info })),
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "error": e })),
    }
}

/// POST /api/update/download —— 下载并校验更新 bundle。
pub async fn handle_download(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
) -> axum::Json<serde_json::Value> {
    match download_update(&state.cfg, &state.update).await {
        Ok(staged) => axum::Json(serde_json::json!({
            "ok": true,
            "data": staged.to_string_lossy()
        })),
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "error": e })),
    }
}

/// POST /api/update/apply —— 应用更新并重启。
pub async fn handle_apply(
    axum::extract::State(state): axum::extract::State<crate::AppState>,
) -> axum::Json<serde_json::Value> {
    match apply_update(&state.cfg, &state.shutdown) {
        Ok(()) => axum::Json(serde_json::json!({ "ok": true, "data": "更新已应用，服务重启中" })),
        Err(e) => axum::Json(serde_json::json!({ "ok": false, "error": e })),
    }
}

/// 查询最新 release 并匹配本平台 bundle。
pub async fn download_update(
    cfg: &DaemonConfig,
    checker: &UpdateChecker,
) -> Result<PathBuf, String> {
    let info = fetch_latest(checker, false).await?;
    if !info.available {
        return Err("当前已是最新版本".to_string());
    }
    let url = info.url.ok_or("未找到更新包下载地址")?;
    let version = info.version.as_deref().unwrap_or("latest");
    let platform = platform_suffix();

    let update_dir = cfg.data_dir.join("update");
    let staged = update_dir.join("staged");
    std::fs::create_dir_all(&staged).map_err(|e| format!("创建更新目录失败: {e}"))?;

    // 清理旧的 staged（避免残留影响校验）
    for entry in std::fs::read_dir(&staged).map_err(|e| format!("读取更新目录失败: {e}"))? {
        let p = entry.map_err(|e| e.to_string())?.path();
        let _ = std::fs::remove_file(&p);
    }

    let bundle_path = update_dir.join(format!("{BUNDLE_PREFIX}{version}-{platform}.tar.gz"));
    let client = build_http_client(120)?;
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("下载更新包失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("下载更新包返回 {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("读取更新包失败: {e}"))?;
    std::fs::write(&bundle_path, &bytes).map_err(|e| format!("写入更新包失败: {e}"))?;

    // 解包到 staged
    unpack_tar_gz(&bundle_path, &staged)?;

    // 校验 manifest + 逐文件 sha256
    verify_bundle(&staged)?;

    info!("更新包下载并校验通过: {}", bundle_path.display());
    Ok(staged)
}

/// POST /api/update/apply —— 应用 staged 更新并重启。
/// 返回后旧进程将优雅退出，新进程已由本函数 spawn（同端口，SO_REUSEPORT 衔接）。
pub fn apply_update(
    cfg: &DaemonConfig,
    shutdown_tx: &broadcast::Sender<()>,
) -> Result<(), String> {
    let staged = cfg.data_dir.join("update").join("staged");
    if !staged.join("chmlfrp-daemon").exists() {
        return Err("未找到已下载的更新（请先执行 download）".to_string());
    }
    // 应用前再校验一次（防止 staged 被篡改）
    verify_bundle(&staged)?;

    let target = cfg.web_dir.parent().ok_or("无法确定 target 目录")?.to_path_buf();
    if !target.exists() {
        return Err(format!("target 目录不存在: {}", target.display()));
    }

    // 1. 备份并替换 daemon 二进制
    let daemon_dst = target.join("chmlfrp-daemon");
    backup_and_replace(&staged.join("chmlfrp-daemon"), &daemon_dst)?;

    // 2. 备份并替换 dist/ 整目录
    let dist_dst = target.join("dist");
    let dist_bak = target.join("dist.bak");
    if dist_bak.exists() {
        let _ = std::fs::remove_dir_all(&dist_bak);
    }
    if dist_dst.exists() {
        std::fs::rename(&dist_dst, &dist_bak).map_err(|e| format!("备份 dist 失败: {e}"))?;
    }
    let staged_dist = staged.join("dist");
    if staged_dist.exists() {
        std::fs::rename(&staged_dist, &dist_dst).map_err(|e| format!("替换 dist 失败: {e}"))?;
    }

    // 3. 权限：保持可执行
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&daemon_dst, std::fs::Permissions::from_mode(0o755));
    }

    info!("更新文件已替换，准备重启");
    // 4. spawn 新进程（同环境变量），旧进程随后由 shutdown 退出
    spawn_new_process(cfg, &daemon_dst)?;

    // 5. 广播关闭，旧进程退出（serve 优雅关闭）
    let _ = shutdown_tx.send(());
    Ok(())
}

/// 备份旧文件并原子替换（先写临时文件再 rename，避免半写状态）。
fn backup_and_replace(src: &Path, dst: &Path) -> Result<(), String> {
    let bak = dst.with_extension("bak");
    if dst.exists() {
        std::fs::rename(dst, &bak).map_err(|e| format!("备份 {} 失败: {e}", dst.display()))?;
    }
    std::fs::copy(src, dst).map_err(|e| format!("替换 {} 失败: {e}", dst.display()))?;
    Ok(())
}

/// spawn 新 daemon（继承环境变量），并更新 PID 文件（cmd/main 依赖）。
fn spawn_new_process(cfg: &DaemonConfig, daemon_bin: &Path) -> Result<(), String> {
    let data_dir = cfg.data_dir.clone();
    let mut cmd = std::process::Command::new(daemon_bin);
    cmd.current_dir(&data_dir);
    // 继承全部环境变量（TRIM_APPDEST / TRIM_PKGVAR / TRIM_SERVICE_PORT 等由 fnOS 注入）
    let child = cmd
        .spawn()
        .map_err(|e| format!("启动新 daemon 失败: {e}"))?;

    let pid = child.id();
    let pid_file = data_dir.join("chmlfrp.pid");
    if let Err(e) = std::fs::write(&pid_file, pid.to_string()) {
        warn!("写入 PID 文件失败: {e}");
    }
    info!("新 daemon 已启动 (PID {pid})");
    Ok(())
}

/// 解包 .tar.gz 到目标目录。
fn unpack_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive).map_err(|e| format!("打开更新包失败: {e}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);
    tar.unpack(dest).map_err(|e| format!("解包更新包失败: {e}"))
}

/// 逐文件校验 staged 内容与 manifest.files 表一致。
fn verify_bundle(staged: &Path) -> Result<(), String> {
    let manifest_path = staged.join("manifest.json");
    let manifest_raw = std::fs::read_to_string(&manifest_path)
        .map_err(|_| "更新包缺少 manifest.json".to_string())?;
    let manifest: BundleManifest = serde_json::from_str(&manifest_raw)
        .map_err(|e| format!("解析 manifest.json 失败: {e}"))?;

    if manifest.platform != platform_suffix() {
        return Err(format!(
            "更新包平台不匹配: 期望 {}, 实际 {}",
            platform_suffix(),
            manifest.platform
        ));
    }
    // bundle 版本必须高于当前 daemon（防降级 / 错误包）
    if !version_compare(env!("CARGO_PKG_VERSION"), &manifest.version) {
        return Err(format!(
            "更新包版本不高于当前版本: bundle {}, 当前 {}",
            manifest.version,
            env!("CARGO_PKG_VERSION")
        ));
    }

    // 收集 staged 下全部文件（相对路径）
    let mut actual: HashMap<String, String> = HashMap::new();
    collect_files(staged, staged, "", &mut actual)?;
    // manifest.json 自身不参与校验
    actual.remove("manifest.json");

    if actual.len() != manifest.files.len() {
        return Err(format!(
            "更新包文件数不匹配: manifest {}, 实际 {}",
            manifest.files.len(),
            actual.len()
        ));
    }
    for (rel, expected) in &manifest.files {
        let got = actual.get(rel).ok_or_else(|| format!("更新包缺少文件: {rel}"))?;
        if got != expected {
            return Err(format!("文件校验失败: {rel}"));
        }
    }
    Ok(())
}

fn collect_files(
    root: &Path,
    dir: &Path,
    prefix: &str,
    out: &mut HashMap<String, String>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let rel = if prefix.is_empty() {
            entry.file_name().to_string_lossy().into_owned()
        } else {
            format!("{prefix}/{}", entry.file_name().to_string_lossy())
        };
        if path.is_dir() {
            collect_files(root, &path, &rel, out)?;
        } else {
            let hash = sha256_hex(&path)?;
            out.insert(rel, hash);
        }
    }
    Ok(())
}

fn sha256_hex(path: &Path) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).map_err(|e| format!("读取文件失败: {e}"))?;
    Ok(hex::encode(Sha256::digest(&data)))
}
