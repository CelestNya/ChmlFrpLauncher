//! ChmlFrp fnOS 守护进程入口。
//!
//! 独立于桌面版（Tauri）的 axum 服务：提供与桌面版 45 个 tauri::command
//! 同语义的 /api/invoke 透传路由，承载 frpc 隧道管理、守护与下载能力。
//! 零 tauri / WebKit 依赖，默认仅监听回环地址（统一网关在 fnOS 侧转发）。

mod auth;
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
use tokio::sync::broadcast;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<config::DaemonConfig>,
    pub frpc: Arc<FrpcManager>,
    pub custom: Arc<CustomManager>,
    pub guard: Arc<GuardState>,
    pub events: broadcast::Sender<Event>,
    /// frpc 日志环形缓冲（WS 断线重连补发）
    pub log_history: LogHistory,
    pub update: update::UpdateChecker,
    /// 自更新 / 终止信号广播（触发 serve 优雅退出）
    pub shutdown: broadcast::Sender<()>,
}

/// SPA fallback：真实文件直接返回，其余（含根路径 /）回退 index.html。
/// （nest 前缀下 ServeDir 的 not_found_service 与显式 / 路由均不可靠，统一走 fallback）
async fn spa_fallback(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let path = req.uri().path().trim_start_matches('/');
    serve_static(&state.cfg.web_dir, path).await
}

/// 从 web_dir 读取文件；不存在或为目录时回退 index.html。
async fn serve_static(web_dir: &Path, rel: &str) -> Response {
    let rel = rel.trim_start_matches('/');
    let candidate = if rel.is_empty() {
        web_dir.join("index.html")
    } else {
        web_dir.join(rel)
    };
    let path = if candidate.is_file() {
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

    // 恢复仍在运行的隧道进程（仅记录与日志，守护接管见 guard.rs）
    let recovered = frpc.persistence.recover_running_tunnels();
    if !recovered.is_empty() {
        info!("发现 {} 个仍在运行的隧道进程", recovered.len());
    }

    // 守护监控（3s 轮询 + 日志模式停止）
    guard::start_guard_monitor(guard.clone(), frpc.clone(), custom.clone(), event_tx.clone());

    let mut shutdown_rx_tcp = shutdown_tx.subscribe();
    #[cfg(unix)]
    let mut shutdown_rx_socket = shutdown_tx.subscribe();
    let state = AppState {
        cfg: Arc::new(cfg.clone()),
        frpc,
        custom,
        guard,
        events: event_tx,
        log_history,
        update: update::UpdateChecker::default(),
        shutdown: shutdown_tx,
    };
    let listen_addr = cfg.listen_addr;

    let inner_app = Router::new()
        .route("/api/bootstrap", get(invoke::bootstrap))
        .route("/api/invoke", post(invoke::handle_invoke))
        .route("/ws/logs", get(ws::ws_logs))
        .route("/api/update/check", get(update::handle_check))
        .route("/api/update/download", post(update::handle_download))
        .route("/api/update/apply", post(update::handle_apply))
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
            // 清理旧 socket 文件（重启衔接）
            let _ = std::fs::remove_file(&sock_path);
            let listener =
                tokio::net::UnixListener::bind(&sock_path).expect("绑定网关 socket 失败");
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