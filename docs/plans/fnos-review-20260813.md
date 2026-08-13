# fnOS 移植代码审查报告（2026-08-13）

> 审查基线：upstream/develop@722d8ef。范围：相对上游的 51 文件 / 8897 行增量
> （fnos-daemon 14 文件、fnos-shim 2 文件、fnos-pack 3 脚本 + fpk 模板、CI 2 个 workflow、
> 前端 10 文件侵入改动）。六角度：功能正确性 / 异常处理 / 边界条件 / 空值处理 / 并发风险 / 性能隐患。
> 方法：5 个并行只读审查代理，辅助工具：cargo clippy、bash -n、eslint、tsc --noEmit、
> 对照 node_modules 内 @tauri-apps 插件 dist-js 真实调用形态、gh api 只读查询。

## 统计

| 领域 | 高 | 中 | 低 |
|---|---|---|---|
| fnos-daemon（Rust） | 3 | 12 | ~15 |
| fnos-shim（浏览器 shim） | 3 | 6 | 8 |
| fnos-pack（构建脚本） | 3 | 7 | ~15 |
| CI（2 workflow） | 2 | 6 | 7 |
| 前端 patch（10 文件） | 1 | 6 | 8 |

---

## fnos-daemon

### 高
1. **静态服务路径穿越 → 任意文件读取（含明文 token）** `main.rs:53-85`
   未校验 `../`，`GET /../../data/g_44.ini` 可读出 frpc 明文 user_token（开发布局下）。
   修复：拒绝 `..`/`\` + canonicalize 后断言 starts_with(web_dir)。
2. **网关 socket 权限 + Gateway 模式信任任意非空 Header** `main.rs:246-248` `auth.rs:64-75`
   socket 权限靠 umask（NAS 上任何本地进程可连）；`x-trim-user` 非空即放行、可伪造；
   None 模式无鉴权 → 任意本地用户可拿全部 45 个 invoke 权限。修复：socket 0600 + HMAC + 警告。
3. **start_frpc 检查-插入竞态 → 孤儿 frpc** `frpc.rs:111-119,154-196` `custom.rs:269-343` `persist.rs:37-59`
   并发 start 同 id 各自 spawn，后 insert 覆盖前 Child → 第一个进程无人追踪（无法 stop、不参与守护、僵尸）。

### 中（摘要）
- 守护重启与手动停止 TOCTOU：tick 判定离线→sleep 1s 期间用户 stop，重启仍发生且清掉 manually_stopped 标记（`guard.rs:119-274`）
- 守护重启失败即永久移除守护，无退避；反之崩溃风暴 3s 无限重启（`guard.rs:176-186`）
- broadcast Lagged 漏掉 STOP 模式日志 → token 失效隧道无限重启（`guard.rs:88`）
- **PID 复用误杀/假运行中**：无 start_time/comm 校验（`persist.rs:139-182`）
- async 上下文持锁阻塞 `Child::wait()`（kill 失败时死锁级）（`frpc.rs:219-235`）
- 自更新非原子替换 + 无回滚 + 双 daemon 并存窗口（`update.rs:279-314`）
- **token 脱敏不完整**：尾段/中段分片不掩码；现有测试未覆盖真分片场景（`frpc.rs:471-502`）
- **恢复的隧道未重新注册守护**（daemon 重启后存量隧道永久失去自动重启，`main.rs:167-171` 注释与实现不符）
- frpc 下载 Range 语义不健壮（200 响应追加损坏、truncate 直写工作二进制）（`download.rs:233-351`）
- HTTP 代理白名单 DNS 化 SSRF + 重定向逃逸（`proxy.rs:25-48`）
- WS 补发与实时帧重复窗口（subscribe 先于 snapshot）（`ws.rs:24-49`）
- 其他：disable→enable 不重注册、delete_custom_tunnel 不移除守护、check_log 裸 events.send 不进缓冲、running_tunnels.json 损坏静默清空

### 测试缺口
仅 4 个测试（sanitize_log，且未覆盖分片场景）。需补：guard 状态机、frpc start/stop 竞态、
persist 恢复、update version_compare/verify_bundle、proxy 白名单、ws 环形缓冲、auth、download Range。

## fnos-shim

### 高
1. **「导出日志」完全失效——下载空文件** `tauri-shim.ts:106-119,260-271`
   `plugin:fs|write_text_file` 真实形态 = `invoke(cmd, Uint8Array, {headers:{path,options}})`；
   shim 丢弃第三参数、把二进制当 args → path 落空、内容为空。同理 dialog 真实 payload 是 `{options:{...}}` 嵌套。
2. **页面加载时 WS 首连补发帧被静默丢弃** `tauri-shim.ts:56-103,377`
   shim 页面加载即连 WS，daemon 立即全量补发，此时 React 未挂载、无 listener → 全部丢弃；
   且补发仅发生在连接建立时 → 重载后日志区空白，要等 60s 断线重连才恢复。违背 daemon 声明的「重载恢复历史」特性。
3. **dialog.save 拿不到 defaultPath** `tauri-shim.ts:318-323`（同源：读 args.defaultPath 而非 args.options.defaultPath）

### 中（摘要）
- updater 下载进度 Channel 永不触发，进度条恒 0 + callbacks 泄漏（`231-247`）
- dialog.open 忽略 filters、accept 硬编码 image/*（视频背景选不中）（`274-317`）
- **invokeCommand 无超时、不检查 HTTP 状态**（与注释宣称不符；daemon 重启时 Promise 永久 pending）（`106-119`）
- unlisten 不清 callbacks 表（真内存泄漏）（`140-149,368-375`）
- **build-shim.mjs 在 Windows 上永远定位不到原生二进制**（win32 包无 bin 目录，恒走 node 兜底）（`build-shim.mjs:50-59`）
- dataURL 2MB 单图 + playlist 4MB 叠加超 localStorage 配额（无 try/catch）

## fnos-pack

### 高
1. **OUTPUT_DIR 的 rm -rf 无边界校验 + 产物替换非原子**（`apply-patches.sh:18,90-92`）
   `OUTPUT_DIR=$REPO_ROOT` 或 `src` 会删仓库；rm→mkdir→cp 中途失败留下残缺 dist-fnos。
2. **gen-patch.sh 自检不覆盖差异完整性**（漏登记/untracked 文件仍自检通过——useTunnelProgress 教训的系统性根源）
   建议：生成前比对 `git diff --name-only $BASE -- src/` 与清单并集；检测 untracked。
3. **图标生成失败仅警告继续** → 产出缺 ICON 的不规范 .fpk（`build-fpk.sh:152-166`）

### 中（摘要）
- `|| true` 滥用：public/ 复制失败静默 → 产物缺音效等文件（`apply-patches.sh:24-28`）
- 失败后残留陈旧 dist-fnos + `--skip-frontend` 只查 index.html 存在
- **Pillow ≥10 移除 Image.LANCZOS** → 构建机升级后整链崩溃（`build-fpk.sh:158-161`）
- cmd/main start 不校验二进制存在即返回 0（偏离官方退出码契约）
- **版本漂移：manifest 0.7.5 vs package.json 0.8.0**，无一致性校验
- daemon 静态链接校验顺序误导（先 grep file 输出再查文件存在）
- 自检子串匹配脆弱（replay/__FNOS__ grep 抽查 2/6 文件）

## CI

### 高
1. **attach-bundles 依赖 release.yml 创建 release，无等待/无兜底**（`fnos-build.yml:113-134`）
   实测：fork releases 为空、release.yml 从未运行、secrets 为空 → tag 首推 attach 必挂 404；
   且 daemon 自更新依赖 bundle 已 attach，缺失时全量设备「检查到新版本但下载 404」。
   修复：轮询等待 + `gh release create` 兜底 + 显式 contents: write。
2. **版本三处来源无一致性校验**（manifest / Cargo.toml / git tag）→ 一处漂移静默杀死自更新通道

### 中（摘要）
- patchgen 无 concurrency 防护：并发 run 乱序 force push + 建 PR TOCTOU（422 红 run 曾真实发生）
- patch 基线（upstream/develop）与 PR base（main）一致性无 CI 校验（src/ 不变式靠人工维持）
- PR 已存在时不刷新 body（已实证 PR #12 body 过期）
- needs: build-fnos 使单架构失败全盘拒发 bundle
- aarch64 musl 工具链未锁版本、无缓存、单点外部依赖
- fnos-build 无 permissions 声明（依赖仓库默认设置）

## 前端 patch

### 高
1. **去重集合「replay 先到、实时副本后到」漏网窗口**（`logStore.ts:26-31` + `ws.rs:24-49`）
   订阅先于快照锁，同帧可能先 replay 后实时，实时副本不查集合 → 仍偶发重复。

### 中（摘要）
- **replaySeen 无限增长，且桌面版同样增长（桌面回归）**——frpcManager 桌面版每帧 replay:false 也入集合
- **isDuplicate 对 replay 去重是死代码**，实际改变桌面版行为（同秒同内容日志被吞，含 develop 新加的 frpc 缺失引导日志）
- localStorage 配额溢出无防护（QuotaExceededError 在 useEffect 中 → 可能白屏）
- clearLogs 竞态 + clear_log_history 失败静默（daemon 缓冲未清 → 下次重连 512 条复活）
- fnOS 重载后进度条无法从补发日志恢复（replay 帧整条跳过）
- 首屏日志延迟：startListening 仅 TunnelList 挂载时注册 + shim 首连帧丢弃（与 shim H2 同根因）

### 集成结论
- frpcManager.ts 与 develop 重构**集成正确无错位**（diff 干净落在重构后代码正确位置）
- App.tsx / Settings 三件套是**无 __FNOS__ 门控的全量裁剪**——patched 分支安全，但未来合并回桌面基线最高危
- logStore.ts / frpcManager.ts 是**运行时影响桌面版**的文件（M1/M2）——上游 PR 前必须门控

---

## 审验结论（2026-08-13 二次核验）

对全部 12 个高危 + 关键中危逐条读代码/对照依赖源码/gh 只读实证复核。**无一误报**，1 处中危部分误判。

### 12 个高危逐条判定（全部属实）

| # | 判定 | 关键证据 |
|---|---|---|
| daemon 高-1 路径穿越 | ✅ 属实 | `main.rs:62-85` join 前无 `..` 校验；生产路径过 nginx 时归一化可能部分缓解，回环 17890/开发模式直接成立 |
| daemon 高-2 socket/auth | ✅ 属实 | `main.rs:246-248` bind 无 set_permissions；`auth.rs:64-75` x-trim-user 非空即放行 |
| daemon 高-3 start 竞态 | ✅ 属实 | `frpc.rs:111-119` 检查锁与 `190-196` 插入锁分离，spawn 在锁外 |
| shim H1 导出日志空文件 | ✅ 属实 | plugin-fs@2.5.1 dist-js:703 实锤：`invoke(cmd, Uint8Array, {headers:{path,...}})`；shim 丢第三参、读 args.path/args.contents 全空 |
| shim H2 首连补发丢弃 | ✅ 属实 | shim:377 IIFE 求值即 connectEvents；82-83 无 listener 直接 return |
| shim H3 dialog.save defaultPath | ✅ 属实 | plugin-dialog@2.7.2 dist-js:115 实锤 `{options}` 嵌套；shim:321 读 args.defaultPath |
| pack H1 rm -rf 边界 | ✅ 属实 | `apply-patches.sh:90` 无校验（低概率高后果） |
| pack H2 gen-patch 自检盲区 | ✅ 属实 | 自检只 dry-run 清单内文件；useTunnelProgress 事故即实证 |
| pack H3 图标仅警告 | ✅ 属实 | `build-fpk.sh:164-166` 警告后继续 |
| CI 高-1 release 依赖 | ✅ 属实 | gh 实证：fork 0 release、0 secrets；attach 无 create 兜底 → tag 首推必 404 |
| CI 高-2 版本三源 | ✅ 属实 | 实测已漂移：manifest 0.7.5 / Cargo.toml 0.7.5 / package.json 0.8.0 |
| 前端 H1 去重窗口 | ✅ 属实 | `logStore.ts:26-31`：replay 先到时实时副本不查集合；`ws.rs:24-49` subscribe 先于 snapshot 锁 |

### 关键中危抽核

- **属实**：token 脱敏分片泄漏（`frpc.rs:471-502` 只切首个 `.`、窗口只扫前缀；现有 2 个「分片」测试消息含完整 token，是**假阳性覆盖**）；async 持锁阻塞 wait（`frpc.rs:219-228`）；恢复隧道未注册守护（`main.rs:167-171` 注释承诺、guard.rs 无实现）；proxy SSRF（`proxy.rs:25-48`，**照抄桌面版=上游同款问题，属继承而非新引入**）；updater Channel 进度永不触发（plugin-updater@2.10.1 dist-js:49 实锤）；unlisten 泄漏 callbacks；Pillow 10 LANCZOS；replaySeen 桌面版也增长 + isDuplicate 注释「恒为 false」错误。
- **部分误判 1 处**：前端 M6「startListening 仅 TunnelList 挂载」——实为 App 层 `useAppInitialization()`（App.tsx:42）已调用；useTunnelProgress:398 只是幂等重复。但「首连快照在 React 挂载前被丢弃」现象本身属实（归入 shim H2）。
- **影响评估修正 1 处**：shim M5（build-shim Windows 原生二进制）——现象属实（win32 包无 bin/ 且脚本过滤 `!d.includes("+")` 恒走 JS wrapper），但「Windows 构建会崩」过度：本地构建实测成功；本质是注释与实现不符，降级为中低。

### 最终结论

1. **审查质量：可信**。12 高危全部属实，可直接作为修复依据；修复前无需再逐条侦查（仅需常规的修复期回归验证）。
2. **最紧迫的是用户可见功能断裂**：导出日志空文件（shim H1）、页面重载日志空白 60s（shim H2）——真机上必现。
3. **上游 PR 的硬门槛**：路径穿越读明文 token、socket/HMAC、脱敏假阳性测试——直接对应「脱敏 + 测试完整性」目标；proxy SSRF 是上游继承问题，PR 中修需注明超范围。
4. **桌面回归风险是上游 owner 最敏感点**：replaySeen/isDuplicate 必须 `__FNOS__` 门控后才能进 PR。
5. 建议修复顺序：B（真机功能）→ A（安全+脱敏测试）→ C（桌面门控）→ D/E（CI/构建链）→ F（测试补全）。

---

## 修复优先级建议（供决策）

- **批次 A 安全（上游 PR 必改）**：daemon 高-1 路径穿越、高-2 socket/HMAC、proxy SSRF、
  token 脱敏完整性（中-7）+ 测试
- **批次 B 真机功能**：shim H1 导出日志、H2 首连竞态、M2 dialog filters、M3 invoke 超时；
  前端 H1 去重窗口、M4 clearLogs、M5 进度恢复
- **批次 C 桌面回归护栏**：前端 M1/M2 加 __FNOS__ 门控（上游 owner 关注点）
- **批次 D CI 可靠性**：CI 高-1 release 兜底、高-2 版本校验、中-1 concurrency、中-3 body 刷新
- **批次 E 构建链**：pack H1 rm -rf 边界、M1 || true、M3 Pillow、M5 版本一致性
- **批次 F 测试补全**：daemon 9 项测试清单（见上）

> 注：所有修复落地前需逐一复验根因（审查代理可能误判）；修复走 patcher 分支常规流程。
