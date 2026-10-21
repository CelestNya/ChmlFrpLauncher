use reqwest::Url;
use serde::Serialize;

/// 头像 URL 的小尺寸化结果。
///
/// 背景：头像原图通常是 640×640，而界面只显示 40~44px，需要 15 倍以上的降采样。
/// WebKitGTK 的 <img> 在大比例降采样时滤波质量不足，细线条会被稀疏采样成暗色
/// 噪点（实测暗像素比例约 1.9%）；Chromium 的实现更好，所以同样代码在 Windows
/// 上噪点不明显。该问题不受 CSS image-rendering 影响。
///
/// 在 Xvfb 中用真实 WebKitGTK 实测对比：
///   源 640px → 暗像素 1.875%
///   源 100px → 暗像素 1.250%   ← 显著改善
///   源 140px → 暗像素 1.625%
///
/// 因此最有效的做法是**让服务端直接返回小图**，从而把降采样比从 15x 降到 2.5x。
/// 这里只改写 URL 的尺寸参数，图片本身仍由服务端缩放、由浏览器原样加载并缓存，
/// 不改变图片格式，也不在本地做任何像素处理。
#[derive(Serialize)]
pub struct AvatarUrlPayload {
    pub url: String,
}

/// 请求上游时使用的头像尺寸。
const AVATAR_REQUEST_SIZE: u32 = 100;

/// 把头像 URL 的尺寸参数改写成小尺寸。
///
/// 腾讯头像接口形如 `?dst_uin=xxx&spec=640&img_type=jpg`，spec 控制返回边长。
/// 仅当 URL 中已存在 `spec` 参数时才改写：其它来源的 URL 语义未知，保持原样，
/// 以免改变上游既有行为。
fn shrink_avatar_url(raw_url: &str) -> Result<String, String> {
    let url = Url::parse(raw_url).map_err(|e| format!("Invalid URL: {}", e))?;

    if url.scheme() != "https" {
        return Err("仅允许 https URL".to_string());
    }

    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();

    let has_spec = pairs.iter().any(|(k, _)| k == "spec");
    if !has_spec {
        return Ok(raw_url.to_string());
    }

    let mut out = url.clone();
    out.set_query(None);
    {
        let mut q = out.query_pairs_mut();
        for (k, v) in &pairs {
            if k == "spec" {
                q.append_pair("spec", &AVATAR_REQUEST_SIZE.to_string());
            } else {
                q.append_pair(k, v);
            }
        }
    }

    Ok(out.into())
}

/// 返回改成小尺寸后的头像 URL。前端把结果直接交给 <img>，
/// 图片仍走浏览器的 HTTP 缓存（上游响应带 Cache-Control: max-age=30 天）。
#[tauri::command]
pub fn resolve_avatar_url(url: String) -> Result<AvatarUrlPayload, String> {
    Ok(AvatarUrlPayload {
        url: shrink_avatar_url(&url)?,
    })
}

#[cfg(test)]
mod tests {
    use super::shrink_avatar_url;

    #[test]
    fn rewrites_spec_when_present() {
        let got = shrink_avatar_url(
            "https://q.qlogo.cn/headimg_dl?dst_uin=2263130787&spec=640&img_type=jpg",
        )
        .unwrap();
        assert!(got.contains("spec=100"), "got: {got}");
        assert!(got.contains("dst_uin=2263130787"), "got: {got}");
        assert!(got.contains("img_type=jpg"), "got: {got}");
    }

    #[test]
    fn keeps_url_without_spec() {
        let src = "https://example.com/avatar.png";
        assert_eq!(shrink_avatar_url(src).unwrap(), src);
    }

    #[test]
    fn rejects_non_https() {
        assert!(shrink_avatar_url("http://q.qlogo.cn/x?spec=640").is_err());
    }
}
