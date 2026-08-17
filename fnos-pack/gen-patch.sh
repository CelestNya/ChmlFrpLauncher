#!/usr/bin/env bash
# fnos-pack/gen-patch.sh
# 从组合工作区生成 fnOS 前端 patch（crop + feature），双输出源（2026-08-16 决策）：
#   crop patch（UI 裁剪）→ fnos-api/adapters/<ver>/ui-crop.patch（提交 adapter 分支）
#   feature patch（功能）→ fnos-pack/patches/fnos-feature-patch.patch（提交 patch 分支）
# 文件清单从 adapter manifest 的 uiFiles 字段读（配置驱动：清单权威归 adapter）。
#
# 用法（组合工作区：src 为应用态，仓库根目录执行）:
#   bash fnos-pack/gen-patch.sh            # 用 origin/main（fork 主分支）作基线
#   BASE=upstream/v0.7.5 bash fnos-pack/gen-patch.sh   # CI 用上游 v0.7.5 作基线（LTS）
#
# 基线 = fork 主分支（其 src/ 与上游 v0.7.5 一致，0 侵入不变式）。
# CI 中显式传 BASE=upstream/v0.7.5 作防御（防止 fork main 意外被污染 src/）。
# 生成前建议先 git fetch，保证 patch 基于最新上游，上游改动与 fnOS 改动的冲突提前暴露。
#
# 产出两个全量 patch（从干净上游可应用，幂等覆盖），提交到各自分支。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

BASE="${BASE:-origin/main}"
PATCH_DIR="$REPO_ROOT/fnos-pack/patches"
MANIFEST_PATH="$REPO_ROOT/fnos-api/adapters/v0.7.5/manifest.json"

# 清单权威 = adapter manifest 的 uiFiles（uiCrop = 裁剪文件 / featureBranch = 功能文件）
if [ ! -f "$MANIFEST_PATH" ]; then
  echo "[gen-patch] ❌ 缺少 adapter manifest（组合态缺失：请先 checkout adapter 分支的 fnos-api/）" >&2
  exit 1
fi
PY_MANIFEST="$MANIFEST_PATH"
if command -v cygpath >/dev/null 2>&1; then
  PY_MANIFEST=$(cygpath -w "$MANIFEST_PATH")
fi
mapfile -t UI_FILES < <(python3 -c "import json,sys; m=json.load(open(sys.argv[1])); print('\n'.join(m['uiFiles']['uiCrop']))" "$PY_MANIFEST" | tr -d '\r')
mapfile -t FEATURE_FILES < <(python3 -c "import json,sys; m=json.load(open(sys.argv[1])); print('\n'.join(m['uiFiles']['featureBranch']))" "$PY_MANIFEST" | tr -d '\r')
# crop 输出路径（adapter manifest 的 uiCropPatch 声明）
CROP_OUT="$REPO_ROOT/fnos-api/$(python3 -c "import json,sys; m=json.load(open(sys.argv[1])); print(m['uiCropPatch'])" "$PY_MANIFEST" | tr -d '\r')"

echo "[gen-patch] 基线: $BASE"
git rev-parse --verify "$BASE" >/dev/null 2>&1 || {
  echo "[gen-patch] ❌ 基线 $BASE 不存在，先 git fetch origin main" >&2
  exit 1
}

# 组合态检查：crop 输出写 adapter 目录、feature 输出写 patch 目录，
# 生成后分别在对应分支提交（改裁剪 → 提交 adapter/v0.7.5；改功能 → 提交 patch）
if [ ! -d "$PATCH_DIR" ]; then
  echo "[gen-patch] ⚠️ 缺少 fnos-pack/patches/（组合态缺失：请先 checkout patch 分支的 fnos-pack/）" >&2
  exit 1
fi

# 生成 crop patch（UI 裁剪，git 双树格式，patch -p1 可应用）
git diff "$BASE" -- "${UI_FILES[@]}" > "$CROP_OUT"
# E9：crop 手工维护项保护（2026-08-18 二次回归教训）——crop 的差异源在 patcher，
# 但 adapter 分支有手工覆盖（App.tsx shouldPadTop=: true 等，patcher 无此代码）。
# gen-patch 全量重新生成会静默冲掉手工项——生成后必须校验保留，丢失即报错拦截。
if grep -q "内容区仍保留 padding" "$CROP_OUT" 2>/dev/null; then
  :
else
  echo "[gen-patch] ❌ crop 丢失手工维护项（App.tsx shouldPadTop=: true，标记'内容区仍保留 padding'）" >&2
  echo "[gen-patch]    从 adapter 分支恢复手工段后重试（勿直接覆盖提交）" >&2
  exit 1
fi
# 生成 feature patch
git diff "$BASE" -- "${FEATURE_FILES[@]}" > "$PATCH_DIR/fnos-feature-patch.patch"

echo "[gen-patch] ✅ 已生成:"
echo "  $CROP_OUT       ($(grep -c '^---' "$CROP_OUT") 文件，提交 adapter 分支)"
echo "  fnos-feature-patch.patch  ($(grep -c '^---' "$PATCH_DIR/fnos-feature-patch.patch") 文件，提交 patch 分支)"

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
UNTRACKED="$(git status --porcelain src/ | grep '^??' | grep -v 'fnos-ui-config.ts' || true)"
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
for p in "$CROP_OUT" "$PATCH_DIR/fnos-feature-patch.patch"; do
  if ! patch -p1 --dry-run < "$p" >/dev/null 2>&1; then
    echo "[gen-patch] ❌ $p 无法从干净基线应用，请检查" >&2
    exit 1
  fi
done
echo "[gen-patch] ✅ 自检通过：两个 patch 均可从 $BASE 干净应用"
