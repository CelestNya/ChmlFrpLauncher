# Handoff — ChmlFrpLauncher fnOS 移植

> 最后更新：2026-08-19
> 本文件是**持久化交接文档**：记录当前工作状态、待决策事项、关键上下文，随代码变更实时更新。
> 新会话先读本文件 + `docs/development.md`（开发须知）+ `docs/architecture.md`（架构状态）。
> 敏感信息（NAS 凭据）不入本文件，见环境提示。

---

## 当前状态（一句话）

**架构 2 定版已落地并发版 v0.7.5-2.0.0（2026-08-19）**：四分支矩阵（main 干净基线+快照 / adapter 纯契约层 / mod 业务+shim+持久化 / patcher 前端开发台）；padding 配置化根治（uiConfig.padTop，crop 零手工维护段）；adapter API 1.1.0（download-result 事件 + storage/httpApi 契约段）。**v0.7.5-2.0.1 发版进行中**（镜像源换血：移除死源 + 每源独立连接超时），CI 构建中，NAS 回滚 2.0.0 → 在线更新 2.0.1 验证待用户操作。

## 待决策事项

1. **fnos-patchgen.yml 适配四分支**（进行中遗留）：现只 PR patches/ 到 main，未含 adapter 的 fnos-api 变更（crop 需单独走 adapter 分支）。架构定版后此工作流未更新。
2. **镜像源候选池**：已实测 gh-proxy.com 可用（4MB/s）；如后续失效需重新摸底国内 GitHub 加速镜像。当前列表只留官方 + gh-proxy.com。
3. **NAS 在线更新验证（本次待办）**：NAS 回滚 2.0.0 → 模拟源指向真实 2.0.1 GitHub asset → 验证镜像回退链路（官方死 → gh-proxy 成功）。**需用户授权/操作。**

## 已完成里程碑（2026-08-13 ~ 08-19）

| 里程碑 | 证据 |
|--------|------|
| 前后端分离重构（ADR-0001~0004） | shim 重定向 + daemon 存储 + 壁纸文件化 + capabilities |
| fnOS API 契约 + adapter（ADR-0005） | fnos-api/API.md + adapters/v0.7.5/manifest.json + verify-adapter.sh |
| 更新流程改进（功能位 6，改进批 1+2） | 分阶段进度/多源退避/异步化(504)/自动刷新/日志持久化/force 检查 |
| 发版 v0.7.5-1.6.0（2026-08-18） | 组合快照 4e4b683 + tag + CI 双架构 fpk + release + bundle |
| **架构 2 定版（2026-08-19）** | 四分支矩阵 + 分支更名 patch→mod + padding 配置化根治 + API 1.1.0 |
| **发版 v0.7.5-2.0.0（2026-08-19）** | 快照 ead7dc4 + tag + CI 全绿 + release + 双架构 bundle + 正式发行说明 |
| **v0.7.5-2.0.1（2026-08-19，进行中）** | 镜像源换血（Cargo/fpk/patches 三处版本对齐）+ 快照 592295c + CI 跑批 |
| NAS 真机验证 | 1.6.1→2.0.0 更新链路走通（用户验收：padding 正常、更新成功） |

## 最近提交（各分支 HEAD）

```
main:        592295c release: v0.7.5-2.0.1 组合快照（镜像源换血 + 每源独立超时）
adapter/v0.7.5: beaab00 refactor(adapter): API 1.1.0 + padding 配置化 crop
mod:         5785d02 fix(mod): 镜像源换血 + 版本 2.0.1
patcher:     4ed14d3 chore: gitignore 交付产物目录 dist-release-200/
```

## 关键环境

- **工作目录**：D:\Projects\2026-SummerHoliday\ChmlFrpLauncher
- **fork**：CelestNya/ChmlFrpLauncher · 上游：TechCat-Team/ChmlFrpLauncher（v0.7.5 基线）
- **worktrees**：chmlfrp-wt-mod（mod）/ chmlfrp-wt-adapter（adapter）/ chmlfrp-wt-main（main，发版快照用）
- **NAS**：192.168.12.32（SSH 凭据在 `C:\Users\CelestNya\AppData\Local\Temp\nas-*.py`，最新 nas-deploy-200.py）；访问 `https://nas.letian.us.kg/app/chmlfrp`
- **CI**：fnos-build.yml（tag v* 触发双架构 fpk + attach bundles）/ fnos-patchgen.yml（待适配）/ fnos-upstream-follow.yml（上游自动跟随）
- **本地交付物**：`dist-release-200/`（2.0.0 双 fpk + 双 bundle，gitignore 已排除）

## 关键经验（发版/部署防踩坑）

1. **快照禁止 `git add -f fnos-daemon` 整目录**——会拖进 target/ 编译产物（134MB 超 GitHub 100MB 限制被拒；已两次踩坑）。精确 add：`fnos-daemon/Cargo.toml` + `Cargo.lock` + `src/` + `fnos-pack/*`。
2. **版本三处对齐**（fpk manifest = Cargo.toml = patches manifest），CI 的 build-fpk 校验不一致直接 fail。
3. **tag 已推远端后 amend 快照**：需 `git push --force` tag（--force-with-lease 会拒绝 tag 更新），且 force push 会触发新 CI run（旧 run 产物作废）。
4. **坏快照被 push 后无法立即清除**：force push 移动分支/tag 指针，坏 commit 留对象库等 gc——不影响后续。
5. **install-fpk 必须带 --volume 1**；install-local 会删 appdata 数据目录（8/13 事故）。
6. **模拟源**：bundle + update-api.json 放 daemon 静态托管 `dist/`，browser_download_url 必须带 `/app/chmlfrp/` 前缀；UPDATE_API_URL 注入 cmd/main（`${TRIM_SERVICE_PORT}` 网关端口）；appcenter-cli 需 sudo（rpcbroker 权限）。
7. **daemon 只能在 Linux/musl 工具链编译**（Windows 本地缺 x86_64-linux-musl-gcc）——发版靠 CI。
8. **镜像实测**（2026-08-18）：官方直连被墙（30s+）；ghproxy.com/mirror.ghproxy.com 已死；gh-proxy.com 可用 4MB/s。

## 待办（下个会话）

- [ ] 查 v0.7.5-2.0.1 CI 结果 → 创建/更新 release + 正式发行说明 → 下载 bundle 到 dist-release-201/
- [ ] NAS 回滚 2.0.0（用 dist-release-200/ 的 x86 bundle 恢复 target）+ 模拟源指向真实 2.0.1 GitHub asset → 用户在线更新验证镜像回退
- [ ] fnos-patchgen.yml 适配四分支（纳入 adapter 变更路径）
- [ ] NAS 只读优先原则：所有写操作先征得用户同意

## Suggested skills（下个 agent）

- 继续发版/部署：按 `docs/development.md` §七发版流程 + 本文件待办
- 诊断问题：`superpowers:systematic-debugging`（先定位根因再改）
- 架构变更：先读 `docs/architecture.md` + grill 对齐用户再动
