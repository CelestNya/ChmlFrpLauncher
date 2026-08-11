# 文档索引

ChmlFrpLauncher 项目文档。

| 文档 | 职责 | 更新时机 |
|------|------|----------|
| [architecture.md](architecture.md) | 当前架构完整描述（系统全貌 / Rust 后端 / 前端 / 外部契约 / 事件通道 / 存储 / 可观测性 / 待商榷） | 架构发生实质变更时（新模块、契约变化、形态演进），由变更作者同步更新 |
| [fnos-porting-plan.md](fnos-porting-plan.md) | fnOS 移植整体 plan（决策记录 Q1-Q5 / 批次 B1-B5 / 待验证清单 / 风险） | grill 决议或批次实施推进时更新 |
| [plans/b1-fnos-daemon.md](plans/b1-fnos-daemon.md) | B1 实施计划（fnos-daemon crate：形态/契约/命令范围/验收/commit 点位） | 批次实施时逐项勾选 |
| [plans/b2-fnos-shim-patch.md](plans/b2-fnos-shim-patch.md) | B2 实施计划（fnos-shim + UI patch：shim 契约/构建链/验收/已知降级） | 批次实施时逐项勾选 |

## 约定

- 架构文档遵循「mermaid 是完整设计，节点带状态标记」约束，章节标题不写状态标记；文本陈述当前状态。
- 状态标记：✅ 已实现 / ❌ 意向设计 / 🔄 存在但待变更 / ⚠️ 条件不具备。
- 外部契约（ChmlFrp API / OAuth / 下载源）变更时优先更新 architecture.md §四，涉及行为变更需配套测试。
- 变更流程：见 grill 产出的开发 plan（开发流程框架落定后补充本节）。