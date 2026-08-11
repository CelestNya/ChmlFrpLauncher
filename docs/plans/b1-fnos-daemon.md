# B1 实施计划：fnos-daemon crate

> 最后更新：2026-08-11
> 状态：✅ **已完成**（C1-C6 全部提交，验收通过）
> 关联：[fnos-porting-plan.md](../fnos-porting-plan.md)（整体 plan）、[architecture.md](../architecture.md)

## 一、前置约定

- **零改动约束**：`src/` 与 `src-tauri/` 一个字节不动；本批全部为新增文件（git 只增不改）
- **分支**：develop 分支开发；commit 遵循 Conventional Commits，scope 统一 `fnos-daemon`
- **三个默认值**（grill 决议）：
  - D1 守护默认**开启**（NAS 无人值守场景）
  - D2 frpc 来源：**.fpk 内置对应架构 frpc 优先**，缺省时自动下载（复用流式下载 + sha256 校验）
  - D3 数据目录：**全部 TRIM_PKGVAR**（frpc 二进制 / 隧道配置 / PID / 自定义隧道），重启保留、不依赖 target 写权限

## 二、形态与模块

独立 crate `fnos-daemon/`（不加入 workspace，避免动根 Cargo.toml）：

```
fnos-daemon/
├── Cargo.toml           ← 独立依赖树（axum/tokio/tower-http/reqwest/serde/sha2 等，零 tauri 依赖）
└── src/
    ├── main.rs          ← 启动：解析 TRIM_* 环境变量 → 数据目录 → axum 服务（默认 127.0.0.1）
    ├── invoke.rs        ← POST /api/invoke 命令分发表（45 命令：32 实现 + 13 NO_OP）
    ├── frpc.rs          ← 启停/配置生成/日志管道/脱敏（规格源：src-tauri commands/process.rs + utils.rs）
    ├── guard.rs         ← 守护轮询 3s + STOP_GUARD_PATTERNS（规格源：process_guard.rs）
    ├── persist.rs       ← PID 持久化 running_tunnels.json + 孤儿回收（规格源：process_persistence.rs）
    ├── custom.rs        ← 自定义隧道全套（规格源：custom_tunnel.rs）
    ├── download.rs      ← frpc 下载：流式/续传/sha256（规格源：download.rs）
    ├── proxy.rs         ← http_request / http_request_raw（规格源：http.rs）
    ├── net.rs           ← ping / 端口检测 / DNS 解析（规格源：ping.rs ports.rs）
    ├── ws.rs            ← tokio broadcast → WS /ws/logs
    ├── auth.rs          ← 鉴权中间件（网关 Header 模式 / token 模式，默认仅回环）
    └── config.rs        ← TRIM_PKGVAR 优先，开发模式本地默认目录
```

## 三、invoke 契约

```json
POST /api/invoke
{ "cmd": "start_frpc", "args": { "tunnelId": 3, "config": { ... } } }
→ 200 { "ok": true, "data": "frpc 已启动 (PID: 123)" }
→ 200 { "ok": false, "error": "该隧道已在运行中" }
```

- 参数结构体 `#[serde(rename_all = "camelCase")]`（与 Tauri IPC 同语义）
- `GET /api/bootstrap` → `{name, version}`
- `WS /ws/logs` → 事件帧 `{type: "frpc-log" | "download-progress" | "tunnel-auto-restarted", payload: {...}}`，载荷与桌面版一致

## 四、命令实现范围（45 个）

| 归组 | 命令 | 处理 |
|------|------|------|
| 实现（32） | process 6（start/stop/is_running/get_running_tunnels/fix_tls 外 5 个）· guard 5 · persist 3 · custom 8 · download 4 · http 2 · ping 1 · ports 2 · resolve_domain_to_ip | 照桌面版逻辑重写 |
| NO_OP（13） | autostart 4 · tray 3 · background 4 · fix_frpc_ini_tls · read_image_folder | 显式返回 `{ok:false, error:"该功能在 fnOS 版不可用"}` |

> 注：fix_frpc_ini_tls 归 NO_OP（TLS 修复是桌面版历史遗留，daemon 生成配置时直接写正确值）。

## 五、安全细节（照抄不简化）

- 日志脱敏（sanitize_token 全量照抄）；frpc 配置写盘 `0o600`
- STOP_GUARD_PATTERNS 11 条原样；MutexGuard drop 顺序约束保持
- 默认仅监听 127.0.0.1（统一网关在 B3 转发）；token 模式默认关闭（开发调试用）

## 六、验收标准（每 commit 与最终 gate）

每个 commit 的验收见下方点位表「验收」。**B1 整体 gate（C6 后执行）**：

1. `cargo build --release` 零警告（无 tauri/WebKit 依赖链）
2. 45 命令分类正确：32 可用、13 NO_OP 返回明确错误
3. 功能冒烟：frpc 启停 / 守护自动重启 / WS 实时日志（含脱敏）/ 自定义隧道全流程 / 下载校验 / 网络探测 / API 代理
4. 安全：日志脱敏照抄；配置 0o600；默认仅 127.0.0.1
5. 回归：`src/`、`src-tauri/` 无任何改动（git status 确认仅新增文件）
6. 文档同步：`docs/architecture.md` §八 fnOS 意向节点更新（B1 已交付 ✅ 局部）；本计划勾选 C1-C6

## 七、commit 点位

| # | commit message | 内容 | 验收 | 状态 |
|---|----------------|------|------|------|
| C0a | `docs: 架构文档与 fnOS 移植整体计划` | architecture.md / fnos-porting-plan.md / docs README | 文档存在且一致 | ✅ |
| C0b | `docs(fnos-daemon): B1 实施计划与验收标准` | 本文档 + README 索引 | 索引含本文档 | ✅ |
| C1 | `feat(fnos-daemon): 脚手架与基础路由` | crate 骨架 + config + auth 骨架 + /api/bootstrap + /api/invoke 空分发 | cargo build ✅；curl bootstrap/unknown 命令正确 | ✅ |
| C2 | `feat(fnos-daemon): frpc 进程管理与 invoke 分发` | invoke 分发引擎 + frpc.rs + persist.rs | curl 启停 frpc；脱敏验证；PID 持久化 | ✅ |
| C3 | `feat(fnos-daemon): 守护轮询与 WS 事件推送` | guard.rs + ws.rs | WS 实时日志；kill 后 3s 自动重启；模式命中停守护 | ✅ |
| C4 | `feat(fnos-daemon): 自定义隧道与 frpc 下载` | custom.rs + download.rs | CRUD+启停；sha256 校验；内置优先 | ✅ |
| C5 | `feat(fnos-daemon): 网络探测与 HTTP 代理` | net.rs + proxy.rs | ping/端口/DNS；API 代理冒烟 | ✅ |
| C6 | `chore(fnos-daemon): 收尾与全量验收` | NO_OP 补齐 + README + 日志完善 | 整体 gate 1-6 全过；桌面版回归 | ✅ |

> 补充：C2 后追加 `fix(fnos-daemon): invoke 嵌套参数与 Tauri snake_case 对齐`（嵌套结构体字段保持 snake_case，与真实前端载荷匹配）。
