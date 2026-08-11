//! ChmlFrp fnOS 守护进程入口。
//!
//! 独立于桌面版（Tauri）的 axum 服务：提供与桌面版 45 个 tauri::command
//! 同语义的 /api/invoke 透传路由，承载 frpc 隧道管理、守护与下载能力。
//! 零 tauri / WebKit 依赖，默认仅监听回环地址（统一网关在 fnOS 侧转发）。

mod auth;
mod config;
mod invoke;

use axum::{routing::get, Router};
use std::sync::Arc;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<config::DaemonConfig>,
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

    let state = AppState {
        cfg: Arc::new(cfg.clone()),
    };
    let listen_addr = cfg.listen_addr;

    let app = Router::new()
        .route("/api/bootstrap", get(invoke::bootstrap))
        .route("/api/invoke", axum::routing::post(invoke::handle_invoke))
        .layer(axum::middleware::from_fn(auth::require_auth))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .expect("绑定监听地址失败");

    axum::serve(listener, app).await.expect("HTTP 服务异常退出");
}
