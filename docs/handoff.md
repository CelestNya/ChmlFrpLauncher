# Handoff — ChmlFrpLauncher fnOS 移植

> 最后更新：2026-08-13
> 本文件是**持久化交接文档**：记录当前工作状态、待决策事项、关键上下文，随代码变更实时更新。
> 新会话先读本文件 + `docs/development.md`（开发须知）+ `docs/architecture.md`（架构状态）。
> 敏感信息（NAS 凭据）不入本文件，见环境提示。

---

## 当前状态（一句话）

**前后端分离重构已完成（阶段 0-5）+ fnOS API 契约/adapter 架构已落地（ADR-0005）**：token/代理配置经 shim 重定向离浏览器 localStorage 落 daemon 文件（0600），壁纸 fnOS 砍轮播 + 文件化，能力协商替代 NO_OP；`fnos-api/` 契约（六面 + apiVersion 1.0.0）+ 第一个 adapter v0.7.5（能力覆盖验证 33 命令全通过）+ 需求→bump→适配→开发流程入库。全量回归绿（daemon 66 / vitest 45 / tsc / eslint / patch 可应用）。

## 待决策事项

1. **verify-adapter.sh 接入 CI**：目前是手动校验（development.md 已记录），可选加轻量 job 到 patchgen workflow 使「patch 对 API 硬依赖」自动强制。
2. **发版策略**：上游 Latest = v0.7.5（2026-05-20），v0.8.0 仅 develop WIP。**版本号与上游同步、不 bump**（升级走 uninstall→install）；跟随上游节奏暂不发 release。
3. **遗留项**：见 `docs/development.md` §七（自更新通道、daemon 自保、OAuth CORS、UI 适配、WS seq）。

## 已完成里程碑（2026-08-13）

| 里程碑 | 证据 |
|--------|------|
| 基线切上游 develop (v0.8.0 WIP) | main src/ == upstream/develop（0 侵入） |
| 六批 TDD 修复 (A-F) | daemon 66 / vitest 45 / lib.sh 9 全绿 |
| CI 修复（pnpm 11 / Node 22） | 双架构构建 success |
| 版本号同步 0.8.0 | manifest / Cargo.toml / package.json / Cargo.lock |
| NAS 部署 + 人工测试 | `docs/plans/fnos-nas-test-checklist.md` |
| 守护默认开启修复 | patcher 0ddb15e + PR #13 |
| 市场升级 dist 权限修复 | patcher 34bc1d9（build-fpk.sh chmod go-w） |
| **前后端分离重构（阶段 0-5）** | CONTEXT.md + docs/adr/0001-0004；shim 重定向 + daemon 存储 + 壁纸文件化 + capabilities |
| **fnOS API 契约 + adapter 架构（ADR-0005）** | fnos-api/API.md + adapters/v0.7.5/manifest.json + verify-adapter.sh（33 命令覆盖验证通过）+ development.md §patch 机制；双分支提交 9c763df/6a8b775 |

## 最近提交（patcher）

```
9c763df feat(fnos): 落地 fnOS API 契约 + adapter v0.7.5（ADR-0005）
1eb476c docs(fnos): 归档 NAS 真机验证（review 状态表 + patch 维护文档）
04bc96d docs(fnos): NAS 测试清单归档 dist 权限修复与守护默认开启验证
34bc1d9 fix(fnos): build-fpk 打包时收紧 dist 权限（市场升级不替换 dist 修复）
0ddb15e fix(fnos): fnOS 环境进程守护默认开启
2234598 chore(fnos): 版本号 0.7.5→0.8.0 同步
```

## 关键环境

- **工作目录**：D:\Projects\2026-SummerHoliday\ChmlFrpLauncher
- **fork**：CelestNya/ChmlFrpLauncher · 上游：TechCat-Team/ChmlFrpLauncher（develop 基线）
- **NAS**：192.168.12.32（SSH 凭据在 `C:\Users\CelestNya\AppData\Local\Temp\nas-*.py`）；访问 `https://nas.letian.us.kg/app/chmlfrp`
- **CI**：fnos-build.yml（构建 .fpk）/ fnos-patchgen.yml（patch 自动 PR）/ fnos-upstream-follow.yml（上游自动跟随，每 6h 轮询）

## 关键经验（详见项目记忆）

1. **install-fpk 必须带 --volume 1**；install-local 会删 appdata 数据目录（8/13 事故）
2. **市场升级不替换 777 目录**——dist 权限已修（chmod go-w）；自更新不受影响
3. **守护日志走 WS 广播不进文件**——SSH 看不到，看前端日志页
4. **守护默认关闭**：桌面版 localStorage 逻辑，fnOS 已修默认开启（__FNOS__ 门控）
5. **真机鉴权**：平台不注入 TRIM_GATEWAY/DAEMON_TOKEN → 生产实际 None 模式，安全 = socket 0600 + 网关独占

## Suggested skills（下个 agent）

- 上游 PR 准备：`code-review`（从基线审变更）
- 继续部署/测试：按 `docs/plans/fnos-nas-test-checklist.md` 手测
- 诊断问题：`superpowers:systematic-debugging`（先定位根因再改）
- 规划发版：先读 `docs/development.md` §六，再与用户确认是否跟随上游
