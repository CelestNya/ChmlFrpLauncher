# 前后端分离重构 —— 完整开发计划（含「为什么不做完」分析）

> 最后更新：2026-08-13
> 范围：本次重构「做完」与「未完成」的完整边界。核心文档：`docs/architecture.md`（架构）、`docs/development.md`（开发须知）、`docs/adr/0001-0004`（决策）。
> 真机验证：`docs/plans/fnos-nas-test-checklist.md`（已归档全部通过）。

## 一、已做完（本次重构交付，全部验证通过）

| 阶段 | 内容 | 验证 |
|------|------|------|
| 0 | 领域模型（CONTEXT.md + ADR-0001~0004） | — |
| 1 | shim localStorage 重定向 + daemon credential/node_settings（0600）+ __FNOS_BOOT__ 首帧注入 | daemon 66 / vitest 46 全绿 |
| 2 | 壁纸文件化（dataURL→落盘→托管 URL）+ fnOS 砍轮播 | 真机：`/app/chmlfrp/assets/backgrounds/` HTTP 200 |
| 3 | 能力协商（GET /api/capabilities + SUPPORTED_COMMANDS）| 真机：capabilities 返回命令面 |
| 4 | patch 面验证（api.ts/frpcManager 零改动） | patch 干净上游可应用 |
| 5 | 全量回归 + 双分支 + 真机部署 | 见 checklist |
| review | P2 凭据完整往返（usergroup 会员门控）/ P3 壁纸 URL 网关前缀 / P5 NO_OP 计数 | 真机：usergroup=vip 往返 + 壁纸 200 |

**「做完」的定义**：本次重构（阶段 0-5 + review 修复）已全部实现并真机验证。**代码、测试、文档、部署四者闭环**。

## 二、「为什么不做完」—— 三类未完成，性质不同

### 类型 A：本次重构范围内，但明确决策「不做」的部分（已完成决策）

| 项 | 决策 | 理由 |
|----|------|------|
| `__FNOS__` → `isCapable()` 前端门控替换 | **不做** | 不回上游时门控工作正常，替换只增 patch 面（阶段 3 明确） |
| shim 启动探测 capabilities + `features` 键 | **部分不做** | ADR-0001 写「shim 探测缓存」，但消费侧无实际需求（前端不按能力自适应）；daemon 端点已备，消费侧留待真需要时补 |
| Standards smells S2（settings vs persist 重复）| **不修** | 照抄是刻意（注释明说），抽公共库反而增加耦合 |
| Standards smells S5（import_background_image_folder 保留）| **不修** | 桌面兼容需要（fnOS 砍轮播但桌面还用） |

### 类型 B：本次重构范围外，历史遗留（development.md §七）

| 项 | 说明 | 阻塞 |
|----|------|------|
| 自更新通道 | 依赖 GitHub Release asset | 等上游发版 |
| daemon 自保 | 崩溃无自拉起 | 应用中心层面，低优先 |
| OAuth CORS | iframe 下可能失败 | 实测正常，低优先 |
| UI 适配 | 标题栏裁切等 | 等用户反馈 |
| WS seq | 前端 B7 已兜底去重 | daemon 侧未做，低优先 |
| IndexedDB（M3）| localStorage 配额 | **部分已被壁纸路径化解决**（base64 不再进 localStorage）|

### 类型 C：可改进的 Standards smells（judgment call，本次未修）

| Smell | 位置 | 价值 | 建议 |
|-------|------|------|------|
| S1 Repeated Switches | shim 5-key if 链 ×3 | 低（可读性）| 需要时改 key→handler 表 |
| S3 Duplicated Code | invoke.rs copy 双臂 | 低（2 行）| 合并 `\|` 多模式 |
| S4 Data Clumps | NodeSettings 8 个 proxy 字段 | 中 | 嵌套 ProxyConfig struct |
| S6 Shotgun Surgery | 能力面三处同步 | 中 | 已文档化（ADR-0001），接受 |
| S7 Mysterious Name | save_background_image | 低 | 改名 save_background_data_url |

**为什么没做 C 类**：都是 judgment call（不破坏正确性），本次聚焦「实质缺陷」（P2/P3 安全与功能），smell 重构收益低、风险分散，留待后续。

## 三、剩余工作清单（按优先级）

### P0：无（本次全部完成）

### P1：能力协商消费侧（可选，等真需要）
- [ ] shim 启动 fetch `/api/capabilities` 缓存，暴露 `isCapable(cmd)`
- [ ] `GET /api/capabilities` 补 `features` 键（当前只有 commands）
- [ ] 前端按能力自适应（替代部分 `__FNOS__` 门控）——**仅当前端需要区分「daemon 有无某能力」时**，不为做而做

### P2：Standards smells 重构（可选）
- [ ] S3：invoke.rs copy_background_image/video 合并 `\|` 单臂
- [ ] S4：NodeSettings 嵌套 ProxyConfig struct（daemon + shim 同步）

### P3：历史遗留（development.md §七，外部依赖或低优先）
- [ ] 自更新通道（等上游 v0.8.0 release）
- [ ] daemon 自保
- [ ] WS seq（前端已兜底）
- [ ] UI 适配（等用户反馈）

## 四、后续里程碑

```
M1（本次）前后端分离重构 —— ✅ 完成
M2 上游自动跟随 CI（fnos-upstream-follow.yml）—— ✅ 完成（schedule 轮询 → merge → gen-patch → build-fpk → 自动 PR / 冲突 issue）
M3 上游发版后：跟随 tag 同步基线 → 发 v0.8.0 + attach fnOS bundle → 启用自更新
M4（可选）能力协商消费侧 + Standards 重构 —— 按需触发
M5（可选）桌面版数据层同步后端化（token 迁移 Rust）—— 当前 fnOS 先行，桌面跟随
```

**维护纪律**：每次改 fnOS 前端文件 → 登记 gen-patch（E6 拦截）；新增凭据/配置字段 → 三处同步（daemon struct + shim 映射 + WHITELIST/SUPPORTED_COMMANDS）。
