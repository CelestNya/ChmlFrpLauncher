# ADR-0001：契约收敛 + 能力协商（替代 NO_OP）

**状态**: accepted
**决定**: daemon 命令面从「桌面并集 + 13 个 NO_OP」收成「能力面 + 探测」。
daemon 暴露 `GET /api/capabilities` 返回 `{commands, features}`，shim 启动时
探测并缓存，前端按能力自适应；NO_OP 命令从分发表删除，返回明确「不支持」。

**为什么**: 现状 daemon 对 13 个桌面命令（`hide_window`/`read_image_folder`/
`copy_background_image`/`is_autostart_enabled` 等）假装支持实则 no-op
（`fnos-daemon/src/invoke.rs:383-397`），前端无从区分「真功能」与「假支持」。
能力协商把「后端有什么」变成显式契约，前端据此决定 UI 入口与降级路径，
也为将来消灭 `__FNOS__` 硬编码门控铺路。

**被否决**: 双仓库（永久合并税）与「破坏性改动全部回上游」（上游活跃开发
v0.8.0，且桌面版无前后端分离设计，坑多）。本重构不回上游，全部走 patch +
shim 重定向。
