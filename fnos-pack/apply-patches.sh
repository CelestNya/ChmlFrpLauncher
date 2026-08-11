#!/usr/bin/env bash
# fnos-pack/apply-patches.sh
# fnOS 构建链：在临时副本上应用 fnos-ui-patch.patch（源码级），随后构建前端并注入 shim。
# 源码副本不落盘到仓库（形态 C：工作树 src/ 零改动）。
#
# 用法（仓库根目录执行）:
#   bash fnos-pack/apply-patches.sh            # 产物输出到 dist-fnos/
#   OUTPUT_DIR=/tmp/x bash fnos-pack/apply-patches.sh
#
# 依赖: patch / pnpm / node
# 本脚本被 B3（.fpk 打包）与 B4（CI）复用。

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PATCH_FILE="$REPO_ROOT/fnos-pack/patches/fnos-ui-patch.patch"
OUTPUT_DIR="${OUTPUT_DIR:-$REPO_ROOT/dist-fnos}"
TMP_WORK="$(mktemp -d)"
trap 'rm -rf "$TMP_WORK"' EXIT

echo "[fnos-pack] 复制源码到临时副本…"
cp -r "$REPO_ROOT/src" "$TMP_WORK/src"
cp "$REPO_ROOT/package.json" "$REPO_ROOT/index.html" "$REPO_ROOT/tsconfig.json" \
   "$REPO_ROOT/tsconfig.app.json" "$REPO_ROOT/tsconfig.node.json" \
   "$REPO_ROOT/vite.config.ts" "$TMP_WORK/" 2>/dev/null || true

# patch 可能涉及的文件（其余依赖源码文件保持原样）
for f in \
  src/App.tsx \
  src/components/pages/Settings/index.tsx \
  src/components/pages/Settings/components/AppearanceSection.tsx \
  src/components/pages/Settings/components/SystemSection.tsx; do
  cp "$REPO_ROOT/$f" "$TMP_WORK/$f"
done

echo "[fnos-pack] 预检 patch（--dry-run）…"
if ! patch -p1 --dry-run -d "$TMP_WORK" < "$PATCH_FILE"; then
  echo "[fnos-pack] ❌ patch 预检失败：请更新 fnos-pack/patches/fnos-ui-patch.patch 以匹配当前 src/" >&2
  exit 1
fi

echo "[fnos-pack] 应用 patch…"
patch -p1 -d "$TMP_WORK" < "$PATCH_FILE"
echo "[fnos-pack] ✅ patch 应用成功"

# 验证：fnOS 已移除的元素不应再出现在源码副本中；保留元素应仍在
if grep -q "TitleBar" "$TMP_WORK/src/App.tsx"; then
  echo "[fnos-pack] ❌ App.tsx 仍引用 TitleBar" >&2
  exit 1
fi
if grep -q "AntivirusWarningDialog" "$TMP_WORK/src/App.tsx"; then
  echo "[fnos-pack] ❌ App.tsx 仍引用 AntivirusWarningDialog" >&2
  exit 1
fi
echo "[fnos-pack] ✅ UI 精简验证通过（TitleBar / 杀软警告已移除）"

echo "[fnos-pack] 构建前端…"
(
  cd "$TMP_WORK"
  # 使用仓库的 node_modules（pnpm store 链接）
  ln -s "$REPO_ROOT/node_modules" node_modules
  node node_modules/typescript/bin/tsc -b tsconfig.app.json --force --pretty false
  node node_modules/vite/bin/vite.js build
)

echo "[fnos-pack] 注入 shim…"
node "$REPO_ROOT/fnos-shim/build-shim.ts" --dist "$TMP_WORK/dist"

# 产物复制到输出目录
rm -rf "$OUTPUT_DIR"
mkdir -p "$OUTPUT_DIR"
cp -r "$TMP_WORK/dist/." "$OUTPUT_DIR/"
echo "[fnos-pack] ✅ 完成：产物位于 $OUTPUT_DIR"
