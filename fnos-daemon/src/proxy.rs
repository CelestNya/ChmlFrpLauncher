//! ChmlFrp API 代理（http_request / http_request_raw）。
//!
//! 规格源：src-tauri/src/commands/http.rs（URL 白名单与 https 强制校验为安全线，
//! 保持不简化）。

use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Deserialize)]
pub struct HttpRequestOptions {
    pub url: String,
    pub method: String,
    pub headers: Option<HashMap<String, String>>,
    pub body: Option<String>,
    pub bypass_proxy: Option<bool>,
}

#[derive(Serialize)]
pub struct HttpResponsePayload {
    pub status: u16,
    pub body: String,
}

/// 目标 IP 属于私网/回环/链路本地/组播等受限网段（SSRF 防线，daemon 中-10）：
/// 白名单域名的子域持有者可把域名解析到内网，必须对解析结果做二次校验。
fn is_forbidden_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (o[0] == 100 && o[1] & 0b1100_0000 == 0b0100_0000) // 100.64.0.0/10 CGNAT
                || v4.is_multicast()
        }
        std::net::IpAddr::V6(v6) => {
            let s = v6.segments();
            v6.is_unspecified()
                || v6.is_loopback()
                || (s[0] & 0xfe00 == 0xfc00) // fc00::/7 唯一本地
                || (s[0] & 0xffc0 == 0xfe80) // fe80::/10 链路本地
                || v6.is_multicast()
        }
    }
}

/// DNS 解析结果为空或含任一受限网段即拒绝。
fn any_forbidden(addrs: &[std::net::IpAddr]) -> bool {
    addrs.is_empty() || addrs.iter().any(|ip| is_forbidden_ip(*ip))
}

fn validate_request_url(raw_url: &str) -> Result<Url, String> {
    let url = Url::parse(raw_url).map_err(|e| format!("Invalid URL: {}", e))?;
    if url.scheme() != "https" {
        return Err("仅允许 https 请求".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL 不允许包含凭据".to_string());
    }
    let host = url.host_str().ok_or_else(|| "URL 缺少 host".to_string())?;
    if !is_allowed_host(host) {
        return Err("URL 不在允许列表".to_string());
    }
    // IP 字面量兜底（正常进不了白名单，防御纵深）
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_forbidden_ip(ip) {
            return Err("目标地址为受限网段".to_string());
        }
    }
    Ok(url)
}

/// ChmlFrp 服务域名白名单（安全线，保持与桌面版一致）。
fn is_allowed_host(host: &str) -> bool {
    host == "cf-v2.uapis.cn"
        || host == "cf-v1.uapis.cn"
        || host.ends_with(".uapis.cn")
        || host == "account-api.qzhua.net"
        || host == "chmlfrp.net"
        || host.ends_with(".chmlfrp.net")
}

async fn send_request(options: HttpRequestOptions) -> Result<HttpResponsePayload, String> {
    let bypass_proxy = options.bypass_proxy.unwrap_or(true);
    let url = validate_request_url(&options.url)?;

    // DNS 解析后拒绝受限网段目标（白名单子域可被解析到内网，daemon 中-10）
    let host = url.host_str().ok_or_else(|| "URL 缺少 host".to_string())?;
    let port = url.port_or_known_default().unwrap_or(443);
    let resolved: Vec<std::net::IpAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| format!("DNS 解析失败: {}", e))?
        .map(|sa| sa.ip())
        .collect();
    if any_forbidden(&resolved) {
        return Err("目标地址解析到受限网段，已拒绝".to_string());
    }

    let mut client_builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("ChmlFrpLauncher/1.0")
        // 不跟随重定向：白名单站点 302 可把请求引到任意地址（绕过上述校验）
        .redirect(reqwest::redirect::Policy::none());

    if bypass_proxy {
        client_builder = client_builder.proxy(reqwest::Proxy::custom(
            move |_url| -> Option<reqwest::Url> { None },
        ));
    }

    let client = client_builder
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))?;

    let mut request = match options.method.as_str() {
        "GET" => client.get(url.clone()),
        "POST" => client.post(url.clone()),
        "PUT" => client.put(url.clone()),
        "DELETE" => client.delete(url.clone()),
        "PATCH" => client.patch(url.clone()),
        _ => return Err(format!("Unsupported method: {}", options.method)),
    };

    if let Some(headers) = options.headers {
        for (key, value) in headers {
            request = request.header(&key, &value);
        }
    }

    if let Some(body) = options.body {
        request = request.body(body);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    Ok(HttpResponsePayload {
        status: status.as_u16(),
        body,
    })
}

pub async fn http_request(options: HttpRequestOptions) -> Result<String, String> {
    let response = send_request(options).await?;

    if !(200..300).contains(&response.status) {
        return Err(format!("HTTP {}: {}", response.status, response.body));
    }

    Ok(response.body)
}

pub async fn http_request_raw(options: HttpRequestOptions) -> Result<HttpResponsePayload, String> {
    send_request(options).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 白名单与协议校验() {
        assert!(validate_request_url("https://cf-v2.uapis.cn/api").is_ok());
        assert!(validate_request_url("https://sub.chmlfrp.net/x").is_ok());
        assert!(validate_request_url("http://cf-v2.uapis.cn").is_err()); // 非 https
        assert!(validate_request_url("https://evil.example.com").is_err()); // 非白名单
        assert!(validate_request_url("https://user:pw@cf-v2.uapis.cn").is_err()); // 凭据
    }

    #[test]
    fn 受限网段ip字面量被拒绝() {
        assert!(validate_request_url("https://127.0.0.1/x").is_err());
        assert!(validate_request_url("https://10.1.2.3/x").is_err());
        assert!(validate_request_url("https://192.168.12.32/x").is_err());
        assert!(validate_request_url("https://169.254.169.254/x").is_err());
        assert!(validate_request_url("https://100.64.0.1/x").is_err());
        assert!(validate_request_url("https://[::1]/x").is_err());
        assert!(validate_request_url("https://[fe80::1]/x").is_err());
    }

    #[test]
    fn 受限网段ip分类() {
        use std::net::IpAddr;
        for ip in [
            "127.0.0.1", "10.0.0.1", "172.16.0.1", "192.168.1.1", "169.254.0.1",
            "100.64.0.1", "0.0.0.0", "224.0.0.1", "::1", "fe80::1", "fc00::1",
        ] {
            assert!(is_forbidden_ip(ip.parse::<IpAddr>().unwrap()), "{ip} 应判为受限");
        }
        for ip in ["1.1.1.1", "114.114.114.114", "2606:4700::1111"] {
            assert!(!is_forbidden_ip(ip.parse::<IpAddr>().unwrap()), "{ip} 不应受限");
        }
    }

    #[test]
    fn 解析结果含受限网段即拒绝() {
        use std::net::IpAddr;
        assert!(any_forbidden(&[]));
        assert!(any_forbidden(&["1.1.1.1".parse::<IpAddr>().unwrap(), "10.0.0.1".parse().unwrap()]));
        assert!(!any_forbidden(&["1.1.1.1".parse::<IpAddr>().unwrap()]));
    }
}