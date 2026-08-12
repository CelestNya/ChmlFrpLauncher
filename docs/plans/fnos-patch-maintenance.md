# fnOS 前端双分支 patch 机制与维护

> 形态 C（零改动补丁）的实践：**main 分支 src/ 保持零改动**，fnOS 前端适配
> 全部以 patch 形式存在；开发在 **patcher 分支**（侵入式），patch 由 CI 自动生成。

## 双分支模型

```
main ──── 纯净上游 + patch 文件 + 构建链（apply-patches.sh / build-fpk.sh / gen-patch.sh / CI）
              ↑ PR（仅 patches/ 目录，CI 自动开）
patcher ── main + src/ 上的侵入式 fnOS 改动（开发态）
```

| 分支 | src/ 状态 | 用途 |
|---|---|---|
| `main` | 零改动（0 侵入），内容 = 上游 develop + fnos 附加文件 | 发布态：CI 构建 .fpk 时在临时副本应用 patch |
| `patcher` | 含全部 fnOS 改动（10 文件） | 开发态：改 fnOS 前端功能直接改这里 |

> **基线说明（2026-08-12 切换）**：上游活跃开发分支是 `develop`（v0.8.0 WIP），
> 发布才合并进上游 main。我们的 fork 基线 = **上游 develop**，main 的 src/ 与之
> 保持一致（0 侵入不变式）。patchgen CI 显式以 `upstream/develop` 作 diff 基线。

## Patch 结构

```
fnos-pack/patches/
├── fnos-ui-patch.patch        # UI 裁剪：删 TitleBar/杀软弹窗/自启/托盘（4 文件）
└── fnos-feature-patch.patch   # fnOS 功能：壁纸/日志/通知/音效/replay（6 文件）
```

两个 patch 都是**全量**（从干净上游可应用），由 `gen-patch.sh` 从 patcher 分支
相对上游 develop 的差异生成，幂等覆盖。

## 日常开发流程（改 fnOS 前端功能）

1. 切到 `patcher` 分支，直接在 `src/` 改 + commit
2. 本地可选：`bash fnos-pack/gen-patch.sh` 预览产物
3. `git push origin patcher` → CI 自动生成 patch → 开 PR（仅 patches/ 目录）
4. review PR 后合并到 main

## 上游同步（主仓更新时）

```
main:   git fetch upstream && git rebase --onto upstream/develop upstream/main main（+ force push）
patcher: git rebase --onto main origin/main patcher（冲突在 patcher 人工解决，main 永远干净）
         → 修改后 push patcher → CI 重新生成 patch
```

- 上游改动这 10 个文件 → patch 应用失败 → apply-patches.sh 的 **dry-run 预检**明确报错，
  不会静默产出残缺产物
- 上游新增 tauri 命令 / Tauri API 调用 → 需补 `fnos-daemon/src/invoke.rs` 命令 +
  `fnos-shim/tauri-shim.ts` 降级

## CI

- `.github/workflows/fnos-build.yml` — 构建 .fpk（tag v*，临时副本应用 patch；双架构 x86_64 + aarch64）
- `.github/workflows/fnos-patchgen.yml` — push patcher 时自动生成 patch + 开 PR（固定分支 `fnos-patch`，幂等跳过已存在 PR；diff 基线 = `upstream/develop`）

## 当前进度（2026-08-12）

- **基线切换上游 develop**（v0.8.0 WIP）：main 39 个 fnos 提交 rebase 到 develop 上，
  0 侵入不变式复验通过；patcher 历史精简为 6 个提交，src/ 侵入改动（10 文件）完整保留
- **修复 patch 漏登记 bug**：`useTunnelProgress.ts` 此前不在 gen-patch.sh 的 FEATURE_FILES，
  其 replay 跳过修复从未进入 CI 产物——日志重复修复不生效的根因。现已补入（feature patch = 6 文件）
- **本地全链验证**：apply-patches 构建 ✅（依赖已按 develop lockfile 重装）、eslint ✅、
  fnos-daemon 测试 4/4 ✅；shim 移除未用参数适配 develop 新 eslint 配置
- **PR #12**（fnos-patch 分支）：CI 重新生成 develop 基线 patch 后待 review 合并
- **真机已验证**：进程守护自动重启、手动停止不复活、WS 日志实时回显、音效
- **日志重复**：根因 = useTunnelProgress 缺 replay 跳过 + logStore 实时帧未进指纹集合；
  第二层修复因 patch 漏登记从未生效，本次切换后重新生成 patch 才真正进产物
- **登录态掉线**（桌面版同问题）：localStorage 持久但界面重登。develop 有「移除自动登录逻辑」
  提交（884c45b），与上游对齐后**待重新评估**

## 验证

`apply-patches.sh` 内置两段验证（失败即 exit 1）：
- UI 精简：`src/App.tsx` 不再含 `TitleBar` / `AntivirusWarningDialog`
- feature：`useBackgroundImage.ts` 含 `__FNOS__`、`useTunnelNotifications.ts` 含 `replay`

`gen-patch.sh` 内置自检：两个 patch 可从基线干净应用。
