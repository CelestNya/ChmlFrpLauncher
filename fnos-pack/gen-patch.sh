#!/usr/bin/env bash
# fnos-pack/gen-patch.sh
# 从 patcher 分支生成 fnOS 前端 patch（ui + feature），覆盖 fnos-pack/patches/。
#
# 用法（patcher 分支，仓库根目录执行）:
#   bash fnos-pack/gen-patch.sh            # 用 origin/main（fork 主分支）作基线
#   BASE=upstream/v0.7.5 bash fnos-pack/gen-patch.sh   # CI 用上游 v0.7.5 作基线（LTS）
#
# 基线 = fork 主分支（其 src/ 与上游 v0.7.5 一致，0 侵入不变式）。
# CI 中显式传 BASE=upstream/v0.7.5 作防御（防止 fork main 意外被污染 src/）。
# 生成前建议先 git fetch，保证 patch 基于最新上游，上游改动与 fnOS 改动的冲突提前暴露。
#
# 产出两个全量 patch（从干净上游可应用，幂等覆盖）：
#   fnos-pack/patches/fnos-ui-patch.patch       # UI 裁剪（4 文件）
#   fnos-pack/patches/fnos-feature-patch.patch  # fnOS 功能（5 文件）

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

BASE="${BASE:-origin/main}"
PATCH_DIR="$REPO_ROOT/fnos-pack/patches"

# 按 patch 分组的文件清单（新增 fnOS 前端功能时在此登记）
UI_FILES=(
  src/App.tsx
  src/components/pages/Settings/index.tsx
  src/components/pages/Settings/components/AppearanceSection.tsx
  src/components/pages/Settings/components/SystemSection.tsx
)
FEATURE_FILES=(
  src/components/pages/Settings/hooks/useBackgroundImage.ts
  src/services/logStore.ts
  src/lib/sound.ts
  src/services/frpcManager.ts
  src/components/App/hooks/useTunnelNotifications.ts
  src/components/pages/TunnelList/hooks/useTunnelProgress.ts
  src/components/pages/Logs.tsx
  src/components/App/hooks/useAppInitialization.ts
  src/components/pages/Settings/hooks/useProcessGuard.ts
)

echo "[gen-patch] 基线: $BASE"
git rev-parse --verify "$BASE" >/dev/null 2>&1 || {
  echo "[gen-patch] ❌ 基线 $BASE 不存在，先 git fetch origin main" >&2
  exit 1
}

# 确保 patcher 分支在基线上（校验 src/ 差异可生成）
if [ "$(git rev-parse --abbrev-ref HEAD)" != "patcher" ]; then
  echo "[gen-patch] ⚠️ 当前分支非 patcher，请切换到 patcher 再生成" >&2
  exit 1
fi

# 生成 ui patch（git 双树格式，patch -p1 可应用）
git diff "$BASE" -- "${UI_FILES[@]}" > "$PATCH_DIR/fnos-ui-patch.patch"
# 生成 feature patch
git diff "$BASE" -- "${FEATURE_FILES[@]}" > "$PATCH_DIR/fnos-feature-patch.patch"

echo "[gen-patch] ✅ 已生成:"
echo "  fnos-ui-patch.patch       ($(grep -c '^---' "$PATCH_DIR/fnos-ui-patch.patch") 文件)"
echo "  fnos-feature-patch.patch  ($(grep -c '^---' "$PATCH_DIR/fnos-feature-patch.patch") 文件)"

# E6：差异完整性自检——src/ 相对基线的全部差异必须被登记清单覆盖
#（useTunnelProgress 教训：漏登记 = 修复不进产物且 dry-run 自检仍通过）。
# 豁免：
#   *.test.ts —— 测试文件是 patcher 独有（不进 patch、不进构建产物，main 保持 0 侵入）
#   src/fnos-ui-config.ts —— 生成文件（adapter manifest 驱动，apply-patches.sh 生成覆盖，不进 patch）
UNREGISTERED="$(git diff --name-only "$BASE" -- src/ \
    | grep -v '\.test\.ts$' \
    | grep -v '^src/fnos-ui-config\.ts$' \
    | grep -vxF -f <(printf '%s\n' "${UI_FILES[@]}" "${FEATURE_FILES[@]}") || true)"
if [ -n "$UNREGISTERED" ]; then
    echo "[gen-patch] ❌ src/ 存在未登记的差异文件（漏 patch！须加入 UI_FILES/FEATURE_FILES）:" >&2
    echo "$UNREGISTERED" | sed 's/^/  /' >&2
    exit 1
fi
UNTRACKED="$(git status --porcelain src/ | grep '^??' || true)"
if [ -n "$UNTRACKED" ]; then
    echo "[gen-patch] ❌ src/ 存在未跟踪文件（git diff 不捕获、不会进 patch）:" >&2
    echo "$UNTRACKED" | sed 's/^/  /' >&2
    exit 1
fi

# 自检：从干净的 src 应用是否通过（用临时目录验证）
echo "[gen-patch] 自检 patch 可应用…"
TMP_CHECK="$(mktemp -d)"
trap 'rm -rf "$TMP_CHECK"' EXIT
for f in "${UI_FILES[@]}" "${FEATURE_FILES[@]}"; do
  mkdir -p "$TMP_CHECK/$(dirname "$f")"
  git show "$BASE:$f" > "$TMP_CHECK/$f" 2>/dev/null || true
done
cd "$TMP_CHECK"
for p in "$PATCH_DIR/fnos-ui-patch.patch" "$PATCH_DIR/fnos-feature-patch.patch"; do
  if ! patch -p1 --dry-run < "$p" >/dev/null 2>&1; then
    echo "[gen-patch] ❌ $p 无法从干净基线应用，请检查" >&2
    exit 1
  fi
done
echo "[gen-patch] ✅ 自检通过：两个 patch 均可从 $BASE 干净应用"
