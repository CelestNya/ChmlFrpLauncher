# ChmlFrpLauncher 开发文档（fnOS 移植）

> 最后更新：2026-08-13
> 本文件是**开发须知**：双分支工作流、构建链、patch 机制、NAS 部署、测试、发版策略。
> 架构状态见 `docs/architecture.md`（反映代码状态）；交接状态见 `docs/handoff.md`（随代码更新）。
> 一次性实施计划（B1-B5）已归档，见 git 历史与临时目录，不再占仓库位置。

---

## 一、双分支工作流

```
main ──── 纯净上游 + patch 文件 + 构建链（apply-patches.sh / build-fpk.sh / gen-patch.sh / CI）
              ↑ PR（仅 patches/ 目录，CI 自动开）
patcher ── main + src/ 上的侵入式 fnOS 改动（开发态）
```

| 分支 | src/ 状态 | 用途 |
|---|---|---|
| `main` | 零改动（0 侵入），内容 = 上游 develop + fnos 附加文件 | 发布态：CI 构建 .fpk 时在临时副本应用 patch |
| `patcher` | 含全部 fnOS 改动（11 文件） | 开发态：改 fnOS 前端功能直接改这里 |

> **基线**：上游活跃开发分支是 `develop`（v0.8.0 WIP），发布才合并进上游 main。
> fork 基线 = **上游 develop**，main 的 src/ 与之保持一致（0 侵入不变式，patchgen CI 断言
> `git diff --quiet upstream/develop origin/main -- src/`）。

### 日常开发流程（改 fnOS 前端功能）

1. 切到 `patcher` 分支，直接在 `src/` 改 + commit
2. 本地可选：`bash fnos-pack/gen-patch.sh` 预览产物
3. `git push origin patcher` → CI 自动生成 patch → 开 PR（仅 patches/ 目录）
4. review PR 后合并到 main

### 上游同步（主仓更新时）

```
main:   git fetch upstream && git rebase --onto upstream/develop upstream/main main（+ force push）
patcher: git rebase --onto main origin/main patcher（冲突在 patcher 人工解决，main 永远干净）
         → 修改后 push patcher → CI 重新生成 patch
```

- 上游改动 patch 涉及文件 → patch 应用失败 → apply-patches.sh 的 **dry-run 预检**明确报错
- 上游新增 tauri 命令 / Tauri API 调用 → 需补 `fnos-daemon/src/invoke.rs` 命令 + `fnos-shim/tauri-shim.ts` 降级

---

## 二、Patch 机制

```
fnos-pack/patches/
├── fnos-ui-patch.patch        # UI 裁剪：删 TitleBar/杀软弹窗/自启/托盘（4 文件）
└── fnos-feature-patch.patch   # fnOS 功能：壁纸/日志/通知/音效/replay/守护（11 文件）
```

两个 patch 都是**全量**（从干净上游可应用），由 `gen-patch.sh` 从 patcher 分支相对上游 develop 的差异生成，幂等覆盖。

**⚠️ 新增 src/ 改动文件必须登记**：gen-patch.sh 的 `UI_FILES` / `FEATURE_FILES` 数组。漏登记 = patch 未生成 = 修复不进产物（useTunnelProgress 事故教训，gen-patch E6 完整性自检会拦截）。

### 前后端分离重构（2026-08-13，ADR-0001~0004）

**范式**：契约收敛 + 能力协商。不回上游，**shim 重定向**承载数据后端化——业务代码（api.ts/frpcManager）零改动。

| 机制 | 位置 | 说明 |
|------|------|------|
| localStorage 白名单拦截 | `fnos-shim/tauri-shim.ts` | `chmlfrp_user`/`frpc_proxy_config`/`frpcLogLevel`/`bypassProxy`/`restartOnEdit` 重定向 daemon，不落真 localStorage |
| 首帧 boot 注入 | `fnos-daemon/src/main.rs` `serve_index_with_boot` | index.html 注入 `__FNOS_BOOT__`（credential+nodeSettings），shim 同步读防闪烁 |
| 凭据/节点设置存储 | `fnos-daemon/src/settings.rs` | `credential.json`/`node_settings.json`（0600），命令 save/get/clear/status |
| 壁纸文件化 | `fnos-daemon/src/background.rs` | fnOS 选图 dataURL → daemon 解 base64 落盘 → 托管 `/assets/backgrounds/`；砍轮播 |
| 能力协商 | `GET /api/capabilities` + `invoke.rs::SUPPORTED_COMMANDS` | 前端可探测 daemon 命令面；NO_OP 桌面专属命令明确「不支持」 |

**关键约束**：
- 新增白名单 key 需同步 shim `WHITELIST` + daemon `NodeSettings`/`Credential` 结构体
- 登出顺序（ADR-0002）：先停隧道清 g_*.ini → 再 clear_credential（前端 logout.ts 已保证）
- 渲染类偏好（theme/背景/模糊）**留守 localStorage**（ADR-0003，首帧同步读约束）

验证：
- `apply-patches.sh` 内置两段验证（失败即 exit 1）：UI 精简（无 TitleBar/AntivirusWarningDialog）、feature（useBackgroundImage 含 __FNOS__、useTunnelNotifications 含 replay）
- `gen-patch.sh` 内置自检：两个 patch 可从基线干净应用

---

## 三、构建链（fnos-pack/）

| 脚本 | 职责 |
|------|------|
| `apply-patches.sh` | 前端 patch 应用到临时副本 → tsc → vite → shim 注入 → `dist-fnos/` |
| `build-fpk.sh` | daemon musl 静态编译 → 组装 fnpack 项目 → `.fpk`（可 `--bundle` 出更新包） |
| `gen-patch.sh` | patcher 相对上游 develop 生成两个 patch + E6 完整性自检 |

**关键约束**：
- **daemon 必须 musl 静态编译**（fnOS 基底 Debian 12 glibc 2.36，gnu 动态链接加载即崩）——`cargo build --target x86_64-unknown-linux-musl`
- **dist 权限**：打包时 `chmod go-w` 收紧（777→755），否则 fnOS 市场升级保留 777 目录不替换 dist（P2.5 事故，已修）
- **E4 版本一致性**：`fnos-pack/fpk/manifest` == `fnos-daemon/Cargo.toml` == `package.json`（构建期校验，不一致 fail）
- 图标生成失败 exit 1（E7）；`cmd/main start` 校验二进制存在（E5）
- OUTPUT_DIR 边界校验（拒绝 `/`、REPO_ROOT、src）+ 产物临时目录原子 mv（E1）

---

## 四、NAS 部署（真机操作）

> ⚠️ **NAS 只读优先**，写操作先征得同意，数据零接触。杀进程用精确 PID，禁止通配符。

### 升级安装（唯一安全路径）

```bash
# fpk 上传后（scp 到 /tmp/）
sudo appcenter-cli install-fpk /tmp/chmlfrp_X.Y.Z_x86.fpk --volume 1
# ⚠️ 必须 --volume 1（1 = 第一存储卷）；应用 running 同版本会跳过（先 stop 或升版本）
```

**🛑 永远不要用 `install-local` 升级已装应用**——它先 uninstall 清空 `/vol1/@appdata/<app>/` 数据目录
（8/13 事故根因：frpc 二进制/隧道配置全删；code 10237 是 cmd 脚本执行位 bug，每次必失败但副作用照常）。

### 验证安装

| 检查 | 命令 |
|------|------|
| 状态 | `appcenter-cli status chmlfrp` → running |
| socket 0600 | `ls -l /vol1/@appcenter/chmlfrp/app.sock` → `srw-------` |
| daemon 版本 | `tail /vol1/@appdata/chmlfrp/chmlfrp.log` → `chmlfrp-daemon vX.Y.Z` |
| dist 已替换 | `grep -o 'index-*.js' /vol1/@appcenter/chmlfrp/dist/index.html`（应匹配最新 fpk） |
| 安装日志 | `grep 'install app' /var/log/trim_app_center/info.log` |

### 守护状态查询（unix socket API）

```python
# POST /app/chmlfrp/api/invoke（注意前缀！字段是 cmd 不是 command）
{"cmd": "get_process_guard_enabled"}   # → {"ok":true,"data":true}
```

### 关键路径

- 应用目录：`/vol1/@appcenter/chmlfrp/`（daemon + dist + app.sock）
- 数据目录：`/vol1/@appdata/chmlfrp/`（frpc 二进制 / g_*.ini / running_tunnels.json / 日志）
- 网关入口：`/app/chmlfrp`（nginx → unix socket 转发）
- 访问：`https://nas.letian.us.kg/app/chmlfrp`（OAuth 走 account.qzhua.net 设备码）

---

## 五、测试

| 层 | 命令 | 数量 |
|----|------|------|
| daemon (Rust) | `cd fnos-daemon && cargo test` | 66（含 settings/background/capabilities 新测试） |
| 前端 (vitest) | `pnpm exec vitest run` | 45（shim 27 + src 18） |
| 构建链 (bash) | `bash fnos-pack/tests/lib.test.sh` | 9 断言 |
| 类型/风格 | `pnpm exec tsc --noEmit` / `pnpm exec eslint` | — |

**NAS 人工测试**：见 `docs/plans/fnos-nas-test-checklist.md`（已归档验证结果——守护重启、WS 重连去重、日志导出/清空、B2 重放、socket 0600）。

---

## 六、发版策略

**当前（2026-08-13 用户决策）**：上游未发 0.8.0 release（Latest = v0.7.5），dev 代码刚动（8-12 升级依赖），**跟随上游节奏暂不发 release**。

- NAS 上的 0.8.0 仅内部测试版，走 `install-fpk --volume 1` 手动升级
- 自更新通道（依赖 GitHub Release asset）等上游发版后再启用
- 上游发版后：跟随 tag 同步基线 → 发我们的 v0.8.0 + attach fnOS bundle

**发版动作（未来）**：打 tag v* → fnos-build CI（双架构 .fpk + --bundle）→ attach bundles → 前端"检测更新"可用内部自更新。

---

## 七、遗留待办

| 项 | 状态 | 说明 |
|----|------|------|
| 自更新通道 | ⏳ 等 release | 依赖 GitHub Release asset |
| daemon 自保 | ⏳ | 应用中心层面，daemon 崩溃无自拉起 |
| OAuth CORS | ⏳ | iframe 下可能失败（本次实测正常） |
| UI 适配 | ⏳ | 标题栏裁切等 |
| daemon 中-11 WS seq | ⏳ | 前端 B7 已兜底去重，daemon 侧 seq 方案未做 |
| 前端 M3 IndexedDB | ⏳ | localStorage 配额溢出防护未做 |
| 上游 PR | ⏳ 待决策 | 六批修复 PR 回上游 develop（桌面回归门控 __FNOS__ 是硬门槛） |
