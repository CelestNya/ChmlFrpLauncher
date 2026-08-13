# ADR-0002：凭据与行为配置后端化（shim 存储重定向）

**状态**: accepted
**决定**: Credential（`chmlfrp_user`）与 NodeSetting（`frpc_proxy_config`/
`frpcLogLevel`/`bypassProxy`/`restartOnEdit`）物理离开浏览器 localStorage，
落到 daemon 文件（`credential.json` / `node_settings.json`，均 0600）。**不改
前端业务代码**（`api.ts`/`frpcManager.ts` 零改动）——由 fnOS 构建注入的 shim
拦截 localStorage 白名单 key 的读写，转发 daemon；桌面版不经 shim，行为不变。

**为什么**:
1. fnOS 是 iframe 网关环境，localStorage 里的 token/明文代理密码对 XSS/第三方
   脚本暴露面大；移 daemon 文件（0600 + 进程独占）显著缩小攻击面。
2. 前端业务代码零改动 → 上游怎么迭代 `api.ts`/`frpcManager.ts` 我们零冲突，
   patch 面维持在「浏览器基座 + fnOS 专属文件」。
3. 桌面版是 Tauri webview 本机隔离，风险面远小于 fnOS iframe，暂不迁移
   （跟随后续版本）。

**诚实边界**: token 仍会短暂出现在前端内存与 daemon 注入的 boot blob HTML
（`api.ts` 调官网 API 需要它），但**不再持久化于 localStorage**。本 ADR 解决
「不持久化」，不承诺「前端永不见 token」。

**登出顺序**（定死）: 先停全部隧道（清 `g_*.ini`）→ 再 `clear_credential`。
顺序反过来会在磁盘残留含 token 的 ini。
