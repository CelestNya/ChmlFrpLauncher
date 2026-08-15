# fnOS API 契约（第一版）

> **API 是什么**：fnOS 适配面的**接口契约层**——连接「上游 ChmlFrpLauncher（服务面）」与「fnOS 适配（adapter/patch/daemon）」的稳定接口。
> **版本化**：API 版本 = 接口演进版本（semver）。adapter 版本号 = **联合版本号**（`{上游基线版本}-{apiVersion}`）。
> **地位**：API 是唯一权威契约。上游程序通过 adapter 映射到 API；patch 业务逻辑硬依赖 API 版本。
>
> 本文档是契约的**人读版**；机器校验用 `fnos-api/manifest.json`。

## 一、API 版本

```
apiVersion: 1.0.0
状态: draft（从当前 patcher 需求提取，待 adapter v0.7.5 落地验证）
```

**semver 语义**：
- **major**：接口破坏性变更（命令删改/参数形态变）→ 旧 patch 失效
- **minor**：新增接口（向后兼容）→ 旧 patch 仍可用
- **patch**：契约文档/校验修复（无接口变化）

## 二、接口面总览

| 面 | 内容 | 稳定度 |
|----|------|--------|
| 1. 命令面 | `invoke(cmd, camelCaseArgs) → {ok, data, error}` | ✅ 稳定（45 命令跨 0.7.5/0.8.0 一致）|
| 2. 事件面 | 三事件：frpc-log / download-progress / tunnel-auto-restarted | ✅ 稳定 |
| 3. 插件面 | dialog / fs / opener / updater / window / app / event | ⚠️ 脆弱（action 名是 Tauri 内部细节）|
| 4. 存储面 | 5 白名单 key 的 snake↔camel 往返 | ✅ 稳定 |
| 5. HTTP 面 | /api/invoke、/api/capabilities、/ws/logs、/assets/backgrounds | ✅ 稳定 |
| 6. UI 适配面 | 需要 fnOS 版的文件清单 + 改动性质 | ⚠️ 脆弱（依赖上游 UI 结构）|

## 三、命令面（45 命令 + fnOS 扩展）

### 3.1 上游命令（45，跨 0.7.5/0.8.0 一致）

**frpc 进程**（9）：`start_frpc`(config) / `stop_frpc`(tunnelId) / `is_frpc_running`(tunnelId) / `get_running_tunnels`() / `get_persisted_running_tunnels`() / `stop_orphan_process`(tunnelId) / `is_tunnel_process_alive`(tunnelId) / `fix_frpc_ini_tls`() / `get_frpc_directory`()

**守护**（6）：`set_process_guard_enabled`(enabled) / `get_process_guard_enabled`() / `add_guarded_process`(tunnelId, config) / `add_guarded_custom_tunnel`(tunnelId, originalId) / `remove_guarded_process`(tunnelId, isManualStop) / `check_log_and_stop_guard`(tunnelId, logMessage)

**自定义隧道**（8）：`save_custom_tunnel` / `get_custom_tunnels` / `get_custom_tunnel_config` / `update_custom_tunnel` / `delete_custom_tunnel` / `start_custom_tunnel` / `stop_custom_tunnel` / `is_custom_tunnel_running`

**frpc 下载**（4）：`download_frpc` / `check_frpc_exists` / `get_download_url` / `get_frpc_directory`

**网络/HTTP**（6）：`ping_host`(host) / `get_ports` / `check_local_port`(port) / `resolve_domain_to_ip`(domain) / `http_request`(options) / `http_request_raw`(options)

**桌面专属（NO_OP）**（9）：`is_autostart_enabled` / `set_autostart` / `get_tunnel_auto_start` / `set_tunnel_auto_start` / `hide_window` / `show_window` / `quit_app` / `read_image_folder` / `fix_frpc_ini_tls`

### 3.2 fnOS 扩展命令（daemon 独有）

**凭据**（4）：`save_credential`(credential) / `get_credential` / `clear_credential` / `credential_status`

**节点设置**（2）：`get_node_settings` / `set_node_settings`(settings)

**壁纸**（5）：`copy_background_image`(sourcePath) / `copy_background_video`(sourcePath) / `save_background_image`(dataUrl, fileName) / `import_background_image_folder`(dirPath) / `get_background_video_path`

**日志**（1）：`clear_log_history`

**能力协商**（1）：`capabilities`（仅 GET /api/capabilities，不注册 invoke）

### 3.3 参数形态（camelCase，与 Tauri IPC 一致）

```json
// start_frpc 的 TunnelConfig（跨 0.7.5/0.8.0 一致）
{
  "tunnelId": 1, "tunnelName": "t", "userToken": "...", "serverAddr": "...",
  "serverPort": 7000, "nodeToken": "...", "tunnelType": "tcp",
  "localIp": "127.0.0.1", "localPort": 80, "remotePort": null,
  "customDomains": null, "httpProxy": null, "logLevel": "info",
  "forceTls": false, "kcpOptimization": false
}
```

## 四、事件面（3 事件，跨端对齐）

| 事件 | payload | 生产方 | 消费方 |
|------|---------|--------|--------|
| `frpc-log` | `{tunnel_id, message, timestamp}` | daemon events.rs + WS | frpcManager.listenToLogs → logStore |
| `download-progress` | `{downloaded, total, percentage}` | daemon download.rs | frpcDownloader（Channel）|
| `tunnel-auto-restarted` | `{tunnel_id, timestamp}` | daemon guard.rs | useTunnelList |

**fnOS 扩展字段**：`replay: true` 标记（WS 断线补发）——已升级为适配接口（前端 patch 消费）。

## 五、存储面（5 白名单 key，snake↔camel 往返）

| localStorage key | daemon 结构 | 往返必须完整（P2 教训）|
|------------------|-------------|----------------------|
| `chmlfrp_user` | `Credential`（username/usertoken/access_token/refresh_token/expires_at/token_type/**usergroup/userimg/tunnel_count/tunnel**）| ⚠️ usergroup 丢失会破坏会员门控 |
| `frpc_proxy_config` | `NodeSettings.proxy_*`（8 字段）| 含明文密码，0600 |
| `frpcLogLevel` | `NodeSettings.log_level` | — |
| `bypassProxy` | `NodeSettings.bypass_proxy` | — |
| `restartOnEdit` | `NodeSettings.restart_on_edit` | — |

## 六、插件面（shim 覆盖范围，脆弱需登记）

| 插件 | action | 参数形态 | 降级行为 |
|------|--------|---------|---------|
| dialog | open / save / message / ask / confirm | `{options:{filters,defaultPath}}` | 浏览器文件框 / 下载 |
| fs | write_text_file | 三参：`(cmd, Uint8Array, {headers:{path}})` | 下载文件 |
| opener | open_url | `{url}` | window.open |
| updater | check / download_and_install | — | daemon /api/update/* |
| window | listen / is_* / size / position | — | no-op / 默认值 |
| app | version / name | — | /api/bootstrap |
| event | listen / unlisten / emit | `{event, handler}` | WS 桥接 |

**脆弱点**：action 名是 @tauri-apps/api 内部约定（open_url/write_text_file/register_listener），上游升级 Tauri SDK 可能变 → adapter 需登记「该版本实际用到的 action 名」。

## 七、HTTP 面（fnOS 网关前缀 /app/chmlfrp）

| 路由 | 方法 | 用途 |
|------|------|------|
| /api/invoke | POST | 命令透传 |
| /api/capabilities | GET | 能力协商（SUPPORTED_COMMANDS）|
| /api/bootstrap | GET | 版本信息 |
| /ws/logs | GET | 事件流（WS）|
| /api/update/check\|download\|apply | GET/POST | 自更新 |
| /assets/backgrounds/* | GET | 壁纸静态托管 |

## 八、UI 适配面（12 文件清单，脆弱）

见 `fnos-pack/adapters/<version>/manifest.json` 的 `uiFiles` 字段（每个 adapter 声明自己需要覆盖哪些文件）。

### 8.1 UI 裁剪配置（uiConfig，统一格式）

**adapter 做裁剪实现**（2026-08-15 用户决策）：manifest 的 `uiConfig.values` 暴露统一格式配置项，
patch 代码消费配置做**条件渲染**（不再硬删代码）。patch 因 feat 增删配置只影响「引用哪些配置项」，
不改 adapter 格式（统一格式先包圆）。

| 配置项 | 控制的 UI | 当前值 (v0.7.5) |
|--------|-----------|-----------------|
| `titleBar` | App.tsx TitleBar/WindowControls + 显示顶部栏设置项 | `false` |
| `antivirusWarningDialog` | 杀软警告弹窗 | `false` |
| `autostart` | 开机自启设置项 | `false` |
| `closeToTray` | 关闭时最小化到托盘设置项 | `false` |
| `backgroundFolderImport` | 背景「选择文件夹」按钮 | `false` |

**消费链路**：
```
adapter manifest uiConfig.values（唯一权威）
    ↓ fnos-api/generate-ui-config.mjs（构建时生成）
src/fnos-ui-config.ts（export const uiConfig = {...} as const）
    ↓ patch 代码 import + 条件渲染（{uiConfig.titleBar && <TitleBar/>}）
产物 dist
```

**约束**：
- 新增裁剪点：manifest `uiConfig.values` 加键 → patch 消费 → `verify-adapter.sh` 校验（引用键 ⊆ 定义键）
- `src/fnos-ui-config.ts` 是生成文件（E6 豁免不进 patch、不进 main，构建时生成覆盖 patcher 手写版）
- 手写版与 manifest 不一致 → verify-adapter.sh 报错（manifest 唯一权威）

## 九、契约维护规则

1. **API 变更必须 bump 版本**（major=破坏/minor=新增/patch=修复）
2. **adapter 版本号 = 联合版本号**（`{上游基线版本}-{apiVersion}`）：基线变（如 0.7.5→0.8.0）或 API 变都反映在 adapterVersion
3. **patch 特性版本号**（`fnos-pack/patches/manifest.json`）：单特性版本，与 API 版本独立演进；只声明 `requires: {api: ">=X"}` 硬依赖，构建期校验
4. **新需求流程**：更新 API + bump → adapter 适配 → patch 基于新 API 开发（bump featureVersion）→ 校验
5. **校验**：patch 需要的接口 ⊆ adapter 提供的接口（构建期，semver 粗筛 + 清单精判）
