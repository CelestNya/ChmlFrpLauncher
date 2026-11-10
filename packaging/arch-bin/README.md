# Arch 镜像包：`chmlfrp-launcher-bin`

打包上游发布页的预编译二进制，产出一个原生 Arch 包（`.pkg.tar.zst`），
不重新编译。

```bash
sudo pacman -U chmlfrp-launcher-bin-0.8.0-1-x86_64.pkg.tar.zst
```

## 为什么不再用源码包

`packaging/arch/` 那条线（源码编译）在 **0.8.0 之后就断了**。上游 commit
`d305024`「feat: 静态集成 ChmlFrp 核心并支持跨平台构建」把内网穿透内核
`frpc` 从独立的预编译文件改成静态链接进主程序：

- 删除 `src-tauri/frp-binaries/` 下的三份二进制（约 42 MB）
- 删除 `src-tauri/src/commands/download.rs` 与前端下载链路
- 新增 `src-tauri/frpc/main.go`（cgo 桥，`//export chmlfrpRun`）与
  `src-tauri/src/frpc_worker.rs`
- `build.rs` 重写成现场 `go build -buildmode=c-archive` 产出 `libchmlfrpc.a`

那个 Go 模块来自 **`TechCat-Team/frp`**，是私有仓库（上游 CI 用
`secrets.CHMLFRP_SOURCE_TOKEN` 检出，`gh api repos/TechCat-Team/frp` 对
普通 token 返回 404）。公开环境拿不到这份源码，`build.rs` 直接 panic。

静态化的副产品是：上游发行包里现在只剩**一个自包含的 ELF**（37 MB，Go 内核
已在其中），没有任何外部 frpc 文件。所以「取现成的二进制重新归档成 Arch 包」
这条路完全可行，而且不需要 Go 工具链、不需要私有源码。

## 这个包做了什么

| 环节 | 做法 |
| --- | --- |
| 取二进制 | `bsdtar` 直接读上游 `.deb`（ar 归档）里的 `data.tar.gz`，不需要 `dpkg` |
| 完整性 | `sha256sums` 校验 `.deb`；另在 CI 里断言包内二进制 sha256 与上游一致 |
| 不重写产物 | `options=('!strip' '!debug')`，保证逐字节等于上游那一份 |
| 图标 | 上游 `ChmlFrpLauncher.png` 改名为 `chmlfrp-launcher.png`，与源码包互换时不残留 |
| `.desktop` | 不用上游那份（`Categories=` 是空值、`Exec` 不带 `%u`），自写一份 |
| 依赖 | 只用 `webkit2gtk-4.1 gtk3 libayatana-appindicator librsvg hicolor-icon-theme`；上游二进制不再链接 `libssl`/`libcrypto` |
| 互斥 | `provides=chmlfrp-launcher` + `conflicts=chmlfrp-launcher`，与源码包不能同时装 |

用户数据在 `~/.local/share/net.chmlfrp.launcher/`，不随包卸载删除，
两个包之间互换**不影响登录态与隧道配置**。

## 相对自建源码版，以下改动不再生效

这些是本 fork 在 `feat/linux-native-wayland-arch` 分支上做的 Linux 适配，
`-bin` 包打的是上游原始二进制，所以它们**全部不存在**：

| 改动 | 现状 | 影响 |
| --- | --- | --- |
| `__NV_DISABLE_EXPLICIT_SYNC=1` 规避（Wayland + NVIDIA 时自动设置） | 不存在（二进制里 0 处） | 实测上游 0.8.0 在原生 Wayland + NVIDIA 下**不再触发** `explicit sync is used, but no acquire point is set` 崩溃，该项可能已自愈；若仍遇到崩溃，手动 `__NV_DISABLE_EXPLICIT_SYNC=1 ChmlFrpLauncher` 即可 |
| `frpc` 候选路径补 `resource_dir()/resources/frpc` | 无意义 | 上游已把 frpc 静态链接进主程序，不再有外部文件 |
| CSP `img-src` 追加 `https:` | 不存在（二进制里仍是原 CSP） | **头像仍会被 CSP 拦住**，只显示首字母兜底 |
| 头像 `spec` 改为 100（`resolve_avatar_url` 命令） | 不存在（二进制里 0 处） | 即使放开 CSP，头像也是直接拉 640px 大图缩到 40px，发糊 |
| `bundle.category = Network` | 不存在 | 只影响上游自己的 `.desktop` 与 AppStream 元数据；本包自写 `.desktop` 已带 `Categories=Network;RemoteAccess;`，**无影响** |

一句话：**功能不缺，缺的是三处观感/健壮性补丁**——其中两项（CSP、头像）
是上游尚未合入的真实缺陷，与 Arch 无关，任何用上游 0.8.0 二进制的地方都一样。

## 构建与发布

```bash
cd packaging/arch-bin
makepkg -si      # 本地构建并安装
makepkg -f       # 只构建
```

CI 在 `.github/workflows/arch-bin-build.yml`：

- push tag `arch-bin-v*` → 构建 + 发布为 **prerelease**
- `workflow_dispatch` → 只构建并上传 artifact

tag 前缀 `arch-bin-v` 与 `arch-v`（源码路线）、`v*-*`（fnOS）都不相交，
且发布为 prerelease，确保 fnOS 的 release 继续持有 `releases/latest`。

发新版本时改 `pkgver`、跑 `updpkgsums` 更新两个 sha256，然后打新 tag。

## 已知限制

- **只支持 x86_64**：上游只发布 `amd64.deb` 与 `aarch64.deb`，后者未在此验证。
- **不能与生产实例并排测试**：单例名 `net.chmlfrp.launcher.SingleInstance`
  写死在二进制里，第二个实例会在约 106 ms 内静默 `exit 0`，并先把已有窗口
  顶到前台。验证时请用私有 D-Bus（`dbus-daemon --session --fork`）+ 独立 `HOME`。
- **控制不了上游发版节奏**：上游一次 release 后本包需要手动 bump `pkgver`。
