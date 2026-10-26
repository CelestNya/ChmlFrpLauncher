# Arch Linux 打包

本目录把本 fork 的 Linux 分支打成 Arch 系发行版可直接安装的 `.pkg.tar.zst`。

## 安装

从 [Releases](https://github.com/CelestNya/ChmlFrpLauncher/releases) 下载 `arch-v*` 前缀的
release 里的 `chmlfrp-launcher-*.pkg.tar.zst`，然后：

```bash
sudo pacman -U chmlfrp-launcher-*.pkg.tar.zst
```

卸载（保留用户数据 `~/.local/share/net.chmlfrp.launcher`）：

```bash
sudo pacman -Rns chmlfrp-launcher
```

## 本地构建

```bash
cd packaging/arch
makepkg -si
```

默认从本仓库的 `feat/linux-native-wayland-arch` 分支取指定 commit 构建。
打本地工作树（含未提交改动）时用：

```bash
_repo="file://$(git rev-parse --show-toplevel)" \
_commit="$(git rev-parse HEAD)" \
makepkg -si
```

## 与上游桌面版的差异

都是 Linux 适配，不含产品行为改变：

| 改动 | 原因 |
| --- | --- |
| Wayland + NVIDIA 时设 `__NV_DISABLE_EXPLICIT_SYNC=1` | NVIDIA 的 EGL-Wayland 实现了 `wp_linux_drm_syncobj_v1` 却从不调 `set_acquire_point`，KWin 据此按协议报 `Error 71` 断连。关掉显式同步即可，DMA-BUF 零拷贝保留。仅在该组合下生效，对 Intel/AMD 无副作用。 |
| `bundled_frpc_candidates()` 增加 `resource_dir()/resources/frpc` | Tauri 在 Linux 上 `resource_dir()` = `/usr/lib/<productName>`，而 `bundle.resources` 被铺在其下，frpc 实际在 `resources/` 子目录里。上游候选表只查 `resource_dir()/frpc`，导致自带 frpc 被判定不存在、首启联网重下。 |
| CSP `img-src` 增加 `https:` | 头像由 `https://q.qlogo.cn` 提供，原策略只有 `'self' asset: http://asset.localhost blob: data:`，头像被 CSP 直接拦掉，只能看到 fallback。仅放宽图片指令，脚本/连接/对象/框架指令保持收紧。 |
| 头像 URL 改写 `spec` 为 100 | WebKitGTK 的 `<img>` 在 16 倍降采样下滤波不足，细线条被采成暗噪点（暗像素约 1.9%）。改成让服务端直接返回 100px 小图后降到 1.25%。只改 URL 参数，不动格式、不做本地像素处理、不绕过浏览器缓存。 |
| `bundle.category = Network` | 元数据补齐，Windows/macOS 的安装器分组也一并受益。 |

## 打包相关的坑（别踩第二遍）

* **必须 `!lto`**。makepkg 默认把 `-flto=auto` 塞进 `CFLAGS`，cc-rs 会把它传给
  ring 自带的 C/汇编源码，生成 LTO IR 后 `rust-lld` 无法解析，链接期炸成一片
  `undefined symbol: ring_core_0_17_14__*`。
* **必须 `!strip`**。frpc 是 Go 静态二进制，strip 会削掉约 200 字节符号，
  使其与官方 CDN 分发文件不再逐字节相同；启动器首启会核对 hash，不匹配就
  判定文件异常并重新联网下载。
* **frpc 落点**：同时装到 `/usr/lib/ChmlFrpLauncher/resources/frpc`（对齐上游
  bundler 布局）和 `/usr/lib/ChmlFrpLauncher/frpc`（扁平路径兜底）。
* **依赖名**：Arch 没有独立的 `cargo` 包，`cargo`/`rustc` 由 `rust` 提供；
  `rustup` 是另一个包。makedepends 写 `rust`。
* **架构**：只支持 `x86_64`。上游 `src-tauri/frp-binaries` 只提交了
  `linux/amd64` 的 frpc，没有 `linux/aarch64`，aarch64 上打不出可用包。
* **不能用 ubuntu runner 交叉构建**。产物要链接 Arch 的 glibc / webkit2gtk-4.1，
  在 Ubuntu 上打出来的二进制 soname 与符号版本都对不上，装上去直接起不来。
  CI 因此跑在 `archlinux:base-devel` 容器里。

## CI

`.github/workflows/arch-build.yml`：

* push `arch-v*` tag → 构建 + 发 prerelease（prerelease 是为了不顶掉 fnOS 侧的
  Latest）。
* `workflow_dispatch` → 只构建并上传 artifact，不发布。
