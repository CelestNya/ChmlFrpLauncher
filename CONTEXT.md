# ChmlFrpLauncher 领域模型

ChmlFrpLauncher 是一个 Tauri 2 + React 桌面应用，并移植到 fnOS NAS。本模型定义
「隧道服务」领域的前后端边界词汇。重构方向：数据后端化（token/行为配置离浏览器
localStorage，落 daemon 文件），能力协商替代硬编码门控。

## Language

### 凭据与身份

**Credential**:
用户的登录态集合（accessToken / refreshToken / usertoken），由 OAuth 设备码流程
产出。是**后端生成 frpc 配置的输入**，必须由后端持有（daemon `credential.json`）。
_Avoid_: 用户、账号、token、登录信息

**CredentialStatus**:
Credential 的存在性只读视图（`{logged_in: bool}`），供前端判断登录态；**绝不包含
token 本身**。
_Avoid_: 登录状态

### 隧道与进程

**Tunnel**:
用户在官网创建的隧道。以 `g_{id}.ini` 为后端产物，frpc 按此运行。
_Avoid_: 配置、服务、通道

**CustomTunnel**:
用户自写 ini 的隧道。数据存后端 `custom_tunnels.json`（桌面 `app_data_dir` /
fnOS `TRIM_PKGVAR`）。
_Avoid_: 自定义配置

**TunnelInstance**:
某个 Tunnel 对应的 frpc 进程实例，有 PID / 运行状态 / 守护归属。生命周期由后端
管理（`running_tunnels.json` 持久化，PID + start_time 身份校验防复用误杀）。
_Avoid_: 进程、frpc

**TunnelConfig**:
后端生成 `g_*.ini` 所需的完整配置快照（server/端口/token/代理/日志级别等）。
fnOS 守护恢复依赖此快照（daemon 中-8）。

### 配置与偏好

**NodeSetting**:
影响 frpc 启动行为或后端出网的配置（代理、日志级别、bypassProxy、restartOnEdit）。
由后端持有（`node_settings.json`），不落浏览器持久化。
_Avoid_: 代理配置、设置、偏好

**Preference**:
纯 UI / 设备级外观偏好（主题、音效、毛玻璃、侧栏模式）。可留前端 localStorage，
但需与业务数据分离（白名单）。
_Avoid_: 配置、设置

### 能力

**Capability**:
后端实际支持的命令 / 特性集合。前端据此自适应，取代硬编码的 `__FNOS__` 环境门控。
daemon 经 `GET /api/capabilities` 暴露，shim 启动时探测缓存。
_Avoid_: 功能、特性、feature flag

### 适配层（ADR-0005）

**ApiContract**:
fnOS 适配面的稳定接口契约（`fnos-api/API.md`），六面：命令 / 事件 / 插件 / 存储 / HTTP / UI。
是**一等公民**，版本化（apiVersion，semver：major=破坏 / minor=新增 / patch=修复）。
_Avoid_: API、接口、定义

**Adapter**:
连接上游程序（具体 chmlfrp 版本）与 ApiContract 的映射层，每上游版本一个
（`fnos-api/adapters/<version>/`）。**adapter 版本号 = API 版本号**，声明其适配的 API 版本与上游版本。
上游内部实现改动（如 token 刷新）只登记差异，**不 bump API**。
_Avoid_: mod、插件、适配器实现

**Patch**:
fnOS 业务逻辑（`fnos-pack/patches/`），对 ApiContract 硬依赖
（manifest 声明 `requires: {api: ">=X"}`）。版本号代表功能演进，与上游版本解耦。
_Avoid_: diff、补丁文件

## Rules

- **凭据与偏好是两极**：凭据必须后端持有，偏好可留前端；居中者是 NodeSetting（归后端）。
- **daemon 是唯一权威**：token / 代理 / 日志级别等影响后端的配置，daemon 从自己文件读，
  不依赖前端传入（契约参数保留但可选，shim 重定向把 localStorage 读转到 daemon 存储）。
- **渲染类偏好留守前端**：主题 / 壁纸 / 模糊 / 遮罩是首帧同步惰性初始化
  （`useBackground.ts:33`、`useAppTheme.ts:31`），移后端会首帧闪烁，留守 localStorage。
- **capabilities 是契约**：命令面 = 能力面，daemon 无 NO_OP「假装支持」；不支持的命令
  返回明确错误，前端按能力自适应。
- **API 是一等公民**（ADR-0005）：新需求先更新 ApiContract 并 bump → adapter 适配（版本号 =
  API 版本号）→ patch 基于新 API 开发并声明硬依赖。上游更新 ≠ API 更新，仅接口形态变化才 bump。
