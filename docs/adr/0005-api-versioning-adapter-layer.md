# ADR-0005：fnOS API 版本化 + adapter 适配层（mod 架构）

**状态**: accepted
**决定**: 引入 `fnos-api/` 作为一等公民的接口契约层，版本化（semver）；每个上游版本一个 adapter（版本号 = API 版本号）；patch 业务逻辑对 API 版本硬依赖（`requires: {api: ">=X"}`）。

## 为什么

用户核心痛点：上游 ChmlFrp（服务面）开发活跃（90 天 14 提交、今天还开 PR #67），我们（NAS 适配面）怕「上游出新也跟着大改」。根因是 patch 直接 diff 上游文件内容（脆弱）。

**三层解耦**：
```
上游 chmlfrp（服务面，不可控）
    ↓ adapter 映射（每版本一个，版本号 = API 版本）
API 契约（fnos-api，可控，版本化）
    ↓ patch 业务逻辑（对 API 版本硬依赖）
fnOS 适配实现
```

**关键事实**（验证过）：0.7.5 与 0.8.0-dev 的命令面**完全一致**（45 命令 diff 零），差异集中在 api.ts 内部（token 刷新重构）与 UI 结构。→ adapter 的命令映射跨版本零工作量，版本差异在插件形态 + UI 适配点。

## 版本化规则

| 对象 | 版本号语义 | 依赖 |
|------|-----------|------|
| **API**（fnos-api）| 接口演进版本（semver）| — |
| **adapter**（每上游版本）| **= API 版本号**（声明适配的 API）| 上游版本 |
| **patch**（业务逻辑）| 功能版本 | `requires: {api: ">=X"}` 硬依赖 |

**semver**：major = 接口破坏性变更；minor = 新增接口；patch = 修复。

## 开发流程（当 patcher 有新需求时）

```
1. 更新 fnos-api（加接口/改契约）→ bump API 版本
2. adapter 适配新 API（对应上游版本）
3. patch 基于新 API 开发功能 → manifest 声明 requires: {api: ">=新版本"}
4. 构建时校验：patch 需要的接口 ⊆ adapter 提供的接口
   （semver 粗筛 + 能力清单精判）
5. 上游出新版本 → 新增 adapter（命令映射复用，差异登记）
```

**关键原则**：adapter 版本号是 API 版本号，不是上游版本号。上游更新 ≠ API 更新——只有「上游改动影响接口形态」才 bump API。上游改内部实现（如 token 刷新）→ adapter 登记差异但不 bump API。

## 被否决

- **能力清单双保险**（版本号 + 能力清单交集校验）：用户判定「不应该出现这类疏漏」（版本号声明已足够严谨），避免多一层清单维护
- **diff patch 承载全部**：保持脆弱性，不做
- **运行时 DOM 裁剪**：否决；实际采用 **adapter 配置驱动条件渲染**（2026-08-15 用户决策）——
  adapter manifest `uiConfig.values` 暴露统一格式配置项，patch 消费做 JSX 条件渲染，
  构建时注入配置（`generate-ui-config.mjs`），非运行时 DOM 操作

## 落地物

- `fnos-api/API.md` —— 契约人读版（六面：命令/事件/插件/存储/HTTP/UI）
- `fnos-api/adapters/v0.7.5/manifest.json` —— 第一个 adapter（能力面 42 implemented + 9 noop，UI 12 文件 + uiConfig 5 配置项）
- `fnos-api/verify-adapter.sh` —— 能力覆盖校验（前端调用 ⊆ adapter 能力面 + uiConfig 键覆盖 + patch 依赖校验）
- `fnos-api/generate-ui-config.mjs` —— 从 manifest 生成前端配置模块（构建链调用）
- 校验结果：前端 33 个实际调用命令全部覆盖 ✅

## 分支矩阵拓扑（2026-08-15 增补）

版本组合用 **git 分支**承载（三套代码生命周期分离）：

```
main ────────── 发布组合态：src（0 侵入，= adapter 声明的 upstream.ref）+ docs + CI
adapter/<ver> ── fnos-api 契约（每上游版本一个分支，upstream.ref 确定 src 基线）
patch ────────── 业务实现（patches/ + daemon + shim + 构建链，不含 src）
```

- **构建组合**：CI（fnos-build workflow_dispatch）选 adapter + patch 分支 → checkout 目录 → 0 侵入验证 → verify（patch requires.api ≤ adapter apiVersion 强制）→ apply-patches → build-fpk
- **src 根据 adapter 版本确定**：adapter manifest 的 `upstream.ref` 是唯一基线来源
- **patch 开发只关注 API 面**：patch 分支不含上游 src，开发/构建时由组合工作区提供基线（Linux patchset 模式）
- **版本号**：adapter 联合版本号（`{基线}-{apiVersion}`）、patch 单特性版本号（`featureVersion` + `requires.api`）
- 迁移：patcher 分支职责并入 patch 分支（原双分支模型废弃）
