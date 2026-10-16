// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(target_os = "linux")]
    apply_linux_workarounds();

    app_lib::run();
}

/// Linux 平台启动前的兼容性处理。
///
/// 只在用户没有显式设置对应环境变量时生效，因此始终可以被外部覆盖。
#[cfg(target_os = "linux")]
fn apply_linux_workarounds() {
    use std::env;

    // 仅针对 NVIDIA 专有驱动。
    //
    // 在 Wayland 会话下（KWin 6.x 提供 wp_linux_drm_syncobj_manager_v1），
    // NVIDIA 的 EGL-Wayland 实现会为该 surface 订阅显式同步扩展，但提交
    // 缓冲区时并未按协议要求设置 acquire point，合成器随即以协议错误终止
    // 连接，GTK 将其转为致命错误并中止进程：
    //
    //   wl_display.error(wp_linux_drm_syncobj_surface_v1, 4,
    //       "explicit sync is used, but no acquire point is set")
    //   Gdk-Message: Error 71 (协议错误) dispatching to Wayland display.
    //
    // 关闭驱动侧的显式同步即可从源头避免该错误，且**保留 DMA-BUF 零拷贝
    // 渲染路径**（相比禁用 WEBKIT DMA-BUF 的做法不损失合成性能），窗口仍
    // 运行在原生 Wayland 上，无需回退到 XWayland。
    if env::var_os("__NV_DISABLE_EXPLICIT_SYNC").is_none() {
        env::set_var("__NV_DISABLE_EXPLICIT_SYNC", "1");
    }
}
