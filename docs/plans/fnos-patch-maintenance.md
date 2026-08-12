# fnOS 前端 patch 机制与维护

> 形态 C（零改动补丁）的实践：主仓 `src/` 工作树保持零改动，
> 所有 fnOS 前端适配抽离为 patch，构建时在临时副本上应用。

## 结构

```
fnos-pack/patches/
├── fnos-ui-patch.patch        # UI 裁剪：删 TitleBar/杀软弹窗/自启/托盘设置（4 文件）
└── fnos-feature-patch.patch   # fnOS 功能：壁纸 dataURL/文件夹轮播、日志去重+清空联动、
                               # replay 通知跳过、音效网关前缀（5 文件）
```

| patch | 覆盖文件 | 内容 |
|---|---|---|
| ui-patch | `src/App.tsx`、`src/components/pages/Settings/index.tsx`、`.../AppearanceSection.tsx`、`.../SystemSection.tsx` | 删除 fnOS 不适用的 UI（标题栏、杀软弹窗、开机自启、托盘） |
| feature-patch | `src/components/pages/Settings/hooks/useBackgroundImage.ts`、`src/services/logStore.ts`、`src/lib/sound.ts`、`src/services/frpcManager.ts`、`src/components/App/hooks/useTunnelNotifications.ts` | fnOS 专属功能分支（`__FNOS__` 环境判断、replay 标记、dataURL 壁纸等） |

应用顺序：ui-patch 先，feature-patch 后。格式均为 git 双树（`a/`/`b/` 前缀），`patch -p1` 应用。

## 维护流程

### 新增/修改 fnOS 前端功能
1. 在 `src/` 直接改（此时是工作树改动）
2. 改的是 ui-patch 文件 → `git diff HEAD -- <该文件>` 更新 `fnos-ui-patch.patch`
3. 改的是 feature-patch 文件 → `git diff HEAD -- <该文件>` 更新 `fnos-feature-patch.patch`
4. **revert 工作树**：`git checkout HEAD -- <改过的文件>`
5. 验证：`bash fnos-pack/apply-patches.sh`（dry-run 预检 + 构建），确认 dist 产物含新逻辑

### 上游同步（主仓更新时）
- 上游改动这 9 个文件 → patch 应用失败 → apply-patches.sh 的 **dry-run 预检**会明确报错
  （"请更新 fnos-ui-patch.patch / fnos-feature-patch.patch"），不会静默产出残缺产物
- 修法：合并 patch 到最新上游状态（或用 `git diff <新基线>` 重新生成）
- 上游新增 tauri 命令 / Tauri API 调用 → 需补 `fnos-daemon/src/invoke.rs` 命令 + `fnos-shim/tauri-shim.ts` 降级

## 验证

`apply-patches.sh` 内置两段验证（失败即 exit 1）：
- UI 精简：`src/App.tsx` 不再含 `TitleBar` / `AntivirusWarningDialog`
- feature：`useBackgroundImage.ts` 含 `__FNOS__`、`useTunnelNotifications.ts` 含 `replay`
