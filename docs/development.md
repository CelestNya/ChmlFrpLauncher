# ChmlFrpLauncher 开发文档（fnOS 移植）

> 最后更新：2026-08-15
> 本文件是**开发须知**：分支矩阵工作流、构建链、patch 机制、NAS 部署、测试、发版策略。
> 架构状态见 `docs/architecture.md`（反映代码状态）；交接状态见 `docs/handoff.md`（随代码更新）。
> 一次性实施计划（B1-B5）已归档，见 git 历史与临时目录，不再占仓库位置。

---

## 一、分支矩阵工作流（2026-08-15 用户决策）

**分支矩阵拓扑**：三套代码按生命周期分三组分支，构建时组合：

```
main ──────────── 发布组合态：src（0 侵入，= adapter 声明的上游基线）+ docs + CI + 根文件
adapter/<ver> ──── 每上游版本一个：fnos-api 契约（API.md + manifest + 校验/生成脚本）
                    adapterVersion = {基线版本}-{apiVersion}（如 0.7.5-1.0.0）
patch ──────────── fnOS 业务实现：patches/*.patch + manifest（featureVersion + requires.api）
                    + fnos-daemon + fnos-shim + 构建链（不含 src）
```

| 分支 | 内容 | 生命周期 |
|---|---|---|
| `main` | src/ 零改动（0 侵入）+ docs + CI | 发布组合态，随上游基线更新 |
| `adapter/v0.7.5`（每上游版本） | 仅 fnos-api/（4 文件） | 随上游版本生灭；`upstream.ref` 声明 src 基线 |
| `patch` | fnos-pack/ + fnos-daemon/ + fnos-shim/ | 随 feat 演进；featureVersion 独立 |

> **基线**：当前 LTS = 上游 v0.7.5 tag（2026-08-15 切回，develop 线暂停）。
> 0 侵入不变式：main 的 src/ 必须与 adapter manifest 声明的 `upstream.ref` 一致
> （CI 组合时断言 `git diff --quiet upstream/v0.7.5 -- src/`）。

### 组合操作（构建/开发的工作区形态）

```bash
# 本地组合（main 工作区 = 完整构建环境）
git checkout main
git checkout adapter/v0.7.5 -- fnos-api/
git checkout patch -- fnos-pack/ fnos-daemon/ fnos-shim/
# 这些目录被 main 的 .gitignore 忽略，不会误提交

# 验证组合（CI 同款）
bash fnos-api/verify-adapter.sh   # 命令面 + uiConfig 键 + patch 依赖校验
```

### 日常开发流程（改 fnOS 功能 → patch 分支）

1. 组合工作区下改前端（src/ 应用态）或 daemon/shim
2. 重新生成 patch（patch 分支不含 src，diff 基线 = adapter 声明的上游 ref）：
   `BASE=v0.7.5 bash fnos-pack/gen-patch.sh`（工作区 src = 上游 + 改动）
3. 只提交 patch 分支的产物（fnos-pack/patches/ + daemon + shim），**不提交 src/**
4. `git push origin patch` → fnos-build CI 组合验证（patch 依赖校验强制）

### 上游同步（主仓更新时）

- **LTS 冻结**：当前基线 v0.7.5 tag 不可变 → upstream-follow CI 常态静默（保留 hotfix 感知）
- **上游发新版（如 v0.8.0）**：新开 `adapter/v0.8.0` 分支（从 v0.7.5 分叉）→ diff 命令面/UI 适配点 → 更新能力面 → main src/ 切新基线 → CI 选新组合构建
- **上游改动 patch 涉及文件** → apply-patches 的 dry-run 预检明确报错 → 在组合工作区人工合并后重新 gen-patch

### 版本号规范（架构.功能.修复，2026-08-16 定版）

- **patchSetVersion**（`fnos-pack/patches/manifest.json`）：架构（破坏性重构）.功能（新能力代）.修复（bug）
- **E4 联合版本号** = app 版本 + patchSetVersion（如 `0.7.5-1.5.2`），同步四处：package.json / Cargo.toml / Cargo.lock / fpk manifest
- **bump 必须带 commit 痕迹**：commit 标题标注新版本号（如 `feat(patch): 1.6.0 新增 XX`），并更新 manifest 的 versionHistory
- 演进历史见 manifest `versionHistory` 字段

### 版本矩阵（CI 构建）

`fnos-build.yml` workflow_dispatch：选 `adapter` 分支 + `patch` 分支 + 架构 → 组合构建。
- push main / push patch 均触发默认组合（adapter/v0.7.5 + patch）验证构建
- 构建内强制：0 侵入验证 → verify-adapter（patch requires.api ≤ adapter apiVersion）→ apply-patches → build-fpk

---

## 二、Patch 机制

```
fnos-pack/patches/
├── fnos-ui-patch.patch        # UI 裁剪：删 TitleBar/杀软弹窗/自启/托盘（4 文件）
└── fnos-feature-patch.patch   # fnOS 功能：壁纸/日志/通知/音效/replay/守护（11 文件）
```

两个 patch 都是**全量**（从干净上游可应用），由 `gen-patch.sh` 在组合工作区（src = 上游基线 + fnOS 改动）生成，幂等覆盖；产物提交在 patch 分支。

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

### fnOS API 版本化 + adapter（ADR-0005，2026-08-13）

**三层解耦**：上游程序（服务面，不可控）→ adapter（每上游版本一个）→ API 契约（fnos-api，可控、版本化）→ patch 业务逻辑（对 API 版本硬依赖）。

```
fnos-api/
├── API.md                        # 契约人读版（六面：命令/事件/插件/存储/HTTP/UI），apiVersion 1.0.0
├── verify-adapter.sh             # 能力覆盖校验：前端 invoke 调用 ⊆ adapter 能力面
└── adapters/
    └── v0.7.5/
        └── manifest.json         # 第一个 adapter（adapterVersion 1.0.0 = apiVersion）
```

**版本化规则**：

| 对象 | 版本号语义 | 依赖 |
|------|-----------|------|
| API | 接口演进 semver（major=破坏 / minor=新增 / patch=修复） | — |
| adapter | **= API 版本号**（声明适配的 API），每上游版本一个 | 上游版本 |
| patch | 功能版本 | `requires: {api: ">=X"}` 硬依赖 |

**开发流程（patch 有新需求时）**：
1. 先更新 `fnos-api/API.md`（加接口/改契约）→ bump API 版本
2. adapter 适配新 API（对应上游版本）→ bump adapterVersion = apiVersion
3. patch 基于新 API 开发 → manifest 声明 `requires: {api: ">=新版本"}`
4. `bash fnos-api/verify-adapter.sh` 校验：前端调用命令 ⊆ adapter 能力面
5. 上游出新版本 → 新增 adapter（命令映射复用，差异登记）；上游内部实现改动（如 token 刷新）→ 登记差异但**不 bump API**

> 关键原则：**上游更新 ≠ API 更新**——只有「上游改动影响接口形态」才 bump API。
> 事实核查（2026-08-13）：0.7.5 与 0.8.0-dev 命令面完全一致（45 命令 diff 零），adapter 命令映射跨版本零工作量。

---

## 三、构建链（fnos-pack/）

| 脚本 | 职责 |
|------|------|
| `apply-patches.sh` | 前端 patch 应用到临时副本 → tsc → vite → shim 注入 → `dist-fnos/` |
| `build-fpk.sh` | daemon musl 静态编译 → 组装 fnpack 项目 → `.fpk`（可 `--bundle` 出更新包） |
| `gen-patch.sh` | 组合工作区相对上游基线生成两个 patch + E6 完整性自检 |

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
# ⚠️ 必须 --volume 1（1 = 第一存储卷）
```

**版本号与上游同步，不 bump**（用户定论 2026-08-13）：同版本升级时 `install-fpk` 幂等跳过
（`Application is installed`，只有全新安装才真正执行）→ 正确姿势 = **先卸载再装**：

```bash
# 1. 备份数据目录（frpc 二进制 / g_*.ini / 日志）到共享文件夹
sudo cp -r /vol1/@appdata/chmlfrp /vol1/1000/1/chmlfrp-backup-$(date +%Y%m%d)
# 2. 卸载（不清数据备份）
sudo appcenter-cli uninstall chmlfrp
# 3. 重装
sudo appcenter-cli install-fpk /tmp/chmlfrp_X.Y.Z_x86.fpk --volume 1
# 4. 验证后还原备份（frpc / 隧道配置）
```

> 已在真机验证（checklist 归档）：备份 → uninstall success → install complete → 命令全可用。

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
| daemon (Rust) | `cd fnos-daemon && cargo test` | 66（含 settings/background/capabilities 新测试 + 联合版本号断言） |
| 前端 (vitest) | `pnpm exec vitest run` | 45（shim 27 + src 18） |
| 构建链 (bash) | `bash fnos-pack/tests/lib.test.sh` | 9 断言 |
| 类型/风格 | `pnpm exec tsc --noEmit` / `pnpm exec eslint` | — |

**NAS 人工测试**：见 `docs/plans/fnos-nas-test-checklist.md`（已归档验证结果——守护重启、WS 重连去重、日志导出/清空、B2 重放、socket 0600）。

---

## 六、发版策略（2026-08-16 定版）

**版本号**：E4 联合号 `app-patchSetVersion`（如 `0.7.5-1.5.2`），语义见 §一「版本号规范」。

**单线版本策略**（用户定论）：任何时候只有一条活跃线——0.8.0 release 前维护 `0.7.5+patch`；0.8.0 release 后停止 0.7.5 线维护，最快速度出 0.8.0 patch。检测更新永远面对一条线，联合号不会造成新旧混乱（daemon `version_compare` 分段比较已验证：同线 patch 升级与跨线切换均正确）。

**发布面**（fork 自治，用户定论）：

- fork **不构建桌面版**（release.yml push 触发已关闭，仅保留手动兜底）——桌面用户走上游 release；fork 的 latest 只有 fnOS release，不会被抢占
- daemon 更新检查直接扫 **fork `releases/latest`**（`CelestNya/ChmlFrpLauncher`），asset 精确匹配 `chmlfrp-fnos-{version}-{platform}.tar.gz`；前端「检测更新」（shim 转发 `/api/update/check`）与 daemon 自更新共用 `fetch_latest`，同一个源
- 发版 tag：**`v<联合号>`**（如 `v0.7.5-1.5.2`），与上游 v* 风格一致；CI 触发 `v*-*` 按「带连字符」隔离——上游 v* tags（v0.7.5）无连字符，进不来
- 上游 15 个 tags 不删不动：无 release 的 tag 不出现在 Releases 页，删了反而破坏上游对照；同步上游**只 fetch 不 push --tags**

**发版 checklist**：

1. 实测稳定确认 → 在 main 打 tag：`git tag v<联合号> && git push origin v<联合号>`
2. CI（fnos-build.yml `v*-*` 触发）：双架构构建 → fpk 完整性验证 → attach-bundles 建 release + 附加 bundle
3. 验证 release：Releases 页确认 bundle asset（x86/arm）齐全、tag 名为 `v<联合号>`
4. NAS 实测自更新：守护调 `/api/update/check` 应返回新版本 → 下载 → 应用

**已发布组合**：NAS 上运行 0.7.5 fpk（含 padding + 掉登录修复；联合号定版前构建，功能一致）。

---

## 七、遗留待办

| 项 | 状态 | 说明 |
|----|------|------|
| 自更新通道 | ⏳ 等首次发版 | 扫 fork latest（fork 无桌面版不会被抢占），随 v0.7.5-1.5.2 发版启用 |
| daemon 自保 | ⏳ | 应用中心层面，daemon 崩溃无自拉起 |
| OAuth CORS | ⏳ | iframe 下可能失败（本次实测正常） |
| UI 适配 | ⏳ | 标题栏裁切等 |
| daemon 中-11 WS seq | ⏳ | 前端 B7 已兜底去重，daemon 侧 seq 方案未做 |
| 前端 M3 IndexedDB | ⏳ | localStorage 配额溢出防护未做 |
| 上游 PR | ⏳ 待决策 | 六批修复 PR 回上游（桌面回归门控 __FNOS__ 是硬门槛） |
