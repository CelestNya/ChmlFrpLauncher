# 文档索引

ChmlFrpLauncher 项目文档（桌面版 + fnOS 移植）。

## 核心文档（仓库持久，随代码变更维护）

| 文档 | 职责 | 更新时机 |
|------|------|----------|
| [architecture.md](architecture.md) | **架构**——系统全貌 / Rust 后端 / 前端 / 外部契约 / 事件通道 / 存储 / fnOS 决策 | 架构实质变更时（新模块、契约变化、形态演进） |
| [development.md](development.md) | **开发须知**——双分支工作流 / patch 机制 / 构建链 / NAS 部署 / 测试 / 发版策略 / 遗留 | 流程、工具、部署方式变更时 |
| [handoff.md](handoff.md) | **交接**——当前状态 / 待决策 / 已完成里程碑 / 关键环境 | 每次会话结束时（随代码变更实时更新） |
| [fnos-api/API.md](../fnos-api/API.md) | **fnOS API 契约**——六面（命令/事件/插件/存储/HTTP/UI），版本化 | API 接口变更时（bump apiVersion） |

## 辅助文档

| 文档 | 职责 |
|------|------|
| [plans/fnos-nas-test-checklist.md](plans/fnos-nas-test-checklist.md) | NAS 真机测试记录（验证结果 + 实测问题 P1/P2/P2.5 + 遗留 P3） |

## 归档说明

一次性实施计划（B1-B5 fnos-daemon/shim/pack/update）、代码审查报告、移植 plan 已完成使命，
**不占仓库位置**——git 历史可查，副本在系统临时目录 `chmlfrp-archived-plans/`。

## 约定

- 架构文档遵循「mermaid 是完整设计，节点带状态标记」约束；状态标记：✅ 已实现 / ❌ 意向设计 / 🔄 存在但待变更 / ⚠️ 条件不具备。
- 外部契约（ChmlFrp API / OAuth / 下载源）变更时优先更新 architecture.md §四，涉及行为变更需配套测试。
- 新增 src/ 前端改动须登记 gen-patch.sh 的 FEATURE_FILES（否则 patch 不生成，修复不进产物）。
