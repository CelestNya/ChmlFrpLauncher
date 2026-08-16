//! 自更新（B5）：GitHub Releases 源 → 下载更新 bundle → sha256 校验 → 原子替换 → 重启。
//!
//! 更新源：`https://api.github.com/repos/CelestNya/ChmlFrpLauncher/releases?per_page=100`（按时间倒序）
//! 发布命名空间隔离（2026-08-16）：发版 tag 统一 `fnos-<联合号>`（如 fnos-0.7.5-1.5.2），
//! 与上游 fork 带来的 `v*` tags 不相交。更新检查只认 `fnos-` 前缀的 release——
//! 桌面 release、误 push 上游 tag 生成的 release 一律跳过（详见 fetch_latest_from）。
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

/// fnOS release 列表（GitHub API 按创建时间倒序）。
/// 不用 `releases/latest`：latest 是 fork 内所有正式 release 共用的指针，
/// 任何其他 release（桌面版、误 push 上游 tag 触发生成）都会顶掉它，导致更新通道失效。
const GITHUB_API_RELEASES: &str =
    "https://api.github.com/repos/CelestNya/ChmlFrpLauncher/releases?per_page=100";
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

/// 查询最新 fnOS release 并匹配本平台 bundle。
/// `UPDATE_API_URL` 环境变量可覆盖 GitHub API 地址（测试 / 自托管源 / 规避限流）。
async fn fetch_latest(checker: &UpdateChecker, force: bool) -> Result<UpdateInfo, String> {
    let api_url = std::env::var("UPDATE_API_URL")
        .unwrap_or_else(|_| GITHUB_API_RELEASES.to_string());
    fetch_latest_from(&api_url, checker, force).await
}

/// 核心检查：release 列表 → 只认 `fnos-` 前缀 → 匹配本平台 bundle。
/// 拦截策略：桌面 release / 误 push 上游 tag 生成的 release（tag 为 `v*`）不含 fnOS bundle，
/// 读 latest 会让它们抢占更新通道（静默失效）；改读列表并跳过非 `fnos-` release，
/// 找不到任何 fnOS release 时返回「无更新」而非报错。
async fn fetch_latest_from(
    api_url: &str,
    checker: &UpdateChecker,
    force: bool,
) -> Result<UpdateInfo, String> {
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
    let resp = client
        .get(api_url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("请求更新源失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GitHub API 返回 {}", resp.status()));
    }
    let releases: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| format!("解析 GitHub 响应失败: {e}"))?;
    if releases.is_empty() {
        return Err("GitHub API 返回空 release 列表".to_string());
    }

    let platform = platform_suffix();
    let current = env!("CARGO_PKG_VERSION");

    // 列表按创建时间倒序 → 第一个含本平台 bundle 的 fnOS release 即最新可用更新。
    // 无 bundle 的 fnOS release（半成品发版）跳过，继续找下一个，不让它挡更新通道。
    let mut matched: Option<(String, String, u64)> = None; // (version, url, size)
    for release in &releases {
        let tag = release["tag_name"].as_str().unwrap_or_default();
        let Some(version) = tag.strip_prefix("fnos-") else {
            continue; // 非 fnOS release（桌面版 / 误触发）→ 跳过
        };
        let version = version.trim_start_matches('v').to_string();
        if let Some(assets) = release["assets"].as_array() {
            for asset in assets {
                let name = asset["name"].as_str().unwrap_or_default();
                let target = format!("{BUNDLE_PREFIX}{version}-{platform}.tar.gz");
                if name == target {
                    matched = Some((
                        version.clone(),
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
        if matched.is_some() {
            break;
        }
    }

    let info = match matched {
        Some((version, url, size)) => UpdateInfo {
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

    // 1. 备份并替换 daemon 二进制（内部失败自动回滚）
    let daemon_dst = target.join("chmlfrp-daemon");
    backup_and_replace(&staged.join("chmlfrp-daemon"), &daemon_dst)?;

    // 2. 备份并替换 dist/ 整目录（失败时整体回滚 daemon，避免半更新状态）
    if let Err(e) = replace_dir_atomic(&staged.join("dist"), &target.join("dist")) {
        let daemon_bak = daemon_dst.with_extension("bak");
        if daemon_bak.exists() {
            let _ = std::fs::rename(&daemon_bak, &daemon_dst);
        }
        return Err(e);
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

/// 备份旧文件并原子替换（daemon 中-6 修复）。
///
/// 原实现 `copy(src, dst)` 直写目标：中断即留半写二进制，下次启动即死且
/// 启动脚本不回退 .bak。现改为：旧文件改名 bak → copy 到同目录临时文件 →
/// rename 原子落位；任一步失败恢复 bak。
fn backup_and_replace(src: &Path, dst: &Path) -> Result<(), String> {
    let bak = dst.with_extension("bak");
    let tmp = dst.with_extension("tmp");
    if dst.exists() {
        std::fs::rename(dst, &bak).map_err(|e| format!("备份 {} 失败: {e}", dst.display()))?;
    }
    if let Err(e) = std::fs::copy(src, &tmp) {
        if bak.exists() {
            let _ = std::fs::rename(&bak, dst); // 回滚
        }
        return Err(format!("替换 {} 失败: {e}", dst.display()));
    }
    if let Err(e) = std::fs::rename(&tmp, dst) {
        if bak.exists() {
            let _ = std::fs::rename(&bak, dst); // 回滚
        }
        return Err(format!("替换 {} 失败: {e}", dst.display()));
    }
    Ok(())
}

/// 整目录原子替换：旧目录改名 bak → staged 目录 rename 落位；失败回滚。
fn replace_dir_atomic(staged_dir: &Path, dst_dir: &Path) -> Result<(), String> {
    let bak = dst_dir.with_extension("bak");
    if bak.exists() {
        let _ = std::fs::remove_dir_all(&bak);
    }
    if dst_dir.exists() {
        std::fs::rename(dst_dir, &bak)
            .map_err(|e| format!("备份 {} 失败: {e}", dst_dir.display()))?;
    }
    if staged_dir.exists() {
        if let Err(e) = std::fs::rename(staged_dir, dst_dir) {
            if bak.exists() {
                let _ = std::fs::rename(&bak, dst_dir); // 回滚
            }
            return Err(format!("替换 {} 失败: {e}", dst_dir.display()));
        }
    }
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

    // daemon 中-6：探测新进程存活——二进制损坏/启动即崩时不应继续关停旧进程
    //（否则双 daemon 都没了）。给新进程 300ms 启动窗口后再探测。
    #[cfg(unix)]
    {
        std::thread::sleep(std::time::Duration::from_millis(300));
        if unsafe { libc::kill(pid as i32, 0) } != 0 {
            return Err(format!("新 daemon (PID {pid}) 启动后立即退出，已取消重启"));
        }
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

    // 布局校验（daemon 中-6）：必须含 daemon 二进制与 dist/ 前端目录，
    // 否则 apply 会把线上 dist 改名 .bak 后无替换，形成半更新
    if !manifest.files.contains_key("chmlfrp-daemon") {
        return Err("更新包缺少 chmlfrp-daemon 二进制".to_string());
    }
    if !manifest.files.keys().any(|k| k.starts_with("dist/")) {
        return Err("更新包缺少 dist/ 前端目录内容".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fnos-update-{}-{}",
            name,
            std::process::id()
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn 版本比较边界() {
        assert!(version_compare("0.7.4", "0.7.5"));
        assert!(version_compare("0.7.5", "0.8.0"));
        assert!(version_compare("v0.7.5", "0.8.0"));
        assert!(version_compare("1.0", "1.0.0"), "前缀相等时更短者视为旧版本");
        assert!(!version_compare("0.7.5", "0.7.5"));
        assert!(!version_compare("0.8.0", "0.7.5"));
        assert!(!version_compare("1.0.0", "1.0"));
    }

    #[test]
    fn 原子替换_成功与失败回滚() {
        // daemon 中-6：copy 直写目标 → 中断即半写二进制；改临时文件 + rename
        let dir = temp_dir("replace");
        let src = dir.join("src");
        let dst = dir.join("dst");
        std::fs::write(&src, "NEW").unwrap();
        std::fs::write(&dst, "OLD").unwrap();
        backup_and_replace(&src, &dst).unwrap();
        assert_eq!(std::fs::read_to_string(&dst).unwrap(), "NEW");
        assert!(dst.with_extension("bak").exists(), "旧文件应保留为 bak");

        // copy 失败（src 不存在）→ 回滚，dst 保持原内容
        let src2 = dir.join("missing");
        let dst2 = dir.join("dst2");
        std::fs::write(&dst2, "KEEP").unwrap();
        assert!(backup_and_replace(&src2, &dst2).is_err());
        assert_eq!(
            std::fs::read_to_string(&dst2).unwrap(),
            "KEEP",
            "失败后应回滚原文件"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn 目录原子替换() {
        let dir = temp_dir("dir-replace");
        let staged = dir.join("staged-dist");
        let dst = dir.join("dist");
        std::fs::create_dir_all(staged.join("assets")).unwrap();
        std::fs::write(staged.join("assets/a.js"), "new-js").unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(dst.join("old.js"), "old-js").unwrap();

        replace_dir_atomic(&staged, &dst).unwrap();
        assert!(dst.join("assets/a.js").exists(), "新 dist 未落位");
        assert!(!dst.join("old.js").exists(), "旧文件应被替换");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bundle布局校验_缺关键内容拒绝() {
        let dir = temp_dir("verify");
        let daemon = dir.join("chmlfrp-daemon");
        let dist_file = dir.join("dist/index.html");
        std::fs::create_dir_all(dir.join("dist")).unwrap();
        std::fs::write(&daemon, "bin").unwrap();
        std::fs::write(&dist_file, "html").unwrap();
        let full_manifest = serde_json::json!({
            "version": "9.9.9",
            "platform": platform_suffix(),
            "files": {
                "chmlfrp-daemon": sha256_hex(&daemon).unwrap(),
                "dist/index.html": sha256_hex(&dist_file).unwrap(),
            }
        });
        std::fs::write(dir.join("manifest.json"), full_manifest.to_string()).unwrap();
        // 完整 → 通过（版本 9.9.9 > 当前）
        assert!(verify_bundle(&dir).is_ok());

        // 缺 daemon 条目 → 拒绝
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::json!({
                "version": "9.9.9",
                "platform": platform_suffix(),
                "files": { "dist/index.html": sha256_hex(&dist_file).unwrap() }
            })
            .to_string(),
        )
        .unwrap();
        assert!(verify_bundle(&dir).is_err(), "缺 chmlfrp-daemon 应拒绝");

        // 缺 dist/ 条目 → 拒绝（daemon 中-6：否则 apply 把线上 dist 改成 .bak 后无替换）
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::json!({
                "version": "9.9.9",
                "platform": platform_suffix(),
                "files": { "chmlfrp-daemon": sha256_hex(&daemon).unwrap() }
            })
            .to_string(),
        )
        .unwrap();
        assert!(verify_bundle(&dir).is_err(), "缺 dist/ 应拒绝");
        let _ = std::fs::remove_dir_all(&dir);
    }

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// 极简 mock GitHub API：一次连接返回一个固定 JSON 响应（releases 列表）。
    async fn mock_api(body: String) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            }
        });
        (format!("http://{addr}"), handle)
    }

    fn fnos_release(tag: &str, asset_names: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "assets": asset_names.iter().map(|name| serde_json::json!({
                "name": name,
                "browser_download_url": format!("https://download.example/{}", name),
                "size": 12345,
            })).collect::<Vec<_>>(),
        })
    }

    /// 拦截策略（2026-08-16）：桌面 release / 误 push 上游 tag 生成的 release
    /// （tag 为 `v*`）不得抢占 fnOS 更新通道——必须跳过并找到最新的 `fnos-` release。
    #[tokio::test]
    async fn 更新检查_跳过桌面release取最新fnos() {
        let target = format!("chmlfrp-fnos-9.9.9-{}.tar.gz", platform_suffix());
        let body = serde_json::json!([
            // 桌面 release（无 fnOS bundle）→ 必须跳过
            { "tag_name": "v0.7.5", "assets": [] },
            fnos_release("fnos-9.9.9", &[&target]),
        ])
        .to_string();
        let (url, server) = mock_api(body).await;
        let info = fetch_latest_from(&url, &UpdateChecker::default(), true)
            .await
            .unwrap();
        server.abort();
        assert!(info.available, "应跳过桌面 release 找到 fnOS 更新");
        assert_eq!(info.version.as_deref(), Some("9.9.9"));
        let expected_url = format!("https://download.example/{target}");
        assert_eq!(info.url.as_deref(), Some(expected_url.as_str()));
    }

    #[tokio::test]
    async fn 更新检查_最新fnos无bundle则找下一个() {
        let target = format!("chmlfrp-fnos-10.0.0-{}.tar.gz", platform_suffix());
        let body = serde_json::json!([
            // 最新 fnOS release 缺本平台 bundle（半成品发版）→ 跳过，取下一个
            fnos_release("fnos-9.9.9", &[]),
            fnos_release("fnos-10.0.0", &[&target]),
        ])
        .to_string();
        let (url, server) = mock_api(body).await;
        let info = fetch_latest_from(&url, &UpdateChecker::default(), true)
            .await
            .unwrap();
        server.abort();
        assert_eq!(info.version.as_deref(), Some("10.0.0"), "应跳过缺 bundle 的 release");
        assert!(info.available);
    }

    #[tokio::test]
    async fn 更新检查_只有桌面release视为无更新不报错() {
        let body = serde_json::json!([
            { "tag_name": "v0.7.5", "assets": [] },
            { "tag_name": "v0.8.0", "assets": [] },
        ])
        .to_string();
        let (url, server) = mock_api(body).await;
        let info = fetch_latest_from(&url, &UpdateChecker::default(), true)
            .await
            .unwrap();
        server.abort();
        assert!(!info.available, "无 fnOS release 应返回无更新而非报错");
        assert!(info.version.is_none());
    }

    #[tokio::test]
    async fn 更新检查_平台bundle不匹配视为无更新() {
        // 只有 arm 平台的 bundle，而本平台是 x86（或反之）→ 无匹配 → 无更新
        let wrong = if platform_suffix() == "x86" { "arm" } else { "x86" };
        let asset = format!("chmlfrp-fnos-9.9.9-{wrong}.tar.gz");
        let body = serde_json::json!([fnos_release("fnos-9.9.9", &[&asset])]).to_string();
        let (url, server) = mock_api(body).await;
        let info = fetch_latest_from(&url, &UpdateChecker::default(), true)
            .await
            .unwrap();
        server.abort();
        assert!(!info.available, "平台不匹配应视为无更新");
    }

    #[tokio::test]
    async fn 更新检查_空列表视为异常() {
        let (url, server) = mock_api("[]".to_string()).await;
        let result = fetch_latest_from(&url, &UpdateChecker::default(), true).await;
        server.abort();
        assert!(result.is_err(), "空 release 列表应报错（正常状态必有 release）");
    }
}
