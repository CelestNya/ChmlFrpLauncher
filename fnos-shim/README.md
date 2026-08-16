# fnos-shim

fnOS 版 Tauri 2 运行时模拟层。构建期把 `tauri-shim.ts` 编译为 IIFE 注入 `dist/index.html`，
让未修改的前端（`@tauri-apps/api` v2）在浏览器 / fnOS iframe 中运行，后端语义由 `fnos-daemon` 提供。

## 文件

- `tauri-shim.ts` — shim 源码（IIFE，无顶层 import/export）
- `build-shim.ts` — 构建期注入脚本（esbuild 编译 → 注入 `</body>` 前普通 `<script>`，同步早于 module bundle）

## 覆盖面（以 src/ 实际 import 为准）

| 模块 | 行为 |
|------|------|
| `core.invoke` 普通命令 | POST `/api/invoke` {cmd,args} → {ok,data,error} 解包 |
| `core.invoke` 插件命令 | event 监听注册 / app 版本 / window 状态查询（布尔/尺寸默认值）/ dialog（save→下载、message/confirm 降级）/ fs 写文件→下载 / opener 新窗口 / updater 无更新 / 其余静默降级 |
| `event.listen` | WS `/ws/logs` 同源桥接，按事件名分发（frpc-log / download-progress / tunnel-auto-restarted），断线指数退避重连 |
| `app.getVersion` | GET `/api/bootstrap` |

## 注入方式

1. 全链：`bash fnos-pack/apply-patches.sh`（patch → tsc → vite build → 注入 shim → `dist-fnos/`）
2. 单独注入已有 dist：`node fnos-shim/build-shim.ts --dist <dist-dir>`

## 与桌面版区别

- 桌面版：原生 WebView + Tauri IPC
- fnOS 版：iframe + daemon（axum）+ WS + shim

## 已知降级 / 待补

- `plugin:dialog|open` 返回 null（浏览器无法取真实路径）→ 背景图选择在浏览器形态受限；daemon 侧
  copy_background_image/video 为 NO_OP，B5 打磨项
- `plugin:updater|check` 返回 null（无更新）；fnOS 自更新走 daemon（B5）
- OAuth 设备码登录为裸 fetch，跨域 CORS 是否放行需 WSL/真机实测；失败则 patch 增补 api.ts 走 daemon 代理
- 前端新增 Tauri 调用需同步补本 shim（覆盖面以 45 命令 + 4 事件为准）
