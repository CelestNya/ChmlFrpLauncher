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
| `main` | 零改动（0 侵入） | 发布态：CI 构建 .fpk 时在临时副本应用 patch |
| `patcher` | 含全部 fnOS 改动（9 文件） | 开发态：改 fnOS 前端功能直接改这里 |

## Patch 结构

```
fnos-pack/patches/
├── fnos-ui-patch.patch        # UI 裁剪：删 TitleBar/杀软弹窗/自启/托盘（4 文件）
└── fnos-feature-patch.patch   # fnOS 功能：壁纸/日志/通知/音效（5 文件）
```

两个 patch 都是**全量**（从干净上游可应用），由 `gen-patch.sh` 从 patcher 分支
相对上游 main 的差异生成，幂等覆盖。

## 日常开发流程（改 fnOS 前端功能）

1. 切到 `patcher` 分支，直接在 `src/` 改 + commit
2. 本地可选：`bash fnos-pack/gen-patch.sh` 预览产物
3. `git push origin patcher` → CI 自动生成 patch → 开 PR（仅 patches/ 目录）
4. review PR 后合并到 main

## 上游同步（主仓更新时）

```
main:   git pull origin main（或 fetch upstream main 后 merge）
patcher: git rebase origin/main（冲突在 patcher 人工解决，main 永远干净）
         → 修改后 push patcher → CI 重新生成 patch
```

- 上游改动这 9 个文件 → patch 应用失败 → apply-patches.sh 的 **dry-run 预检**明确报错，
  不会静默产出残缺产物
- 上游新增 tauri 命令 / Tauri API 调用 → 需补 `fnos-daemon/src/invoke.rs` 命令 +
  `fnos-shim/tauri-shim.ts` 降级

## CI

- `.github/workflows/fnos-build.yml` — 构建 .fpk（tag v*，临时副本应用 patch）
- `.github/workflows/fnos-patch-gen.yml` — push patcher 时自动生成 patch + 开 PR

## 验证

`apply-patches.sh` 内置两段验证（失败即 exit 1）：
- UI 精简：`src/App.tsx` 不再含 `TitleBar` / `AntivirusWarningDialog`
- feature：`useBackgroundImage.ts` 含 `__FNOS__`、`useTunnelNotifications.ts` 含 `replay`

`gen-patch.sh` 内置自检：两个 patch 可从基线干净应用。
