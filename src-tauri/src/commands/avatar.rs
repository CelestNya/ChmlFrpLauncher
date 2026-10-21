use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use image::imageops::FilterType;
use reqwest::Url;
use serde::Serialize;

/// 头像代理下载 + 高质量缩放后的 dataURL。
///
/// 背景：头像原图通常为 640×640，而界面仅显示 40~44px，需要 14~16 倍降采样。
/// WebKitGTK 的 <img> 在大比例降采样时滤波质量不足，细线条会被稀疏采样成暗色
/// 噪点；Chromium 的实现更好，因此同样代码在 Windows 上噪点不明显。实测该问题
/// 不受 CSS image-rendering 影响（auto / smooth / pixelated / -webkit-optimize-
/// contrast 的渲染结果虽有差异，但都达不到高质量重采样的水平）。
///
/// 在 Xvfb 中用真实 WebKitGTK 对比多组方案后，效果排序为：
///   原图 640px 直出            暗像素 1.875%
///   Lanczos3 逐级减半到 96px   暗像素 1.188%
///   **改用接口返回的 100px 小图  暗像素 1.125%（最优）**
///   Lanczos + 锐化             1.5%~1.75%（更差，锐化会放大噪点）
///
/// 因此这里采用两级策略：
///   1. 若 URL 支持尺寸参数（如腾讯头像的 spec），先改写成小尺寸再下载——减小
///      降采样比是收益最大的一步，同时少下载数十 KB；
///   2. 下载后若原图仍偏大，再用 Lanczos3 逐级减半兜底。
#[derive(Serialize)]
pub struct AvatarPayload {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}

/// 目标边长。略大于界面最大显示尺寸（h-11 = 44px），兼顾 HiDPI 屏幕。
const AVATAR_TARGET: u32 = 96;

/// 请求上游时使用的小图尺寸。取 100 是实测最优（暗像素最低），
/// 且在 2x 缩放下仍有余量。
const AVATAR_REQUEST_SIZE: u32 = 100;

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

/// 把头像 URL 的尺寸参数改写成小尺寸。
///
/// 腾讯头像接口形如 `?dst_uin=xxx&spec=640&img_type=jpg`，其中 spec 控制返回
/// 的边长。仅当已存在该参数时才改写，避免影响其它来源的 URL 语义。
fn shrink_avatar_url(url: &Url) -> Url {
    let mut out = url.clone();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let has_spec = pairs.iter().any(|(k, _)| k == "spec");
    if !has_spec {
        return out;
    }
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
    out
}

/// 逐级减半后再缩放到目标尺寸，抑制混叠。
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
    let request_url = shrink_avatar_url(&parsed);

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
        .get(request_url)
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to read bytes: {}", e))?;

    let img = image::load_from_memory(&bytes).map_err(|e| format!("Decode failed: {}", e))?;

    // 若上游已返回较小尺寸，则无需缩放（实测直接用小图效果最好）
    let processed = if img.width() <= AVATAR_TARGET * 2 && img.height() <= AVATAR_TARGET * 2 {
        img.to_rgba8()
    } else {
        downsample_high_quality(&img, AVATAR_TARGET)
    };

    let (w, h) = (processed.width(), processed.height());
    let mut png_bytes: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgba8(processed)
        .write_to(
            &mut std::io::Cursor::new(&mut png_bytes),
            image::ImageFormat::Png,
        )
        .map_err(|e| format!("Encode failed: {}", e))?;

    Ok(AvatarPayload {
        data_url: format!("data:image/png;base64,{}", B64.encode(&png_bytes)),
        width: w,
        height: h,
    })
}
