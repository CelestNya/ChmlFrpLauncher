//! ChmlFrp fnOS 守护进程入口。
//!
//! 独立于桌面版（Tauri）的 axum 服务：提供与桌面版 45 个 tauri::command
//! 同语义的 /api/invoke 透传路由，承载 frpc 隧道管理、守护与下载能力。
//! 零 tauri / WebKit 依赖，默认仅监听回环地址（统一网关在 fnOS 侧转发）。

mod auth;
mod config;
mod events;
mod frpc;
mod guard;
mod invoke;
mod persist;
mod ws;

use axum::routing::{get, post};
use axum::Router;
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
    pub guard: Arc<GuardState>,
    pub events: broadcast::Sender<Event>,
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

    let (event_tx, _) = broadcast::channel(1024);
    let guard = GuardState::new(); // D1：守护默认开启
    let frpc = Arc::new(FrpcManager::new(cfg.data_dir.clone(), event_tx.clone(), guard.clone()));

    // 恢复仍在运行的隧道进程（仅记录与日志，守护接管见 guard.rs）
    let recovered = frpc.persistence.recover_running_tunnels();
    if !recovered.is_empty() {
        info!("发现 {} 个仍在运行的隧道进程", recovered.len());
    }

    // 守护监控（3s 轮询 + 日志模式停止）
    guard::start_guard_monitor(guard.clone(), frpc.clone(), event_tx.clone());

    let state = AppState {
        cfg: Arc::new(cfg.clone()),
        frpc,
        guard,
        events: event_tx,
    };
    let listen_addr = cfg.listen_addr;

    let app = Router::new()
        .route("/api/bootstrap", get(invoke::bootstrap))
        .route("/api/invoke", post(invoke::handle_invoke))
        .route("/ws/logs", get(ws::ws_logs))
        .layer(axum::middleware::from_fn(auth::require_auth))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .expect("绑定监听地址失败");

    axum::serve(listener, app).await.expect("HTTP 服务异常退出");
}