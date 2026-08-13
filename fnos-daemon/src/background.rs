//! 壁纸文件管理（前后端分离重构·阶段 2a）。
//!
//! 规格源：src-tauri/src/commands/background.rs（桌面版行为照搬，适配 fnOS）：
//! - `copy_background_image` / `copy_background_video`：复制用户选的文件到
//!   data_dir/backgrounds/，返回文件系统路径（前端加 daemon 托管前缀渲染）
//! - `import_background_image_folder`：导入文件夹图片到 backgrounds/slideshow/
//!   （fnOS 砍轮播后前端不再调用，保留实现兼容未来恢复）
//! - `get_background_video_path`：取 backgrounds/ 顶层第一个视频路径
//!
//! ADR-0004：fnOS 壁纸从 base64 dataURL 改为文件路径，经 daemon 静态托管
//! `backgrounds/` 访问 + 浏览器缓存，消灭 localStorage 配额问题（M3）。
//! 注意：文件落 data_dir/backgrounds/ 而非 web_dir——daemon 需额外挂载
//! `/assets/backgrounds/*` 静态路由指向该目录（见 main.rs）。

use std::fs;
use std::path::{Path, PathBuf};

/// 壁纸文件根目录（data_dir/backgrounds）。
fn background_dir(data_dir: &Path) -> Result<PathBuf, String> {
    let background_dir = data_dir.join("backgrounds");
    fs::create_dir_all(&background_dir).map_err(|e| format!("创建壁纸目录失败: {}", e))?;
    Ok(background_dir)
}

/// 复制单个文件到壁纸目录（图片/视频通用），返回目标绝对路径。
/// 与桌面版语义一致：文件名取源文件名，覆盖同名。
pub fn copy_background_file(data_dir: &Path, source_path: &str) -> Result<String, String> {
    let background_dir = background_dir(data_dir)?;
    let source = PathBuf::from(source_path);
    let file_name = source
        .file_name()
        .ok_or_else(|| "无法获取文件名".to_string())?
        .to_string_lossy()
        .to_string();
    let dest_path = background_dir.join(&file_name);
    fs::copy(source_path, &dest_path).map_err(|e| format!("复制文件失败: {}", e))?;
    Ok(dest_path.to_string_lossy().to_string())
}

/// 保存浏览器上传的 dataURL 图片（ADR-0004 阶段 2b）。
///
/// fnOS 浏览器环境无文件系统路径，shim 把选图的 dataURL 上传，本函数解 base64
/// 落盘 `data_dir/backgrounds/<sanitized_file_name>`，返回托管相对路径
/// `backgrounds/<file>`（daemon 静态托管 /assets/backgrounds/ 的映射）。
pub fn save_background_data_url(
    data_dir: &Path,
    data_url: &str,
    file_name: &str,
) -> Result<String, String> {
    // dataURL 形如 "data:image/png;base64,iVBOR..." → 逗号后是 base64 载荷
    let base64_part = data_url
        .split_once(',')
        .map(|(_, payload)| payload)
        .ok_or_else(|| "无效的 dataURL（缺逗号分隔）".to_string())?;
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_part.trim())
        .map_err(|e| format!("base64 解码失败: {}", e))?;

    // 文件名净化：仅保留字母数字 - _ .，防路径穿越（shim 传文件名可能是用户输入）
    let safe_name: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe_name.is_empty() || safe_name == "." || safe_name == ".." {
        return Err("非法文件名".to_string());
    }

    let background_dir = background_dir(data_dir)?;
    let dest_path = background_dir.join(&safe_name);
    fs::write(&dest_path, &bytes).map_err(|e| format!("写入文件失败: {}", e))?;
    Ok(format!("backgrounds/{}", safe_name))
}

/// 导入文件夹图片到 backgrounds/slideshow/（桌面版同语义，fnOS 轮播砍后前端不调用）。
/// 清空旧 slideshow 目录，源目录中图片按 `{:03}_{safe_stem}.{ext}` 复制。
pub fn import_background_image_folder(
    data_dir: &Path,
    dir_path: &str,
) -> Result<Vec<String>, String> {
    let source_dir = PathBuf::from(dir_path);
    if !source_dir.is_dir() {
        return Err("选择的路径不是文件夹".to_string());
    }

    let slideshow_dir = background_dir(data_dir)?.join("slideshow");
    if slideshow_dir.exists() {
        fs::remove_dir_all(&slideshow_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&slideshow_dir).map_err(|e| e.to_string())?;

    let mut imported = Vec::new();
    let mut counter = 0usize;
    let extensions = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];

    let entries = fs::read_dir(&source_dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        let ext = ext.to_lowercase();
        if !extensions.contains(&ext.as_str()) {
            continue;
        }

        let file_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("image");
        let safe_stem: String = file_stem
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let file_name = format!("{:03}_{}.{}", counter, safe_stem, ext);
        let dest_path = slideshow_dir.join(file_name);

        fs::copy(&path, &dest_path)
            .map_err(|e| format!("复制文件失败 {}: {}", path.to_string_lossy(), e))?;
        imported.push(dest_path.to_string_lossy().to_string());
        counter += 1;
    }

    Ok(imported)
}

/// 取 backgrounds/ 顶层第一个视频文件路径（桌面版同语义）。
pub fn get_background_video_path(data_dir: &Path) -> Result<Option<String>, String> {
    let background_dir = data_dir.join("backgrounds");
    if !background_dir.exists() {
        return Ok(None);
    }

    let entries = fs::read_dir(&background_dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                let ext_lower = ext.to_string_lossy().to_lowercase();
                if matches!(ext_lower.as_str(), "mp4" | "webm" | "ogv" | "mov") {
                    return Ok(Some(path.to_string_lossy().to_string()));
                }
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "fnos-bg-{}-{}",
            name,
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn 复制图片到backgrounds目录() {
        let dir = temp_dir("copy");
        let src = dir.join("src.png");
        fs::write(&src, b"PNG-DATA").unwrap();

        let dest = copy_background_file(&dir, &src.to_string_lossy()).unwrap();
        let dest_path = PathBuf::from(&dest);
        assert!(dest_path.exists());
        assert_eq!(dest_path.file_name().unwrap(), "src.png");
        assert_eq!(fs::read(&dest_path).unwrap(), b"PNG-DATA");

        // 源文件不存在 → 报错
        let err = copy_background_file(&dir, &dir.join("missing.png").to_string_lossy());
        assert!(err.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 导入文件夹按序号复制() {
        let dir = temp_dir("import");
        let src_dir = dir.join("src");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("a.png"), b"A").unwrap();
        fs::write(src_dir.join("b.JPG"), b"B").unwrap();
        fs::write(src_dir.join("c.txt"), b"ignore").unwrap(); // 非图片扩展名
        fs::write(src_dir.join("noext"), b"ignore").unwrap(); // 无扩展名

        let imported = import_background_image_folder(&dir, &src_dir.to_string_lossy()).unwrap();
        assert_eq!(imported.len(), 2, "只导入图片扩展名文件");

        let slideshow = dir.join("backgrounds").join("slideshow");
        assert!(slideshow.join("000_a.png").exists());
        assert!(slideshow.join("001_b.JPG").exists());
        assert!(!slideshow.join("c.txt").exists());

        // 文件名含非法字符 → 下划线替换
        fs::write(src_dir.join("my photo!.png"), b"C").unwrap();
        let imported2 = import_background_image_folder(&dir, &src_dir.to_string_lossy()).unwrap();
        assert_eq!(imported2.len(), 3);
        assert!(slideshow.join("002_my_photo_.png").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 导入非文件夹报错() {
        let dir = temp_dir("notdir");
        let err = import_background_image_folder(&dir, &dir.join("x.txt").to_string_lossy());
        assert!(err.is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 取视频路径() {
        let dir = temp_dir("video");
        let bg = dir.join("backgrounds");
        fs::create_dir_all(&bg).unwrap();
        fs::write(bg.join("img.png"), b"img").unwrap();
        fs::write(bg.join("movie.mp4"), b"video").unwrap();

        let found = get_background_video_path(&dir).unwrap();
        assert_eq!(found, Some(bg.join("movie.mp4").to_string_lossy().to_string()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn 无壁纸目录返回None() {
        let dir = temp_dir("novideo");
        assert_eq!(get_background_video_path(&dir).unwrap(), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dataurl保存为壁纸文件() {
        let dir = temp_dir("dataurl");
        // "hello" 的 base64 = aGVsbG8=
        let rel = save_background_data_url(
            &dir,
            "data:image/png;base64,aGVsbG8=",
            "my-bg.png",
        )
        .unwrap();
        assert_eq!(rel, "backgrounds/my-bg.png");

        let dest = dir.join("backgrounds").join("my-bg.png");
        assert!(dest.exists());
        assert_eq!(fs::read(&dest).unwrap(), b"hello");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dataurl_文件名净化防穿越() {
        let dir = temp_dir("sanitize");
        // "../evil.png" → "/" 变 "_"，结果 ".._evil.png"（无独立 ".." 段，不构成穿越）
        let rel = save_background_data_url(
            &dir,
            "data:image/png;base64,aGVsbG8=",
            "../evil.png",
        )
        .unwrap();
        assert_eq!(rel, "backgrounds/.._evil.png", "路径穿越字符应被净化");
        // 结果文件落在 backgrounds/ 内（非父目录）
        assert!(dir.join("backgrounds").join(".._evil.png").exists());
        assert!(!dir.join("evil.png").exists(), "不得写出 backgrounds/ 目录");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn dataurl_非法base64报错() {
        let dir = temp_dir("badb64");
        let err = save_background_data_url(&dir, "data:image/png;base64,!!!", "x.png");
        assert!(err.is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
