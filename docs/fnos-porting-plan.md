# fnOS 移植整体开发 Plan

> 最后更新：2026-08-11
> 状态：第 ② 步已定稿（grill Q1-Q5），进入第 ③ 步逐批 grill。
> 关联文档：[architecture.md](architecture.md)；fnOS 文档调研结论见项目记忆 `fnos-porting-assessment`。

## 一、目标

把 ChmlFrpLauncher 移植到 fnOS（飞牛 NAS），以**形态 C（零改动补丁方案）**交付：

- 现有源码（`src/` + `src-tauri/src/`）**零改动**，git 只增不改
- 前端 `dist/` 产物复用；构建期 patch 精简 UI；运行时 shim 模拟 Tauri API
- 新增独立 `fnos-daemon` crate（axum），承载 frpc 管理与隧道服务
- 打包 `.fpk`（fnpack）+ CI 自动构建 x86_64 / arm64 双架构
- 软件内自更新（target 目录文件替换机制）

## 二、架构决策记录（grill 决议）

| # | 决策 | 决议 |
|---|------|------|
| Q1 | 分批原则 | 按依赖纵向切片 + 契约先行（B1 定 API 契约后 B2/B3 可并行） |
| Q2 | 代码组织 | **形态 C**：现有代码零改动；新增 daemon crate + shim + patch，CI 组合构建 |
| Q3 | invoke 契约 | **通用透传** `POST /api/invoke {cmd, args}`，命令面 = 现有 45 命令，不扩展 |
| Q4 | UI 精简 | patch 删：开机自启设置/关闭到托盘/显示顶部栏/TitleBar/杀软警告/深链接提示；**保留**：隧道管理全流程、每隧道自动启动、进程守护、网络 4 开关、日志、更新、**全部个性化（背景/毛玻璃/音效/主题）**、首页 |
| Q5 | 网络形态 | **统一网关优先**（复用 NAS 登录态 + TLS 终结 + WS 支持，公网域名可访问）；daemon 只监听 127.0.0.1；鉴权中间件可插拔（网关模式读转发 Header / token 模式仅开发用） |

## 三、批次框架

### B1 — fnos-daemon（Rust crate，可独立推进）

- 新增 `fnos-daemon/`（或 `src-tauri` 同级独立 crate）：axum + tokio
- 路由：`POST /api/invoke`（通用透传，serde camelCase 参数，同 Tauri 语义）、`WS /ws/logs`（frpc-log / download-progress / tunnel-auto-restarted）、静态托管（SPA fallback → index.html）、`/api/bootstrap`（版本号）
- 业务重写（照 `src-tauri/src/commands/` 现有实现为规格）：frpc 启停（配置生成/日志管道/脱敏/PID 持久化/孤儿回收）、守护轮询（3s + STOP_GUARD_PATTERNS）、自定义隧道、frpc 下载（sha256/续传）、`http_request` 代理、DNS 解析、ping/端口检测
- 鉴权中间件：网关 Header 校验（预留）/ token 模式（开发用）
- 数据目录：环境变量驱动（TRIM_PKGVAR 优先，开发默认本地目录）；frpc 二进制与配置落 data 目录
- **验收**：`cargo build` ✅；curl 全流程（启动隧道→日志 WS→停止→守护重启）；桌面版零影响

### B2 — fnos-shim + patch（前端适配，依赖 B1 契约）

- 新增 `fnos-shim/tauri-shim.ts`：模拟 `window.__TAURI_INTERNALS__`——invoke 转发 `/api/invoke`、listen 桥接 WS、`getVersion` 返回 daemon 版本、updater no-op（自更新走 daemon 接口）、openUrl 降级 `window.open`、窗口类 API no-op
- 新增 `fnos-pack/patches/*.patch`（Q4 清单）+ `apply-patches.sh`（临时目录副本应用，CI 与本地可复用）
- **验收**：浏览器直开 daemon 页面，登录（设备码）→ 隧道全流程 → 日志实时 → 设置开关可操作；patch 后 UI 与 Q4 清单一致

### B3 — fnos-pack（.fpk 打包，依赖 B1 产物，可与 B2 并行）

- 新增 `fnos-pack/`：manifest（`service_port`、`platform`、`ctl_stop`）、`config/privilege`（run-as=package）、`cmd/main`（start/stop/status 拉起 daemon）、`app/ui/config`（**网关模式入口**，type=iframe）、图标、生命周期脚本
- 双架构：x86_64 + aarch64 交叉编译
- **验收**：fnpack build 通过；测试设备安装/启动/停止/升级/卸载全流程
- **实测清单**（本批核心交付）：① fnOS 应用是否随系统启动；② OAuth 设备码 CORS；③ 统一网关注册细节（gatewayPrefix/gatewaySocket/SPA fallback/公网域名访问）；④ target 目录写权限（run-as=package）；⑤ frpc Linux arm64 二进制可得性

### B4 — CI 构建（主仓 workflow）

- 新增 job（ubuntu runner）：checkout → pnpm install → **patch 应用到临时副本 → vite build** → **注入 shim** → cargo build daemon（x86_64 + aarch64 交叉）→ fnpack build → .fpk 产物（附件到 Release / artifact）
- 桌面版现有 workflow 不动
- **验收**：Actions 全绿，.fpk 在测试设备可装

### B5 — 自更新 + 打磨

- daemon `/api/update`：检查远端版本 → 流式下载新 daemon/前端资源 → sha256 校验 → 原子替换 target 目录文件 → 重启生效
- UI 打磨（NAS 文案）、错误提示、首启引导
- **验收**：设备上升级链路完整（v1 → v2）

## 四、关键技术要点

- **invoke 透传契约**：`{cmd: string, args: Record<string, unknown>}` → daemon handler 表（命令名 ↔ 实现），参数 `#[serde(rename_all = "camelCase")]`；返回值 JSON 序列化，错误统一 `{error: string}`；`listen` 事件名/载荷与桌面版一致
- **shim 注入**：构建产物 `index.html` 追加 `<script>` 标签引用 shim（构建期处理），shim 挂 `window.__TAURI_INTERNALS__`
- **安全**：daemon 只监听 127.0.0.1（网关转发）；日志脱敏逻辑照抄不简化（安全线）；frpc 配置 0o600
- **版本一致性**：自定义隧道 id 哈希（string_to_i32）算法保持；MutexGuard drop 顺序约束保持

## 五、待验证清单（B3 集中实测）

| # | 项 | 影响 |
|---|----|------|
| 1 | fnOS 应用中心是否随系统启动应用 | 隧道自启链路（无则需备选：cmd/main 注册或提示用户） |
| 2 | OAuth 设备码端点 CORS（iframe 环境） | 登录流程 |
| 3 | 统一网关：注册方式 / WebSocket / SPA fallback / 公网域名 | 入口形态（A 方案成立性） |
| 4 | target 目录写权限（run-as=package） | 自更新落地位置（兜底 TRIM_PKGVAR） |
| 5 | frpc Linux arm64 二进制 | arm64 发布范围 |

## 六、风险与对策

| 风险 | 对策 |
|------|------|
| 网关对 SPA fallback / WS 支持不满足预期 | 退 B 模式（service_port + token），daemon 鉴权中间件已可插拔 |
| patch 随前端迭代失配 | patch 按文件分片 + CI `git apply --check` 预检 + 本地可试跑 |
| daemon 与桌面版逻辑双份（分叉） | 以桌面版为规格源，B5 后评估是否抽 core 收敛（可选演进，不阻塞） |
| arm64 frpc 缺失 | 先发 x86_64，arm64 版待官方资源就绪 |