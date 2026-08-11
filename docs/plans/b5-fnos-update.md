# B5 实施计划：fnOS 软件内自更新（✅ 已完成并验收）

> 状态：2026-08-11 全部交付。WSL 全链路升级模拟通过（0.7.5 → 0.7.6）。

## 目标
fnOS 版软件内自更新：daemon 检查远端 GitHub Release → 下载更新 bundle（新 daemon + 前端 dist）→ sha256 校验 → 原子替换 → 重启生效。前端"检查更新/下载更新"入口复用现有 UpdateSection。

## 形态与决策
- **更新源**：`https://api.github.com/repos/CelestNya/ChmlFrpLauncher/releases/latest`（无 token 限流 60 次/时，daemon 侧缓存 5 分钟）
- **更新包格式**：单文件 `chmlfrp-fnos-{version}-{platform}.tar.gz`（平台 x86/arm）：
  ```
  manifest.json   # {version, platform, daemon_sha256, dist_sha256}
  chmlfrp-daemon  # 新 daemon 二进制
  dist/           # 新前端资源（shim 注入后）
  ```
- **版本基准**：daemon Cargo 版本统一为前端版本（0.7.5），`/api/bootstrap` 返回即判定基准；manifest 版本与之一致
- **替换位置**：target 目录（TRIM_APPDEST）：daemon 二进制 + dist/；下载/暂存落 TRIM_PKGVAR/update/（保证可写）
- **重启机制**：daemon 替换文件后触发优雅关闭（axum graceful shutdown）→ 释放端口 → spawn 新进程（同环境变量）→ 旧进程退出；PID 文件更新
- **安全**：https 下载（rustls）；manifest.json 内 sha256 逐一校验（daemon、dist 递归）；替换前备份旧文件为 `*.bak`

## 模块与 commit 点位

### U1 — `feat(fnos-daemon): 自更新检查与下载`
- 新增 `update.rs`：`check_update()`（GitHub API latest release → 匹配平台 asset → 比对版本，5 分钟缓存）、`download_update()`（流式下载 bundle → 解包到 TRIM_PKGVAR/update/staged/ → 校验 manifest.json 内 sha256）
- 路由：`GET /api/update/check`、`POST /api/update/download`；invoke 分发表追加 `check_update` / `download_update`
- **验收**：curl 返回 `{available, version, url}`；下载后 staged 目录文件 sha256 与 manifest 一致

### U2 — `feat(fnos-daemon): 应用更新与重启`
- `POST /api/update/apply`：校验 staged → 备份 target 旧文件 → 原子替换（daemon rename + dist 整目录）→ 触发 graceful shutdown → spawn 新 daemon → 旧进程退出
- main.rs 接入 shutdown 广播（update 触发 + SIGTERM）
- **验收**：WSL 模拟 v0.1.0→v0.2.0：apply 后旧进程退出、新进程监听同端口、bootstrap 返回新版本、隧道配置保留

### U3 — `feat(fnos-shim): 更新接线`
- shim `plugin:updater|check` → 透传 daemon `/api/update/check`（返回 null 语义改为可用更新对象）；`downloadAndInstall` → 触发 daemon 下载+应用
- 前端 UpdateSection 无需改动（已有 check/install 入口）；更新进度事件由 daemon 推送（复用 download-progress 通道或新增 update-progress）
- **验收**：浏览器直开 daemon 页面，点"检查更新"返回真实可用更新

### U4 — `ci(fnos-build): fnOS 更新 bundle 发布`
- fnos-build.yml 增加步骤：打 tag 时把 `chmlfrp-fnos-{version}-{platform}.tar.gz`（bundle）作为 asset 附加到该 release（与 .fpk 并列）
- **验收**：release asset 列表含 bundle

### U5 — `docs(fnos): B5 完成同步`
- `docs/plans/b5-fnos-update.md` 勾选；整体 plan 状态更新

## 验收标准（最终 gate）
1. ✅ WSL 全链路：旧 daemon(v0.7.5) → `check_update` 发现 v0.7.6 → `download_update` sha256 通过 → `apply` 替换+重启 → 新版本 bootstrap 0.7.6、同端口、用户数据保留、旧进程退出
2. ✅ shim updater 接线（check→daemon /api/update/check；download_and_install→download+apply）
3. ✅ 安全：bundle 逐文件 sha256 校验失败拒绝 apply；下载走 rustls https；bundle 版本必须高于当前（防降级）；`UPDATE_API_URL` 可覆盖更新源（测试/自托管）
4. ✅ 桌面版零影响（daemon 独立 crate）
5. ✅ CI：fnos-build.yml 增加 `--bundle` 出包 + attach job 把 bundle 附加到 Release

## 风险
- GitHub API 限流（60/h）：5 分钟缓存；`UPDATE_API_URL` 可覆盖
- run-as=package 对 target 写权限未验证：apply 若失败报明确错误；真机验证清单第 4 条
- 替换后端口竞争：SO_REUSEPORT + 先 spawn 新进程再优雅关闭旧进程，WSL 验证无中断
- 版本号联动：bundle manifest 版本必须高于当前 daemon（apply 前校验）

