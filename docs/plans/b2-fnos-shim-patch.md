# B2 实施计划：fnos-shim + patch（✅ 已完成并验收）

> 状态：2026-08-11 全部交付。B1→B2 顺序按用户修正（shim + patch 先于打包，.fpk 需完整前端）。
> 遗留：真机/浏览器端全流程冒烟（登录 → 隧道 → 日志）待 WSL 或 fnOS 真机验证（见"后续"）。

## 形态与契约
- **形态 C 零改动**：`src/` + `src-tauri/` 零字节变动；git 只增不改（本批全部为新增文件）
- **构建链**：`apply-patches.sh`（源码级 patch → tsc → vite build → shim 注入）→ 产物 `dist-fnos/`
- **shim 覆盖面**：以 `src/` 实际 import 为准（core / event / window / app / plugin-dialog / plugin-opener / plugin-fs / plugin-updater），新增 Tauri 调用需同步补
- **事件**：frpc-log / download-progress / tunnel-auto-restarted（daemon `/ws/logs` 同源推送）

## 交付物
| 文件 | 说明 |
|------|------|
| `fnos-shim/tauri-shim.ts` | 运行时模拟层：IIFE，注入 dist/index.html；invoke→POST /api/invoke（{ok,data,error} 解包）；listen→WS /ws/logs 分发；插件命令按 fnOS 降级（window 状态返回布尔/尺寸、updater check 返回 null、dialog save→下载、open_url→window.open 等） |
| `fnos-shim/build-shim.ts` | 构建期注入脚本：esbuild（.pnpm 内定位 CLI）编译为 ES2017 → 注入 `</body>` 前普通 `<script>`（同步早于 module bundle）；幂等 |
| `fnos-pack/patches/fnos-ui-patch.patch` | 源码级 unified diff，4 文件 19 hunk，`patch -p1` 可应用 |
| `fnos-pack/apply-patches.sh` | 全链脚本：临时副本 + dry-run 预检 + patch + 验证 + tsc + vite + shim 注入，产物到 `dist-fnos/`（CI 与本地复用） |

## patch 覆盖（Q4 清单，对照当前源码）
删除（fnOS 无桌面窗口/托盘语义）：
- `App.tsx`：TitleBar/WindowControls 渲染与 import、useTitleBar、杀软警告（AntivirusWarningDialog 渲染与 import；hook 保留以维持 frpc 自动下载兜底）、平台判断变量、标题栏相关布局常量与占位
- `Settings/index.tsx`：useAutostart/useCloseBehavior 接线与 import、isMacOS/isWindows 变量、Appearance/System 两节被删 props 的传参
- `AppearanceSection.tsx`：显示顶部栏开关（含 interface/解构）
- `SystemSection.tsx`：开机自启、关闭窗口时最小化到托盘（含 interface/解构）

保留（全部个性化 + 隧道全流程）：主题/背景/毛玻璃/音效/侧边栏样式/轮播、进程守护、Frpc 日志等级、修改后重启、网络 4 开关、自动检测更新、隧道 CRUD 与启停、首页登录（设备码）。

> 注：原计划中"深链接提示 DeepLinkPrompt"在当前源码中已不存在（grep 无此组件），不纳入 patch。

## 关键约束与实现要点
- **tsconfig 严格模式**（noUnusedLocals/noUnusedParameters）：删 JSX 必须同步删 interface/解构/接线，否则 TS6133/TS2322 报错——已按此处理并零错误通过
- **shim 不设 `__TAURI__` 之外的假分支**：`"__TAURI__" in window` 为 true 时 api.ts 走 http_request 代理（daemon 白名单 cf-v2.uapis.cn / account-api.qzhua.net）；OAuth 设备码为裸 fetch，跨域 CORS 待实测
- **esbuild CLI**：pnpm store 不 hoist 且 bin link 缺失，脚本直接定位 `.pnpm/esbuild@*/node_modules/esbuild/bin/esbuild`
- **WS 同源**：shim 默认 `location.host/ws/logs`，daemon 与 SPA 同源部署；预留 `__FNOS_WS_URL__` 覆盖

## 验收结果（已执行）
1. ✅ `apply-patches.sh` 全链 exit 0：patch dry-run 通过 → 应用 → UI 验证 → tsc 零错误 → vite build 成功 → shim 注入
2. ✅ 产物 `dist-fnos/index.html` 含注入 shim（`__TAURI_INTERNALS__` 存在，vm 冒烟通过：invoke/listen/版本/window/updater）
3. ✅ 产物 JS 文案检查：开机自启/关闭到托盘/显示顶部栏 已移除；主题/毛玻璃/背景/进程守护 保留
4. ✅ 工作树 `src/` 零改动（patch 只作用于临时副本）；`.gitignore` 增补 `dist-fnos/`
5. ✅ 残留清理：`test-b2.js`/`test-b2.sh`/无效 `pnpm-workspace.yaml` 已删

## 后续（WSL/真机冒烟）
- WSL 起 daemon（Linux 二进制）→ `DAEMON_WEB_DIR=dist-fnos` 静态托管 → Windows 浏览器访问：登录（设备码，验证 OAuth 跨域）、隧道全流程、WS 日志实时、设置开关
- OAuth 设备码若因 CORS 失败：patch 增补 api.ts 走 daemon 代理（http_request 白名单已含 account-api.qzhua.net）

## 风险提示
- patch 随前端迭代失配 → `apply-patches.sh` dry-run 预检会失败并提示更新 patch
- shim 覆盖面以现有调用面为准，前端新增 Tauri 调用需同步补
- 背景图选择（dialog open 返回 null）在浏览器形态受限，属已知降级（daemon 侧 copy_background_* 为 NO_OP，B5 打磨项）
