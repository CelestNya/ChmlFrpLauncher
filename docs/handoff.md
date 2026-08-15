# Handoff — ChmlFrpLauncher fnOS 移植

> 最后更新：2026-08-15
> 本文件是**持久化交接文档**：记录当前工作状态、待决策事项、关键上下文，随代码变更实时更新。
> 新会话先读本文件 + `docs/development.md`（开发须知）+ `docs/architecture.md`（架构状态）。
> 敏感信息（NAS 凭据）不入本文件，见环境提示。

---

## 当前状态（一句话）

**分支矩阵拓扑已落地（2026-08-15）+ 基线切 v0.7.5 LTS**：main（src 0 侵入）+ adapter/v0.7.5（fnos-api 契约，联合版本号 0.7.5-1.0.0）+ patch（业务实现，featureVersion + requires.api，不含 src）。CI 支持 adapter+patch 组合矩阵构建（patch 依赖校验强制）。0.7.5 fpk 已部署 NAS 实测通过（守护默认开启/凭据保留/能力面正常）。

## 待决策事项

1. ~~verify-adapter.sh 接入 CI~~ → **已完成**（fnos-build 组合步骤内置）
2. **发版策略**：基线 = 上游 v0.7.5（LTS）。**版本号与上游同步、不 bump**（升级走 uninstall→install）；跟随上游节奏暂不发 release。
3. **遗留项**：见 `docs/development.md` §七（自更新通道、daemon 自保、OAuth CORS、UI 适配、WS seq）。

## 已完成里程碑（2026-08-15）

| 里程碑 | 证据 |
|--------|------|
| **分支矩阵拓扑** | main/adapter/v0.7.5/patch 三分支；组合构建 CI（workflow_dispatch 选 adapter+patch） |
| **基线切 v0.7.5 LTS** | main src/ == v0.7.5 tag（0 侵入）；产品版本号 0.7.5（E4 四文件）；依赖回退 recharts 2.15.4 |
| **版本号设计定稿** | adapter 联合版本号（基线-API）；patch 单特性版本号（requires.api 硬依赖） |
| **UI 裁剪配置化** | adapter uiConfig 5 配置项驱动条件渲染（generate-ui-config 构建期生成） |
| **0.7.5 fpk 部署实测** | 备份→uninstall→install-fpk --volume 1；守护默认开启 ✓ 凭据保留 ✓ 能力面 ✓ |
| 前后端分离重构（阶段 0-5） | CONTEXT.md + ADR-0001~0004；shim 重定向 + daemon 存储 + 壁纸文件化 |
| fnOS API 契约 + adapter（ADR-0005） | fnos-api/API.md + adapters/v0.7.5/manifest.json + verify-adapter.sh |

## 最近提交（patch 分支）

```
1a16983 chore(patch): patch 分支——fnOS 业务实现（patches/daemon/shim/构建链），不含 src
```

## 关键环境

- **工作目录**：D:\Projects\2026-SummerHoliday\ChmlFrpLauncher
- **fork**：CelestNya/ChmlFrpLauncher · 上游：TechCat-Team/ChmlFrpLauncher（v0.7.5 LTS 基线）
- **分支**：main（发布组合态）/ adapter/v0.7.5（fnos-api 契约）/ patch（业务实现，不含 src）；组合操作见 development.md §一
- **NAS**：192.168.12.32（SSH 凭据在 `C:\Users\CelestNya\AppData\Local\Temp\nas-*.py`）；访问 `https://nas.letian.us.kg/app/chmlfrp`
- **CI**：fnos-build.yml（组合矩阵构建：dispatch 选 adapter+patch，push main/patch 触发）/ fnos-upstream-follow.yml（hotfix 感知，v0.7.5 tag 不可变常态静默）

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
