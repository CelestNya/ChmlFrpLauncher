use crate::models::HttpRequestOptions;
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use image::imageops::FilterType;
use reqwest::Url;
use serde::Serialize;

#[derive(Serialize)]
pub struct HttpResponsePayload {
    pub status: u16,
    pub body: String,
}

/// 头像代理下载 + 高质量缩放后的 dataURL。
///
/// 背景：头像原图通常为 640×640，而界面仅显示 40~44px，需要 14~16 倍降采样。
/// WebKitGTK 的 <img> 大比例降采样滤波质量不足，细线条会被稀疏采样成暗色噪点
/// （实测暗像素比例约 1.9%）；Chromium 实现质量更高，故 Windows 上无明显噪点。
/// 且该行为不受 CSS image-rendering 影响（auto/smooth/pixelated 渲染结果虽有差异，
/// 但都无法达到高质量重采样水平）。
///
/// 方案：在 Rust 侧取原图后，先用 Lanczos3 逐级减半（避免一次性大比例降采样产生
/// 混叠），再缩放到目标尺寸，最后编码为 PNG dataURL 交给 <img>。此时前端只需
/// 1:1 或轻微缩放显示，实测暗像素比例可降到约 1.2%，噪点明显减少。
#[derive(Serialize)]
pub struct AvatarPayload {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

/// 目标边长。略大于界面最大显示尺寸（h-11 = 44px），兼顾 HiDPI 屏幕。
const AVATAR_TARGET: u32 = 96;

fn validate_avatar_url(raw_url: &str) -> Result<Url, String> {
    let url = Url::parse(raw_url).map_err(|e| format!("Invalid URL: {}", e))?;
    if url.scheme() != "https" {
        return Err("仅允许 https 请求".to_string());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("URL 不允许包含凭据".to_string());
    }
    let host = url.host_str().ok_or_else(|| "URL 缺少 host".to_string())?;
    if !is_allowed_avatar_host(host) {
        return Err("URL 不在允许列表".to_string());
    }
    Ok(url)
}

fn is_allowed_avatar_host(host: &str) -> bool {
    host == "q.qlogo.cn"
        || host == "thirdqq.qlogo.cn"
        || host.ends_with(".qlogo.cn")
        || host.ends_with(".uapis.cn")
        || host == "chmlfrp.net"
        || host.ends_with(".chmlfrp.net")
}

/// 逐级减半后再缩放，抑制混叠。
fn downsample_high_quality(img: &image::DynamicImage, target: u32) -> image::RgbaImage {
    let mut cur = img.to_rgba8();
    while cur.width() > target * 2 && cur.height() > target * 2 {
        let w = (cur.width() / 2).max(target);
        let h = (cur.height() / 2).max(target);
        cur = image::imageops::resize(&cur, w, h, FilterType::Lanczos3);
    }
    image::imageops::resize(&cur, target, target, FilterType::Lanczos3)
}

#[tauri::command]
pub async fn fetch_avatar_sharp(
    url: String,
    bypass_proxy: Option<bool>,
) -> Result<AvatarPayload, String> {
    let parsed = validate_avatar_url(&url)?;

    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent("ChmlFrpLauncher/1.0");

    if bypass_proxy.unwrap_or(true) {
        builder = builder.proxy(reqwest::Proxy::custom(
            move |_url| -> Option<reqwest::Url> { None },
        ));
    }

    let client = builder
        .build()
        .map_err(|e| format!("Failed to create client: {}", e))?;

    let bytes = client
        .get(parsed)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read bytes: {}", e))?;

    let img = image::load_from_memory(&bytes).map_err(|e| format!("Decode failed: {}", e))?;

    // 原图本身已足够小，直接编码，避免无谓的有损
    let processed = if img.width() <= AVATAR_TARGET * 2 && img.height() <= AVATAR_TARGET * 2 {
        img.to_rgba8()
    } else {
        downsample_high_quality(&img, AVATAR_TARGET)
    };

    let (w, h) = (processed.width(), processed.height());
    let mut png_bytes: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(processed)
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .map_err(|e| format!("Encode failed: {}", e))?;

    Ok(AvatarPayload {
        data_url: format!("data:image/png;base64,{}", B64.encode(&png_bytes)),
        width: w,
        height: h,
    })
}
