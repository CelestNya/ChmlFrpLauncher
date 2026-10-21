// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    apply_linux_workarounds();

    app_lib::run();
}

/// 判断当前系统是否在用 NVIDIA 专有驱动。
///
/// 两个条件任一成立即可：
///   1. 存在 NVIDIA 内核设备节点（`/dev/nvidiactl` 或 `/dev/nvidia0`）；
///   2. 能加载到 `libnvidia-glcore.so`（部分容器/特殊部署下设备节点不可见）。
///
/// 之所以要判定，是因为下面的 `__NV_DISABLE_EXPLICIT_SYNC` 只被 NVIDIA 的
/// 专有库读取。Mesa（AMD / Intel）完全忽略它，设置与否都不影响功能；但为了让
/// 非 NVIDIA 用户的环境保持干净、避免传播无关变量，仍然只在确实用到 NVIDIA
/// 驱动时才设置。
#[cfg(target_os = "linux")]
fn has_nvidia_driver() -> bool {
    use std::path::Path;

    if Path::new("/dev/nvidiactl").exists() || Path::new("/dev/nvidia0").exists() {
        return true;
    }

    // 退化判定：动态库是否存在（不加载，仅探测）
    const CANDIDATES: [&str; 3] = [
        "/usr/lib/libnvidia-glcore.so",
        "/usr/lib/libnvidia-glcore.so.1",
        "/usr/lib64/libnvidia-glcore.so",
    ];
    CANDIDATES.iter().any(|p| Path::new(p).exists())
}

/// Linux 平台启动前的兼容性处理。
///
/// 只在用户没有显式设置对应环境变量时生效，因此始终可以被外部覆盖。
#[cfg(target_os = "linux")]
fn apply_linux_workarounds() {
    use std::env;

    // 仅针对 NVIDIA 专有驱动，且仅在 Wayland 会话下才有意义。
    //
    // KWin 6.x 等现代合成器会提供 wp_linux_drm_syncobj_manager_v1（显式同步），
    // NVIDIA 的 EGL-Wayland 实现会为该 surface 订阅该扩展，但提交缓冲区时并未
    // 按协议要求设置 acquire point，合成器随即以协议错误终止连接，GTK 将其转为
    // 致命错误并中止进程：
    //
    //   wl_display.error(wp_linux_drm_syncobj_surface_v1, 4,
    //       "explicit sync is used, but no acquire point is set")
    //   Gdk-Message: Error 71 (协议错误) dispatching to Wayland display.
    //
    // 关闭驱动侧的显式同步即可从源头避免该错误，且**保留 DMA-BUF 零拷贝渲染
    // 路径**（相比禁用 WebKit DMA-BUF 的做法不损失合成性能），窗口仍运行在
    // 原生 Wayland 上，无需回退到 XWayland。
    //
    // 注意：该变量只被 NVIDIA 专有库读取，Mesa（AMD / Intel）完全忽略它，
    // 因此在非 NVIDIA 环境下设置它也是无害的；这里仍然加上判定，让无关环境
    // 不被注入额外变量。
    let on_wayland = env::var_os("WAYLAND_DISPLAY").is_some()
        || env::var_os("XDG_SESSION_TYPE")
            .map(|v| v.to_string_lossy().eq_ignore_ascii_case("wayland"))
            .unwrap_or(false);

    if on_wayland
        && has_nvidia_driver()
        && env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none()
    {
        env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
    }
}

/// 该判定函数依赖运行环境（设备节点 / 动态库 / 会话类型），
/// 这里仅验证“不会 panic、返回布尔值”这一契约；
/// 具体取值由机器环境决定，不适合断言固定值。
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn has_nvidia_driver_returns_bool() {
        // 只要求能正常求值，不假设具体结果
        let _ = has_nvidia_driver();
    }

    #[test]
    fn apply_workarounds_is_idempotent() {
        // 不覆盖用户已有的显式设置
        std::env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "0");
        apply_linux_workarounds();
        assert_eq!(
            std::env::var("__NV_DISABLE_EXPLICIT_SYNC").unwrap(),
            "0",
            "已有的用户设置不应被覆盖"
        );
        std::env::remove_var("__NV_DISABLE_EXPLICIT_SYNC");
    }
}
