//! 鉴权中间件（骨架）。
//!
//! fnOS 目标形态（Q5 决议）：统一网关优先——网关终结 TLS 并校验 NAS 登录态，
//! 把用户身份以 Header 转发给本服务，daemon 只监听 127.0.0.1 不对外暴露。
//! 本模块预留两种模式：
//! - Gateway：校验网关转发的用户身份 Header（B3 接入统一网关后启用）
//! - Token：`X-Auth-Token` 校验（开发/局域网调试用，默认关闭）

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

/// 网关转发用户身份的标准 Header 名（fnOS 网关约定，B3 实测确认）。
const GATEWAY_USER_HEADER: &str = "x-trim-user";
/// Token 模式校验头。
const TOKEN_HEADER: &str = "x-auth-token";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    /// 无鉴权（仅回环监听 + 开发模式；对外暴露时必须改为 Gateway/Token）
    None,
    /// 校验固定 token（从环境变量 DAEMON_TOKEN 读取）
    Token,
    /// 校验网关用户 Header（fnOS 统一网关，B3 启用）
    Gateway,
}

pub fn load_mode() -> AuthMode {
    if std::env::var("DAEMON_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        return AuthMode::Token;
    }
    if std::env::var("TRIM_GATEWAY")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false)
    {
        return AuthMode::Gateway;
    }
    AuthMode::None
}

/// 鉴权中间件：Token 模式校验请求头，Gateway 模式校验用户 Header，
/// None 模式放行（仅限回环监听）。
pub async fn require_auth(req: Request, next: Next) -> Result<Response, StatusCode> {
    let mode = load_mode();
    match mode {
        AuthMode::None => Ok(next.run(req).await),
        AuthMode::Token => {
            let expected = std::env::var("DAEMON_TOKEN").unwrap_or_default();
            let provided = req
                .headers()
                .get(TOKEN_HEADER)
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default();
            if provided == expected && !expected.is_empty() {
                Ok(next.run(req).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        AuthMode::Gateway => {
            let has_user = req
                .headers()
                .get(GATEWAY_USER_HEADER)
                .and_then(|v| v.to_str().ok())
                .map(|v| !v.trim().is_empty())
                .unwrap_or(false);
            if has_user {
                Ok(next.run(req).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
    }
}