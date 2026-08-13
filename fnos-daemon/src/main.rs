//! ChmlFrp fnOS 守护进程入口。
//!
//! 独立于桌面版（Tauri）的 axum 服务：提供与桌面版 45 个 tauri::command
//! 同语义的 /api/invoke 透传路由，承载 frpc 隧道管理、守护与下载能力。
//! 零 tauri / WebKit 依赖，默认仅监听回环地址（统一网关在 fnOS 侧转发）。

mod auth;
mod background;
mod config;
mod custom;
mod download;
mod events;
mod frpc;
mod guard;
mod invoke;
mod net;
mod persist;
mod proxy;
mod settings;
mod update;
mod ws;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use custom::CustomManager;
use events::{Event, LogHistory};
use frpc::FrpcManager;
use guard::GuardState;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use serde_json::json;
use tokio::sync::broadcast;
use tracing::{info, warn};

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<config::DaemonConfig>,
    pub frpc: Arc<FrpcManager>,
    pub custom: Arc<CustomManager>,
    pub guard: Arc<GuardState>,
    /// 凭据与节点设置的后端存储（ADR-0002：token/代理配置离浏览器）
    pub settings: Arc<settings::SettingsStore>,
    pub events: broadcast::Sender<Event>,
    /// frpc 日志环形缓冲（WS 断线重连补发）
    pub log_history: LogHistory,
    pub update: update::UpdateChecker,
    /// 自更新 / 终止信号广播（触发 serve 优雅退出）
    pub shutdown: broadcast::Sender<()>,
}

/// SPA fallback：真实文件直接返回，其余（含根路径 /）回退 index.html。
/// （nest 前缀下 ServeDir 的 not_found_service 与显式 / 路由均不可靠，统一走 fallback）
/// fnOS 专属：index.html 注入 `__FNOS_BOOT__`（credential + nodeSettings，阶段 1c）——
/// shim 启动时同步读入内存缓存，业务代码首帧 getItem(chmlfrp_user) 命中缓存，不闪烁。
async fn spa_fallback(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    // 索引页（/ 或 /index.html）注入 boot blob
    if path.is_empty() || path.ends_with("index.html") {
        if let Some(html) = serve_index_with_boot(&state).await {
            return html;
        }
    }
    serve_static(&state.cfg.web_dir, path).await
}

/// 读取 index.html 并在 `<head>` 注入 `__FNOS_BOOT__`（含 daemon 侧凭据/节点设置）。
/// 返回 None 表示 index.html 不存在（回退 serve_static 的正常处理）。
async fn serve_index_with_boot(state: &AppState) -> Option<Response> {
    let index = state.cfg.web_dir.join("index.html");
    let content = tokio::fs::read_to_string(&index).await.ok()?;
    let boot_json = json!({
        "credential": state.settings.get_credential(),
        "nodeSettings": state.settings.get_node_settings(),
    })
    .to_string();
    let script = format!(
        "<script>window.__FNOS_BOOT__ = {boot_json};</script>"
    );
    // 注入到 <head> 开头（shim 是 body 末尾普通脚本，先于它执行；React 挂载前就绪）
    let injected = if content.contains("</head>") {
        content.replace("</head>", format!("{script}</head>").as_str())
    } else {
        format!("{script}{content}")
    };
    Some(
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(injected))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    )
}

/// 校验并规范化静态资源请求路径（第一层防御：语法拒绝）。
/// 返回 None = 拒绝（路径穿越尝试）；Some(rel) = 规范化后的 web_dir 相对路径。
/// 防御点：
/// - 拒绝明文/百分号编码的 `..` 段（`%2e%2e` 解码后判定）
/// - 拒绝反斜杠（Windows 风格穿越，任何平台都拒绝，防手滑移植）
/// - 拒绝解码后含 `/`、`\`、NUL 的段（编码伪装分隔符）
/// - 折叠空段与 `.` 段
pub fn sanitize_web_path(rel: &str) -> Option<String> {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() {
        return Some(String::new());
    }
    let mut segments = Vec::new();
    for raw in rel.split('/') {
        let seg = percent_decode(raw)?;
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains('/') || seg.contains('\\') || seg.contains('\0') {
            return None;
        }
        segments.push(seg);
    }
    Some(segments.join("/"))
}

/// 解码 URL 百分号编码（%XX）。非法序列（截断/非 hex/非 UTF-8）返回 None。
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hi = hex_val(*b.get(i + 1)?)?;
            let lo = hex_val(*b.get(i + 2)?)?;
            out.push(hi * 16 + lo);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// 第二层防御：canonicalize 后断言候选仍在 web_dir 内（防符号链接等绕过）。
fn within_web_dir(web_dir: &Path, candidate: &Path) -> bool {
    let Ok(canon) = candidate.canonicalize() else {
        return false;
    };
    let Ok(root) = web_dir.canonicalize() else {
        return false;
    };
    canon.starts_with(root)
}

/// 从 web_dir 读取文件；不存在或为目录时回退 index.html。
async fn serve_static(web_dir: &Path, rel: &str) -> Response {
    // 第一层：语法拒绝穿越（明文/编码的 ..、反斜杠、编码分隔符）
    let Some(rel) = sanitize_web_path(rel) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let candidate = if rel.is_empty() {
        web_dir.join("index.html")
    } else {
        web_dir.join(&rel)
    };
    let path = if candidate.is_file() && within_web_dir(web_dir, &candidate) {
        candidate
    } else {
        web_dir.join("index.html")
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mime = mime_for(&path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime)
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// 精简 MIME 推断（前端产物仅 html/js/css/svg/png 等）。
fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "application/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("json") => "application/json",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("mp3") => "audio/mpeg",
        Some("webm") => "video/webm",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
}

/// 构建监听 socket：设置 SO_REUSEPORT，使自更新（B5）spawn 的新进程
/// 能在旧进程释放端口前完成 bind，实现无中断衔接（旧进程随后优雅退出）。
fn build_listener(addr: std::net::SocketAddr) -> std::io::Result<tokio::net::TcpListener> {
    #[cfg(unix)]
    {
        use socket2::{Domain, Socket, Type};
        let domain = if addr.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };
        let socket = Socket::new(domain, Type::STREAM, None)?;
        socket.set_reuse_address(true)?;
        socket.set_reuse_port(true)?;
        socket.bind(&addr.into())?;
        socket.listen(1024)?;
        socket.set_nonblocking(true)?;
        tokio::net::TcpListener::from_std(socket.into())
    }
    #[cfg(not(unix))]
    {
        let socket = std::net::TcpListener::bind(addr)?;
        socket.set_nonblocking(true)?;
        tokio::net::TcpListener::from_std(socket)
    }
}

/// 绑定网关 unix socket，并收紧文件权限为 0600（daemon 高-2 修复）：
/// 默认 umask 下 socket 权限为 0755，NAS 上任何本地用户都能连接并伪造
/// `x-trim-user` 头获得全部 invoke 权限。0600 后仅应用用户（与网关进程）可连。
#[cfg(unix)]
fn bind_gateway_socket(sock_path: &Path) -> std::io::Result<tokio::net::UnixListener> {
    use std::os::unix::fs::PermissionsExt;
    // 清理旧 socket 文件（重启衔接）
    let _ = std::fs::remove_file(sock_path);
    let listener = tokio::net::UnixListener::bind(sock_path)?;
    // 权限收紧失败与 bind 失败同级：静默放行等于安全线失效
    std::fs::set_permissions(sock_path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cfg = config::DaemonConfig::load();
    cfg.ensure_dirs();

    // 鉴权模式声明（daemon 高-2）：None 模式仅限回环 + socket 0600 的可信环境，
    // 显式警告避免静默裸奔。
    // 真机实测（2026-08-13）：商店部署不注入 TRIM_GATEWAY/DAEMON_TOKEN，None 是生产实际模式。
    match auth::load_mode() {
        auth::AuthMode::None => warn!(
            "鉴权模式: None（无鉴权）。仅限回环监听 + 网关 socket 0600 的可信环境；\
             对外暴露前必须设置 DAEMON_TOKEN 或 TRIM_GATEWAY"
        ),
        auth::AuthMode::Gateway => info!("鉴权模式: Gateway（信任网关转发的 x-trim-user 头）"),
        auth::AuthMode::Token => info!("鉴权模式: Token"),
    }

    info!(
        "chmlfrp-daemon v{} 启动，数据目录: {}, 监听: {}",
        env!("CARGO_PKG_VERSION"),
        cfg.data_dir.display(),
        cfg.listen_addr
    );
    info!("前端静态目录: {}", cfg.web_dir.display());

    let (event_tx, _) = broadcast::channel(1024);
    // 自更新关闭信号：apply 后广播，触发 serve 优雅退出（新进程已先于旧进程 bind）
    let (shutdown_tx, _) = broadcast::channel(1);
    let shutdown_tx_for_signal = shutdown_tx.clone();
    let guard = GuardState::new(); // D1：守护默认开启
    // frpc 日志环形缓冲（WS 断线重连补发，容量见 events::LOG_HISTORY_CAP）
    let log_history: LogHistory = Arc::new(Mutex::new(VecDeque::new()));
    let frpc = Arc::new(FrpcManager::new(
        cfg.data_dir.clone(),
        event_tx.clone(),
        log_history.clone(),
        guard.clone(),
    ));
    let custom = Arc::new(CustomManager::new(frpc.clone()));
    let settings = Arc::new(settings::SettingsStore::new(&cfg.data_dir));

    // 恢复仍在运行的隧道进程（仅记录与日志，守护接管见 guard.rs）
    let recovered = frpc.persistence.recover_running_tunnels();
    if !recovered.is_empty() {
        info!("发现 {} 个仍在运行的隧道进程", recovered.len());
        // daemon 中-8：重新登记守护，恢复自动重启覆盖
        let n = guard::re_register_guarded(&guard, &recovered);
        info!("已为 {n} 个存量隧道重新注册守护");
    }

    // 守护监控（3s 轮询 + 日志模式停止）
    let guard_ops =
        Arc::new(guard::GuardOpsImpl { frpc: frpc.clone(), custom: custom.clone() });
    guard::start_guard_monitor(guard.clone(), guard_ops, event_tx.clone());

    let mut shutdown_rx_tcp = shutdown_tx.subscribe();
    #[cfg(unix)]
    let mut shutdown_rx_socket = shutdown_tx.subscribe();
    let state = AppState {
        cfg: Arc::new(cfg.clone()),
        frpc,
        custom,
        guard,
        settings,
        events: event_tx,
        log_history,
        update: update::UpdateChecker::default(),
        shutdown: shutdown_tx,
    };
    let listen_addr = cfg.listen_addr;

    let inner_app = Router::new()
        .route("/api/bootstrap", get(invoke::bootstrap))
        // ADR-0001：能力协商端点——前端据此判断 daemon 支持哪些命令
        .route(
            "/api/capabilities",
            get(|| async {
                axum::Json(serde_json::json!({
                    "commands": invoke::SUPPORTED_COMMANDS,
                }))
            }),
        )
        .route("/api/invoke", post(invoke::handle_invoke))
        .route("/ws/logs", get(ws::ws_logs))
        .route("/api/update/check", get(update::handle_check))
        .route("/api/update/download", post(update::handle_download))
        .route("/api/update/apply", post(update::handle_apply))
        // ADR-0004：壁纸文件经 daemon 静态托管（data_dir/backgrounds，只读）——
        // fnOS 前端用相对 URL 渲染壁纸，替代桌面 app:// 协议
        .nest_service(
            "/assets/backgrounds",
            tower_http::services::ServeDir::new(cfg.data_dir.join("backgrounds"))
                .precompressed_gzip(),
        )
        .fallback(spa_fallback)
        .layer(axum::middleware::from_fn(auth::require_auth))
        .with_state(state);

    // fnOS 统一网关：桌面 iframe 经 gatewayPrefix（/app/chmlfrp）→ unix socket 转发。
    // TCP 与 socket 均挂在同一前缀下，保证前端资源（vite base=/app/chmlfrp/）
    // 与 shim 的 API/WS 路径在两种访问方式下一致。
    let gateway_app = Router::new().nest("/app/chmlfrp", inner_app);

    // 优雅关闭：SIGTERM / SIGINT（fnOS cmd/main stop）或自更新 apply
    let signal_task = tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        let sigterm = async {
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("注册 SIGTERM 失败")
                .recv()
                .await;
        };
        #[cfg(not(unix))]
        let sigterm = std::future::pending::<()>();

        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm => {}
        }
        info!("收到终止信号，优雅退出");
        let _ = shutdown_tx_for_signal.send(());
    });

    let shutdown_task_tcp = async move {
        let _ = shutdown_rx_tcp.recv().await;
    };
    #[cfg(unix)]
    let shutdown_task_socket = async move {
        let _ = shutdown_rx_socket.recv().await;
    };

    // TCP 监听（fnOS 下仅回环；统一网关走 socket，TCP 供本地调试/健康检查）
    let listener = build_listener(listen_addr).expect("绑定监听地址失败");
    let tcp_serve = axum::serve(listener, gateway_app.clone())
        .with_graceful_shutdown(shutdown_task_tcp);

    // 统一网关 unix socket（TRIM_APPDEST/app.sock）——仅 unix 平台存在
    #[cfg(unix)]
    let socket_task = async {
        if let Some(sock_path) = cfg.gateway_socket_path() {
            let listener =
                bind_gateway_socket(&sock_path).expect("绑定网关 socket 失败（含 0600 权限收紧）");
            info!("网关 socket 监听: {}", sock_path.display());
            axum::serve(listener, gateway_app)
                .with_graceful_shutdown(shutdown_task_socket)
                .await
                .expect("网关 socket 服务异常退出");
        } else {
            info!("未设置 TRIM_APPDEST，跳过网关 socket 监听");
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let socket_task = async {
        info!("非 unix 平台，跳过网关 socket 监听");
        std::future::pending::<()>().await;
    };

    tokio::select! {
        res = tcp_serve => res.expect("TCP 服务异常退出"),
        _ = socket_task => {}
    }

    signal_task.abort();
    info!("chmlfrp-daemon 已退出");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use std::fs;

    #[test]
    fn 路径穿越拒绝_明文点点点() {
        assert_eq!(sanitize_web_path("../data/g_44.ini"), None);
        assert_eq!(sanitize_web_path("/../data/g_44.ini"), None);
        assert_eq!(sanitize_web_path("a/../../etc/passwd"), None);
    }

    #[test]
    fn 路径穿越拒绝_百分号编码() {
        assert_eq!(sanitize_web_path("%2e%2e/data/g_44.ini"), None);
        assert_eq!(sanitize_web_path("%2E%2E%2Fdata"), None);
        assert_eq!(sanitize_web_path("..%2fdata"), None);
        assert_eq!(sanitize_web_path("a%2fb%2f..%2fc"), None);
    }

    #[test]
    fn 路径穿越拒绝_反斜杠() {
        assert_eq!(sanitize_web_path("a\\..\\b"), None);
        assert_eq!(sanitize_web_path("a%5c..%5cb"), None);
    }

    #[test]
    fn 正常路径放行并规范化() {
        assert_eq!(
            sanitize_web_path("assets/index-abc.js"),
            Some("assets/index-abc.js".to_string())
        );
        assert_eq!(
            sanitize_web_path("./assets/x.js"),
            Some("assets/x.js".to_string())
        );
        assert_eq!(sanitize_web_path("index.html"), Some("index.html".to_string()));
        // 非穿越段（..b）不受影响
        assert_eq!(sanitize_web_path("a/..b/c"), Some("a/..b/c".to_string()));
        assert_eq!(sanitize_web_path(""), Some(String::new()));
        assert_eq!(sanitize_web_path("/"), Some(String::new()));
    }

    #[tokio::test]
    async fn serve_static_拒绝穿越并正常回退() {
        let web_dir = std::env::temp_dir().join(format!("fnos-web-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("fnos-outside-{}", std::process::id()));
        fs::create_dir_all(&web_dir).unwrap();
        fs::write(web_dir.join("index.html"), "<html>ok</html>").unwrap();
        fs::write(web_dir.join("secret.txt"), "INNER").unwrap();
        fs::write(&outside, "OUTER-SECRET").unwrap();

        // 明文穿越：web_dir/../fnos-outside-<pid> 触达外部文件 → 404
        let rel = format!("../{}", outside.file_name().unwrap().to_string_lossy());
        let resp = serve_static(&web_dir, &rel).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // 编码穿越 → 404
        let rel = format!(
            "%2e%2e/{}",
            outside.file_name().unwrap().to_string_lossy()
        );
        let resp = serve_static(&web_dir, &rel).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // 正常文件 → 200 + 内容
        let resp = serve_static(&web_dir, "secret.txt").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"INNER");

        // 空路径 → 回退 index.html
        let resp = serve_static(&web_dir, "").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&bytes[..], b"<html>ok</html>");

        let _ = fs::remove_dir_all(&web_dir);
        let _ = fs::remove_file(&outside);
    }

    /// 网关 socket 权限必须收紧为 0600：仅应用用户可连（网关进程除外），
    /// 否则 NAS 上任何本地用户都能连 socket 伪造网关头（daemon 高-2）。
    #[cfg(unix)]
    #[tokio::test]
    async fn 网关socket权限收紧为0600() {
        use std::os::unix::fs::PermissionsExt;
        let sock_path = std::env::temp_dir().join(format!("fnos-sock-{}", std::process::id()));
        let _ = fs::remove_file(&sock_path);

        let listener = bind_gateway_socket(&sock_path).expect("绑定失败");
        let mode = fs::metadata(&sock_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "socket 权限未收紧");

        drop(listener);
        let _ = fs::remove_file(&sock_path);
    }

    /// index.html 注入 __FNOS_BOOT__：首帧同步读 credential/nodeSettings（阶段 1c）。
    /// daemon 托管 SPA 时业务代码 getItem(chmlfrp_user) 命中 boot 注水缓存，不闪烁。
    #[tokio::test]
    async fn index注入boot_blob() {
        use crate::settings::{Credential, SettingsStore};
        use axum::body::to_bytes;

        let dir = std::env::temp_dir().join(format!("fnos-boot-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("index.html"), "<html><head></head><body></body></html>").unwrap();

        let store = SettingsStore::new(&dir);
        store
            .save_credential(&Credential {
                username: Some("u".to_string()),
                usertoken: None,
                access_token: Some("access".to_string()),
                refresh_token: Some("refresh".to_string()),
                access_token_expires_at: Some(1750000000),
                token_type: Some("Bearer".to_string()),
            })
            .unwrap();

        let state = AppState {
            cfg: Arc::new(config::DaemonConfig {
                data_dir: dir.clone(),
                web_dir: dir.clone(),
                app_dest: None,
                listen_addr: "127.0.0.1:17890".parse().unwrap(),
            }),
            settings: Arc::new(store),
            frpc: Arc::new(crate::frpc::FrpcManager::new(
                dir.clone(),
                tokio::sync::broadcast::channel(8).0,
                Arc::new(std::sync::Mutex::new(VecDeque::new())),
                GuardState::new(),
            )),
            custom: Arc::new(CustomManager::new(Arc::new(
                crate::frpc::FrpcManager::new(
                    dir.clone(),
                    tokio::sync::broadcast::channel(8).0,
                    Arc::new(std::sync::Mutex::new(VecDeque::new())),
                    GuardState::new(),
                ),
            ))),
            guard: GuardState::new(),
            events: tokio::sync::broadcast::channel(8).0,
            log_history: Arc::new(std::sync::Mutex::new(VecDeque::new())),
            update: update::UpdateChecker::default(),
            shutdown: tokio::sync::broadcast::channel(1).0,
        };

        let req = axum::http::Request::builder()
            .uri("/index.html")
            .body(Body::empty())
            .unwrap();
        let resp = spa_fallback(State(state), req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let html = String::from_utf8(bytes.to_vec()).unwrap();

        assert!(html.contains("__FNOS_BOOT__"), "缺少 __FNOS_BOOT__ 注入");
        assert!(
            html.contains("\"access_token\":\"access\""),
            "boot 应含凭据 access_token"
        );
        assert!(
            !html.contains("\"accessToken\":\"access\""),
            "boot 应保持 daemon snake_case（shim 负责转 camelCase）"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}