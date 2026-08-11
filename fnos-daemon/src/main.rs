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

use axum::routing::{get, post};
use axum::Router;
use custom::CustomManager;
use events::Event;
use frpc::FrpcManager;
use guard::GuardState;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<config::DaemonConfig>,
    pub frpc: Arc<FrpcManager>,
    pub custom: Arc<CustomManager>,
    pub guard: Arc<GuardState>,
    pub events: broadcast::Sender<Event>,
    pub update: update::UpdateChecker,
    /// 自更新 / 终止信号广播（触发 serve 优雅退出）
    pub shutdown: broadcast::Sender<()>,
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
        tokio::net::TcpListener::bind(addr).await
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
    let frpc = Arc::new(FrpcManager::new(cfg.data_dir.clone(), event_tx.clone(), guard.clone()));
    let custom = Arc::new(CustomManager::new(frpc.clone()));

    // 恢复仍在运行的隧道进程（仅记录与日志，守护接管见 guard.rs）
    let recovered = frpc.persistence.recover_running_tunnels();
    if !recovered.is_empty() {
        info!("发现 {} 个仍在运行的隧道进程", recovered.len());
    }

    // 守护监控（3s 轮询 + 日志模式停止）
    guard::start_guard_monitor(guard.clone(), frpc.clone(), custom.clone(), event_tx.clone());

    let mut shutdown_rx = shutdown_tx.subscribe();
    let state = AppState {
        cfg: Arc::new(cfg.clone()),
        frpc,
        custom,
        guard,
        events: event_tx,
        update: update::UpdateChecker::default(),
        shutdown: shutdown_tx,
    };
    let listen_addr = cfg.listen_addr;

    let app = Router::new()
        .route("/api/bootstrap", get(invoke::bootstrap))
        .route("/api/invoke", post(invoke::handle_invoke))
        .route("/ws/logs", get(ws::ws_logs))
        .route("/api/update/check", get(update::handle_check))
        .route("/api/update/download", post(update::handle_download))
        .route("/api/update/apply", post(update::handle_apply))
        .layer(axum::middleware::from_fn(auth::require_auth))
        .with_state(state)
        // SPA 静态托管：未知路径回退 index.html（前端路由）
        .fallback_service(tower_http::services::ServeDir::new(&cfg.web_dir).not_found_service(
            tower_http::services::ServeFile::new(cfg.web_dir.join("index.html")),
        ));

    let listener = build_listener(listen_addr).expect("绑定监听地址失败");

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

    let app_state = app;
    let shutdown_task = async move {
        // 等待广播关闭信号
        let _ = shutdown_rx.recv().await;
    };

    axum::serve(listener, app_state)
        .with_graceful_shutdown(shutdown_task)
        .await
        .expect("HTTP 服务异常退出");

    signal_task.abort();
    info!("chmlfrp-daemon 已退出");
}