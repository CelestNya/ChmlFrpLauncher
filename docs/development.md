# ChmlFrpLauncher 开发文档（fnOS 移植）

> 最后更新：2026-08-19
> 本文件是**开发须知**：四分支工作流、契约层职责、构建链、patch 机制、版本号体系、NAS 部署、测试、发版流程。
> 架构状态见 `docs/architecture.md`（反映代码状态）；交接状态见 `docs/handoff.md`（随代码更新）。
> 一次性实施计划（B1-B5）已归档，见 git 历史与临时目录，不再占仓库位置。

---

## 一、分支矩阵（2026-08-19 架构定版）

四分支按「谁持有改动、谁拥有文件」切分，各司其职：

```
main ──────── 干净基线（src/ 与上游一致，0 侵入）+ 每次发版的组合快照（fnos-* 目录 force-add）
adapter/v0.7.5 ── fnos-api 纯契约层（manifest + crop patch + 生成/校验脚本），只暴露标准化接口
mod ──────── fnos-daemon + fnos-shim + fnos-pack（业务实现 + shim + 持久化 + 打包链）
patcher ──── 前端开发台（src/ 上的侵入式 fnOS 改动 + 测试文件），gen-patch 的差异源
```

| 分支 | 拥有目录 | 职责 | 构建参与 |
|------|----------|------|----------|
| `main` | 全部（快照） | 发布态：tag 触发 CI 构建 .fpk | 是（组合快照） |
| `adapter/v0.7.5` | `fnos-api/` | 契约面：manifest（命令/事件/存储/HTTP/UI 配置）、ui-crop.patch、generate-ui-config、verify-adapter | 是（crop + 生成 uiConfig） |
| `mod`（原 patch 更名） | `fnos-daemon/` `fnos-shim/` `fnos-pack/` | 业务逻辑 + 运行时 shim + 数据持久化 + feature patch + 打包链 | 是（daemon + shim + feature） |
| `patcher` | `src/` + 全部（本地开发） | 前端 src 开发 + 测试文件 + gen-patch 差异源 | 否（不直接出产物） |

**职责边界（用户 2026-08-16 决策定版）**：

- **adapter 只做最核心的接口暴露**：把上游版本差异全部标准化为接口/配置，不含业务实现、不含 shim。padding、UI 裁剪等一次性改动统一进 `uiConfig` 配置（manifest 驱动，构建期生成）。
- **mod 只调用 adapter 暴露的标准化接口**：完全不感知原 app 的版本差异。shim（Tauri 运行时模拟层）、daemon 持久化（credential/node_settings/日志 0600）、业务逻辑全在 mod。
- **存储归 mod 管理持久化**：adapter 在 `manifest.capabilities.storage` 登记归属与权限（契约），mod 实现落盘。
- **patcher 只做前端开发**：不持有 fnos-* 目录的权威版本（工作副本可含，用于本地编译/gen-patch，提交时只提 src/ 与测试）。

### 日常开发流程

| 要改什么 | 在哪改 | 提交到 |
|----------|--------|--------|
| 前端功能（src/）| 切换 `patcher` 分支改 `src/` | `patcher` |
| UI 裁剪/配置（padding、TitleBar 等）| 改 `adapter` 分支 `fnos-api/.../manifest.json`（uiConfig.values）| `adapter/v0.7.5` |
| daemon 命令/持久化/shim 行为 | 切换 `mod` 分支改 `fnos-daemon/` `fnos-shim/` | `mod` |
| 打包链/脚本 | 切换 `mod` 分支改 `fnos-pack/` | `mod` |

前端改完后**重新生成 patch**（见 §二）：`bash fnos-pack/gen-patch.sh`（在组合工作区：src 为应用态 + fnos-api 来自 adapter + fnos-pack 来自 mod）→ crop 提交 adapter、feature 提交 mod。

### 上游同步

**自动跟随**：`fnos-upstream-follow.yml`（每 6 小时 + 手动触发）——上游有新提交 → main 合并（src/ 0 侵入天然无冲突）→ gen-patch 重生成 → 自动 PR 供 review。上游改动 patch 涉及文件 → 冲突在 patcher 人工解，apply-patches 的 dry-run 预检明确报错。

---

## 二、Patch 机制

```
fnos-api/adapters/v0.7.5/
├── manifest.json          # 契约权威：命令/事件/存储/HTTP/UI 配置/文件清单
└── ui-crop.patch          # UI 裁剪（App.tsx / AppearanceSection / SystemSection，由 gen-patch 生成）

fnos-pack/patches/
├── manifest.json          # mod 层版本声明（patchSetVersion 2.0.0，requires.api >=1.1.0）
└── fnos-feature-patch.patch  # fnOS 功能（13 文件登记，12 个实际含差异）
```

- 两个 patch 都是**全量**（从干净上游可应用），由 `gen-patch.sh` 从 patcher 分支相对基线的差异生成，幂等覆盖。
- 文件清单从 adapter manifest 的 `uiFiles`（uiCrop / featureBranch）读取——**清单权威归 adapter**（配置驱动）。
- **新增 src/ 改动文件必须登记**到 manifest 的 uiFiles：漏登记 = patch 未生成 = 修复不进产物。gen-patch 的 E6 完整性自检（src/ 相对基线的全部差异必须被清单覆盖）+ 未跟踪文件检查会拦截。
- 测试文件（`*.test.ts`）是 patcher 独有：不进 patch、不进构建产物，main 保持 0 侵入（E6 豁免）。

### padding 配置化（2026-08-19 根治）

`shouldPadTop` 兜底分支读 `uiConfig.padTop`（adapter manifest 配置项，构建期生成 fnos-ui-config.ts）。

- 配置项在 manifest `uiConfig.values.padTop: true`；App.tsx 引用 `uiConfig.padTop`。
- 构建期由 `generate-ui-config.mjs` 生成 `src/fnos-ui-config.ts`（manifest 唯一权威，覆盖 patcher 手写版）。
- **为什么这是根治**：此前 padding 是 patcher 硬编码 `false` + adapter crop 手工改 `true`，gen-patch 每次全量重新生成就把手工段冲回——两次回归同源（1.5.5 / 1.6.1）。配置化后 crop 零手工维护段。
- 守护：gen-patch 校验「padTop 在 manifest + uiConfig.padTop 在 crop 引用」双在；apply-patches 校验生成配置不含 `padTop: false` + App.tsx 必须引用 `uiConfig.padTop`。

### 前后端分离（2026-08-13，ADR-0001~0004）

**范式**：契约收敛 + 能力协商。不回上游，**shim 重定向**承载数据后端化——业务代码（api.ts/frpcManager）零改动。

| 机制 | 位置 | 说明 |
|------|------|------|
| localStorage 白名单拦截 | `fnos-shim/tauri-shim.ts` | `chmlfrp_user`/`frpc_proxy_config`/`frpcLogLevel`/`bypassProxy`/`restartOnEdit` 重定向 daemon，不落真 localStorage |
| 首帧 boot 注入 | `fnos-daemon/src/main.rs` `serve_index_with_boot` | index.html 注入 `__FNOS_BOOT__`（credential+nodeSettings），shim 同步读防闪烁 |
| 凭据/节点设置存储 | `fnos-daemon/src/settings.rs` | `credential.json`/`node_settings.json`（0600），命令 save/get/clear/status |
| 壁纸文件化 | `fnos-daemon/src/background.rs` | fnOS 选图 dataURL → daemon 解 base64 落盘 → 托管 `/assets/backgrounds/`；砍轮播 |
| frpc 日志持久化 | `fnos-daemon/src/logfile.rs` | frpc 日志落盘（轮转 .1），应用/daemon 重启不丢（改进批 1 用户验收项） |
| 能力协商 | `GET /api/capabilities` + `invoke.rs::SUPPORTED_COMMANDS` | 前端可探测 daemon 命令面；NO_OP 桌面专属命令明确「不支持」 |

### 更新流程（改进批 1+2，功能位 6，B8/B9/B10）

| 机制 | 位置 | 说明 |
|------|------|------|
| 分阶段下载进度 | `update.rs` + `UpdateDialog.tsx` | 连接→下载→校验→应用四阶段，progress 事件驱动 |
| 多源镜像退避 | `update.rs` `mirror_urls` | 官方 GitHub 优先 + ghproxy 系列镜像，各源 300ms 退避，错误聚合显示 |
| 下载异步化 | `update.rs` B9 | fnOS 网关转发超时（504）→ POST download 立即返回 + tokio 后台任务 + `download-result` 事件推送 |
| 更新完成自动刷新 | `tauri-shim.ts` B8 | WS 重连后比对 /api/bootstrap 版本，变化则 overlay「更新完成」+ location.reload() |
| 失败重试 UI | `UpdateDialog.tsx` | 聚合错误限高滚动（800+ 字符防裁切）+ 重试按钮 |
| 检查强制刷新 | `update.rs` B10 | `check?force=1` 绕过 5 分钟缓存（防二次检查误报「已是最新」） |
| 防降级 | `update.rs` `verify_bundle` | 更新包版本不高于当前即拒绝 |

---

## 三、契约层（fnos-api）与版本号体系

### 分层

```
上游 chmlfrp（服务面，不可控，0.7.5 基线）
    ↓ adapter 映射（每上游版本一个，纯契约：命令/事件/存储/HTTP/UI 配置）
API 契约 fnos-api（可控，apiVersion 1.1.0）
    ↓ mod 业务逻辑（调用 adapter 接口，manifest 声明 requires: {api: ">=1.1.0"} 硬依赖）
fnOS 适配实现（fnos-daemon + fnos-shim）
```

- 契约位置：`fnos-api/API.md`（六面：命令/事件/存储/HTTP/UI/插件）；`fnos-api/adapters/<上游版本>/manifest.json`（能力面声明）；`verify-adapter.sh`（前端 invoke 调用 ⊆ adapter 能力面 + uiConfig 键 ⊆ manifest keys + 手写 fnos-ui-config 与 manifest 生成版一致）
- 版本化规则：adapter 版本号 = `{上游基线}-{apiVersion}`（如 `0.7.5-1.1.0`）；apiVersion 按接口演进 semver（major=破坏 / minor=新增 / patch=修复）。**上游更新 ≠ API 更新**——只有接口形态变化才 bump（0.7.5/0.8.0 命令面 diff 零，命令映射跨版本零工作量）。

### 版本号体系（2026-08-16 用户决策）

**patchSetVersion = 架构.功能.修复**（mod 层版本）：

| 位 | 语义 | 示例 |
|----|------|------|
| 架构（major） | 破坏性架构变更（契约面/uiConfig 格式/shim 重构/分支重构） | 1 → 2（adapter 纯契约定版 + API 1.1.0 + mod 更名 + padding 配置化） |
| 功能（minor） | 新功能/能力一代 +1 | 5 → 6（更新流程升级：分阶段/镜像退避/异步化/日志持久化） |
| 修复（patch） | bug 修复 +1 | 2 → 3（padding 回归、掉登录、限高滚动…） |

**E4 联合版本号** = app 版本 + patchSetVersion（如 `0.7.5-2.0.0`），贯穿：`fnos-pack/fpk/manifest` == `fnos-daemon/Cargo.toml` == `package.json`（构建期校验，不一致 fail）。

已发布版本链：1.5.3（UI 裁剪归 adapter）→ 1.5.4（改进批 1：日志/阶段进度/镜像退避）→ 1.6.0（功能位 6 更新流程全链路 + 改进批 2 前端）→ **2.0.0**（架构 2 定版 + padding 配置化根治）。中间检验版 1.5.5/1.5.6/1.6.1 未发版。

---

## 四、构建链（fnos-pack/）

| 脚本 | 职责 |
|------|------|
| `apply-patches.sh` | 前端 patch（feature + crop）应用到临时副本 → tsc → vite → shim 注入 → `dist-fnos/`。内置 UI 精简验证（TitleBar/杀软裁剪 + padTop 断言）、feature 存在性验证（__FNOS__/replay） |
| `build-fpk.sh` | daemon musl 静态编译 → 组装 fnpack 项目 → `.fpk`（`--bundle` 出更新 bundle） |
| `gen-patch.sh` | patcher 相对基线生成 crop + feature 两个 patch + E6 完整性自检 + padding 配置化守护 |

**关键约束**：
- **daemon 必须 musl 静态编译**（fnOS 基底 Debian 12 glibc 2.36，gnu 动态链接加载即崩）——Windows 本地无法编译（缺 musl 工具链），由 CI（ubuntu runner + musl.cc 交叉工具链）构建。
- **dist 权限**：打包时 `chmod go-w` 收紧（777→755），否则 fnOS 市场升级保留 777 目录不替换 dist。
- OUTPUT_DIR 边界校验（拒绝 `/`、REPO_ROOT、src）+ 产物临时目录原子 mv（E1）；图标生成失败 exit 1（E7）；`cmd/main start` 校验二进制存在（E5）。
- 组合态要求：src 为应用态（patcher）+ fnos-api 来自 adapter + fnos-pack/fnos-shim/fnos-daemon 来自 mod——用 worktree 或手动 checkout 组合，三者齐全才能 gen-patch / apply-patches。

---

## 五、NAS 部署（真机操作）

> ⚠️ **NAS 只读优先**，写操作先征得同意，数据零接触。杀进程用精确 PID，禁止通配符。

### 升级安装（唯一安全路径）

```bash
# fpk 上传后（scp 到 /tmp/）
sudo appcenter-cli install-fpk /tmp/chmlfrp_X.Y.Z_x86.fpk --volume 1
# ⚠️ 必须 --volume 1（1 = 第一存储卷）
```

同版本升级时 `install-fpk` 幂等跳过 → 正确姿势 = 先备份数据目录 → 卸载 → 重装 → 还原备份。

**🛑 永远不要用 `install-local` 升级已装应用**——它先 uninstall 清空 `/vol1/@appdata/<app>/` 数据目录（8/13 事故根因）。

### 模拟更新源（自更新链路真机检验）

1. 上传 bundle 到 daemon 静态托管目录：`/vol1/@appcenter/chmlfrp/dist/`
2. 生成 `update-api.json`（GitHub API 形态：`tag_name` + `assets[] {name, browser_download_url, size}`），browser_download_url 指向 `http://127.0.0.1:17890/app/chmlfrp/<bundle>`（**/app/chmlfrp/ 前缀必须带**）
3. `cmd/main` 启动脚本注入 `export UPDATE_API_URL="http://127.0.0.1:${TRIM_SERVICE_PORT}/app/chmlfrp/update-api.json"`（daemon 启动前 export，重启生效）
4. `sudo appcenter-cli stop chmlfrp && sudo appcenter-cli start chmlfrp`
5. 验证：`curl http://127.0.0.1:17890/app/chmlfrp/api/update/check?force=1` 返回 `available:true`

### 关键路径

- 应用目录：`/vol1/@appcenter/chmlfrp/`（daemon + dist + app.sock，静态托管）
- 数据目录：`/vol1/@appdata/chmlfrp/`（frpc / 隧道配置 / 日志 / credential.json / node_settings.json，0600）
- 网关入口：`/app/chmlfrp`（nginx → unix socket 转发）；访问 `https://nas.letian.us.kg/app/chmlfrp`
- 应用权限：appcenter-cli 报 rpcbroker 权限错误时用 `sudo` 前缀执行

---

## 六、测试

| 层 | 位置/命令 | 数量（2.0.0 基线） |
|----|-----------|------|
| daemon (Rust) | `cd fnos-daemon && cargo test` | 76（含 update 异步下载/镜像退避/logfile 轮转/settings 持久化） |
| 前端 + shim (vitest) | `pnpm exec vitest run`（根目录，覆盖 src/ + fnos-shim/） | 54（含 shim-update-reload/async、shim-storage-redirect） |
| 构建链 (bash) | `bash fnos-pack/tests/lib.test.sh` | 9 断言 |
| 类型/风格 | `pnpm exec tsc --noEmit` / `pnpm exec eslint` | — |
| 契约校验 | `bash fnos-api/verify-adapter.sh fnos-api/adapters/v0.7.5/manifest.json` | 命令面 ⊆ 能力面 + uiConfig 键一致 |

**回归门**（发版前必跑）：cargo test + vitest + 组合态 `apply-patches.sh`（含 tsc + vite + shim 注入 + UI 验证）。

---

## 七、发版流程（已跑通，v0.7.5-2.0.0 为最新）

1. **各分支就绪**：patcher 前端改动提交 → gen-patch 重生成 → crop 提交 adapter / feature 提交 mod；版本号三个位点对齐（fpk manifest = Cargo.toml = package.json = patchSetVersion）
2. **回归全绿**：cargo test + vitest + 组合态 apply-patches 构建（见 §六）
3. **组合快照**：切 `main` → `git checkout adapter/v0.7.5 -- fnos-api` + `git checkout mod -- fnos-pack fnos-daemon fnos-shim` → `git add -f` 上述目录（**.gitignore 含 fnos-*，必须 -f**；**禁止 git add -f fnos-daemon 整目录——会带进 target/ 编译产物（134MB 超 GitHub 限制被拒的教训），只加 src + Cargo 文件**）→ commit「release: v0.7.5-X.Y.Z 组合快照」
4. **tag + push**：`git tag v0.7.5-X.Y.Z` → push main + tag（`--force` 处理 amend 后的 tag 更新）
5. **CI**：tag 触发 fnos-build.yml → 双架构 `.fpk` + `--bundle` → attach-bundles job 上传到 Release（Release 缺失时自动创建，正文是模板——**发版后人工补正式发行说明**：架构变更 + 具体 feat 清单 + 安装/校验说明，禁写含糊代际措辞）
6. **NAS 人工审验**：模拟源部署（§五）→ 用户实测更新链路 → 通过后方可对外宣称发布

---

## 八、遗留待办

| 项 | 状态 | 说明 |
|----|------|------|
| fnos-patchgen.yml 自动 PR | ⏳ | 需适配四分支（现只 PR patches/ 到 main，未含 adapter 的 fnos-api 变更；crop 需单独走 adapter 分支） |
| 镜像源列表 | ⏳ 待决策 | 已实测 ghproxy.com/mirror.ghproxy.com 死源，gh-proxy.com 可用——候选：移除死源 + 每源独立短超时 |
| daemon 自保 | ⏳ | 应用中心层面，daemon 崩溃无自拉起 |
| 上游 PR | ⏳ 待决策 | 桌面回归门控 __FNOS__ 是硬门槛 |