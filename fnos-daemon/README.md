# chmlfrp-daemon

ChmlFrp fnOS 守护进程：axum 服务承载 frpc 隧道管理，替代桌面版（Tauri）的 IPC 层。

## 运行

```bash
cargo run --release --manifest-path fnos-daemon/Cargo.toml
# 或指定数据目录/前端目录/端口（开发调试）
DAEMON_DATA_DIR=./data DAEMON_WEB_DIR=../dist TRIM_SERVICE_PORT=17890 cargo run -p ...
```

fnOS 环境变量（由 .fpk 生命周期脚本注入）：
- `TRIM_PKGVAR`：数据目录（frpc 二进制 / 隧道配置 / PID 持久化 / 自定义隧道）
- `TRIM_APPDEST`：应用安装目录（前端 dist 位于 `{TRIM_APPDEST}/dist`）
- `TRIM_SERVICE_PORT`：服务端口

## API

| 路由 | 说明 |
|------|------|
| `GET /api/bootstrap` | 版本信息（shim 用） |
| `POST /api/invoke` | 命令透传（与 Tauri IPC 同语义，见 invoke.rs 契约） |
| `GET /ws/logs` | 事件推送（frpc-log / download-progress / tunnel-auto-restarted） |
| `*` | SPA 静态托管（回退 index.html） |

默认仅监听 `127.0.0.1`；`DAEMON_TOKEN` 设置后启用 Token 鉴权（开发用），
fnOS 目标形态由统一网关转发（见 docs/fnos-porting-plan.md Q5）。

## 模块

| 模块 | 职责 | 规格源（src-tauri） |
|------|------|---------------------|
| `config.rs` | 环境变量 / 数据目录 / 监听地址 | — |
| `auth.rs` | 鉴权中间件（网关 Header / Token） | — |
| `invoke.rs` | /api/invoke 命令分发表 | lib.rs invoke_handler |
| `frpc.rs` | 官方隧道启停 / 配置生成 / 日志脱敏 | commands/process.rs + utils.rs |
| `guard.rs` | 3s 守护轮询 + 错误模式停止 | commands/process_guard.rs |
| `persist.rs` | PID 持久化 / 孤儿回收 | commands/process_persistence.rs |
| `custom.rs` | 自定义隧道 CRUD / 启停 / ini 解析 | commands/custom_tunnel.rs |
| `download.rs` | frpc 下载 / 续传 / sha256 | commands/download.rs |
| `proxy.rs` | ChmlFrp API 代理（URL 白名单） | commands/http.rs |
| `net.rs` | ping / 端口检测 / DNS | commands/ping.rs + ports.rs |
| `events.rs` / `ws.rs` | 事件广播 / WebSocket 推送 | models.rs LogMessage 等 |
