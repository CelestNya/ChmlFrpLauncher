# fnOS NAS 人工测试清单（2026-08-13 部署版 v0.8.0）

> 部署内容：批 A-F 修复后的 daemon + 前端（基线上游 develop v0.8.0 WIP）。
> 访问地址：`https://nas.letian.us.kg/app/chmlfrp`
> 硬约束：NAS 数据零接触；杀进程用精确 PID，禁止通配符；测试前先 Ctrl+F5 强刷浏览器。

## ✅ 部署后自动验证（已通过）

- [x] app.sock 权限 = `srwx------`（0600，A2 修复生效）
- [x] daemon 启动日志含「鉴权模式: None」警告（A3；真机实测平台不注入 TRIM_GATEWAY/DAEMON_TOKEN，None 是生产实际模式）
- [x] daemon v0.8.0 启动，网关 socket 监听正常
- [x] frpc 自动重新下载（14MB，md5 与旧版一致），网关注册 `/app/chmlfrp` 成功

## ✅ 功能验收（浏览器实测通过）

### 隧道
- [x] 隧道列表：7 个隧道完整显示（登录 CelestNyaa 后）
- [x] 隧道启动：h8ySiSBZ 启动，进度推进，连接成功
- [x] 隧道配置恢复：登录后 daemon 从 ChmlFrp API 重新生成配置（g_308681.ini + running_tunnels.json 快照含 config）

### 日志
- [x] 启动日志实时回显（无 60s 延迟）
- [x] **导出日志**：保存日志通知成功（B1 修复，IAB 环境下载无法验证文件名，通知确认链路通）
- [x] **清空日志**：通知「日志已清空」+ 显示「等待日志输出...」，无 toast 报错（B9）
- [x] **页面重载**：重载后历史日志立即显示（B2 修复——旧版空白 60s）
- [x] **WS 断线重连**：挂机 70s 后日志不重复、无 toast/音效重放（B7/B8 双向去重）

### 守护（A8/A9 重点验证）
- [x] **kill frpc 后自动重启**：kill 438210 → 守护 3s 轮询检测离线 → 自动拉起新 PID 446055 → 重新连接服务器 → 隧道重新上线
- [x] 守护日志广播：`[W] 检测到进程离线，触发守护进程，自动重启中` + 完整启动链路日志正确显示
- [x] running_tunnels.json PID 同步更新（438210 → 446055）

### 登录（上游 0.8 修复项）
- [x] OAuth 设备码登录成功（轻爪账号 account.qzhua.net，设备码流程正常）
- [x] 登录后刷新不要求重登

## ⚠️ 发现的问题（需修复）

### P1：进程守护默认关闭（fnOS 适配缺陷）——已修复并部署
- **现象**：设置页「进程守护」开关默认关闭；守护不工作直到用户手动打开
- **根因**：`src/components/App/hooks/useAppInitialization.ts:15` 用 `localStorage.getItem("processGuardEnabled") === "true"` 判断，本地无存储 → `null !== "true"` → 初始化调 `set_process_guard_enabled(false)` 关闭守护。这是桌面版逻辑（桌面默认关、用户手动开），但 fnOS 上守护是核心安全特性（A8 恢复守护的意义），应默认开启
- **影响**：frpc 崩溃后无人拉起（用户不知道要手动开）
- **修复**：fnOS 环境（`__FNOS__` 门控）且 localStorage 无记录 → 守护默认 true；用户手动切换后尊重其选择。改 `useAppInitialization.ts` + `useProcessGuard.ts`，登记进 gen-patch FEATURE_FILES
- **部署验证**：daemon 重启后守护状态 `{"ok":true,"data":true}`（无前端干扰时默认开启）；新 dist 部署（index-CwqS55AV.js）含修复代码；前端加载后守护保持 true（localStorage 无记录 → fnOS 默认 true）；kill frpc 458988 → 守护 6s 内自动拉起 459218（最终链路验证）

### P2：install-local 危险（部署工具认知）
- `appcenter-cli install-local` 会先卸载并清空 `/vol1/@appdata/<app>/` 数据目录（8/13 事故根因）。正确升级姿势：`appcenter-cli install-fpk xxx.fpk --volume 1`（必须带 --volume，全新安装时才真正执行 install）

### P2.5：市场升级不替换 dist（已修复）
- **现象**：install-fpk 升级后 daemon 是新的但 dist 停留旧版——守护默认开启等前端修复白部署（手动替换 dist 才生效）
- **根因**：fpk 内 `dist/` 是 777 权限，fnOS 升级时保留「777 可写目录」不覆盖
- **修复**：`build-fpk.sh` 组装时对 `app/dist` 执行 `chmod go-w`（目录 777→755、文件 777/666→755/644），保留 owner 写位与执行位
- **验证**：新 fpk 内 dist 755/644（run 31679385751）；NAS 真实 Linux 上 chmod go-w 行为确认（777→755）
- **备注**：内部自更新（update.rs apply_update）本就用 replace_dir_atomic 整目录替换 dist，不受此问题影响——两者都干净后，市场升级与自更新一致

### P3：遗留（本轮不修）
- OAuth CORS（iframe 下可能失败，本次实测正常）
- 自更新通道（未发 Release）
- daemon 自保（应用中心层面）

---

## ✅ 前后端分离重构真机验证（2026-08-13，ADR-0001~0004）

> 部署：`chmlfrp_0.8.0_x86.fpk`（main 分支 apply-patches + WSL musl 编译 daemon + build-fpk）。
> ⚠️ **部署坑**：install-fpk 对 running 应用**保留已运行的 daemon 二进制**（只替换 dist 不换 daemon 进程），
> 必须 stop → 手动替换 `chmlfrp-daemon`（从 fpk app.tgz 解出）→ start 才生效。
> 曾误以为代码没生效，实为二进制未换（日志仍旧、新命令报「未知命令」）。

### 新命令面（daemon 侧，curl --unix-socket 验证）

- [x] `credential_status` → `{"ok":true,"data":{"logged_in":false}}`（未登录）
- [x] `get_node_settings` → `{"ok":true,"data":{}}`（空默认）
- [x] `GET /api/capabilities` → `{"commands":["start_frpc","stop_frpc",...,"save_background_image"]}`（能力面含新命令，无 NO_OP 桌面命令）
- [x] `get_process_guard_enabled` → `{"ok":true,"data":true}`（fnOS 守护默认开，重构后保持）

### 数据后端化端到端（写 → 落盘 → 读回 → 清理）

- [x] `save_credential` → credential.json 落盘（0600，`-rw------- chmlfrp`）→ `credential_status` logged_in=true → `clear_credential` 删除
- [x] `set_node_settings`（代理/日志级别）→ node_settings.json 落盘（0600）→ `get_node_settings` 读回一致 → 清理
- [x] credential.json / node_settings.json 权限均为 **0600**（仅 chmlfrp 用户可读，token/明文密码不落浏览器）

### 前端（dist 已部署 index-CwqS55AV.js，含 __FNOS_BOOT__/shim 重定向逻辑）

- [x] dist/index.html 含 `__FNOS_BOOT__` + shim（浏览器加载时注入，业务代码零改动）
- [x] daemon 启动日志：鉴权 None + 守护默认开 + 存量隧道恢复注册

### review 修复真机验证（2026-08-13 晚，P2/P3/P5）

> 部署方式更新：**同版本 install-fpk 幂等跳过**（app 表 version=0.8.0 已装，install 只 verify 不执行，
> 无 install_start 日志）。**正确升级姿势 = uninstall → install-fpk --volume 1**（先备份数据目录到
> `/vol1/1000/1/`），无需 bump 版本号（版本与上游同步）。

- [x] **P2 凭据完整往返**：save_credential（含 `usergroup:"vip"`/userimg/tunnel_count/tunnel）→ credential.json 落盘 → get_credential 读回全部字段一致（`{"usergroup":"vip",...}`）→ clear_credential 清理。会员门控字段不再丢
- [x] **P3 壁纸托管 URL**：save_background_image（dataURL `aGVsbG8=`）→ backgrounds/test-bg.png 落盘 → `GET /app/chmlfrp/assets/backgrounds/test-bg.png` → **HTTP 200 + 内容 `hello`**（网关前缀下可达）。前端经 `new URL(rel, location.href)` 得含 `/app/chmlfrp/` 的绝对 URL 可渲染
- [x] 守护默认开 `{"data":true}` + capabilities 能力面正常（不含 capabilities 自身、不含 NO_OP 桌面命令）
- [x] uninstall → install 流程验证：备份（frpc/日志）→ uninstall success → install complete → start → 新 daemon 15:19 启动 → frpc 保留（备份带回）→ 命令全可用
