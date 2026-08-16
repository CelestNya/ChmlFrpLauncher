//! 鉴权中间件（骨架）。
//!
//! fnOS 目标形态（Q5 决议）：统一网关优先——网关终结 TLS 并校验 NAS 登录态，
//! 把用户身份以 Header 转发给本服务，daemon 只监听 127.0.0.1 不对外暴露。
//! 本模块预留两种模式：
//! - Gateway：校验网关转发的用户身份 Header（B3 接入统一网关后启用）
//! - Token：`X-Auth-Token` 校验（开发/局域网调试用，默认关闭）
//!
//! ⚠️ 真机实测（2026-08-13，PID 69661）：fnOS 应用商店部署时**不注入**
//! TRIM_GATEWAY / DAEMON_TOKEN 环境变量 → 生产实际运行在 None 模式。
//! 当前安全模型 = socket 0600（bind_gateway_socket）+ 仅回环监听 + 网关独占访问；
//! Gateway 模式是未启用的预留路径，接入统一网关前不要依赖它。

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;

/// 网关转发用户身份的标准 Header 名（fnOS 网关约定；真机实测未注入，见模块注释）。
const GATEWAY_USER_HEADER: &str = "x-trim-user";
/// Token 模式校验头。
const TOKEN_HEADER: &str = "x-auth-token";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    /// 无鉴权（仅回环监听 + 开发模式；对外暴露时必须改为 Gateway/Token）
    None,
    /// 校验固定 token（从环境变量 DAEMON_TOKEN 读取）
    Token,
    /// 校验网关用户 Header（fnOS 统一网关；预留路径，真机未注入 TRIM_GATEWAY）
    Gateway,
}

pub fn load_mode() -> AuthMode {
    let token_set = std::env::var("DAEMON_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let gateway_set = std::env::var("TRIM_GATEWAY")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);
    load_mode_from(token_set, gateway_set)
}

/// 纯函数：从环境标志推导鉴权模式（便于测试，不读环境变量）。
fn load_mode_from(token_set: bool, gateway_set: bool) -> AuthMode {
    if token_set {
        AuthMode::Token
    } else if gateway_set {
        AuthMode::Gateway
    } else {
        AuthMode::None
    }
}

/// 恒定时间字符串比较（防时序侧信道；长度不同时按最长遍历补齐）。
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (x, y) = (a.as_bytes(), b.as_bytes());
    let mut diff = (x.len() != y.len()) as u8;
    for i in 0..x.len().max(y.len()) {
        let xb = x.get(i).copied().unwrap_or(0);
        let yb = y.get(i).copied().unwrap_or(0);
        diff |= xb ^ yb;
    }
    diff == 0
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
            if !expected.is_empty() && constant_time_eq(provided, &expected) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 模式优先级_token优先于gateway() {
        assert_eq!(load_mode_from(true, true), AuthMode::Token);
        assert_eq!(load_mode_from(true, false), AuthMode::Token);
        assert_eq!(load_mode_from(false, true), AuthMode::Gateway);
        assert_eq!(load_mode_from(false, false), AuthMode::None);
    }

    #[test]
    fn 恒时比较_等值与不等值() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc123", "abc1234")); // 长度不同
        assert!(!constant_time_eq("abc123", "")); // 空 expected 不匹配任何非空
        assert!(constant_time_eq("", "")); // 双方皆空（上层会用 !expected.is_empty() 拦截放行）
    }
}