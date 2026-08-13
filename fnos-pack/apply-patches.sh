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
# 纯函数库（E1 目录校验等，可独立单测：bash fnos-pack/tests/lib.test.sh）
source "$REPO_ROOT/fnos-pack/lib.sh"
PATCH_FILE="$REPO_ROOT/fnos-pack/patches/fnos-ui-patch.patch"
FEATURE_PATCH_FILE="$REPO_ROOT/fnos-pack/patches/fnos-feature-patch.patch"
OUTPUT_DIR="${OUTPUT_DIR:-$REPO_ROOT/dist-fnos}"
# E1：输出目录边界校验（本脚本会 rm -rf 该目录，防误删仓库）
validate_output_dir "$OUTPUT_DIR" "$REPO_ROOT" || exit 1
TMP_WORK="$(mktemp -d)"
trap 'rm -rf "$TMP_WORK"' EXIT

echo "[fnos-pack] 复制源码到临时副本…"
cp -r "$REPO_ROOT/src" "$TMP_WORK/src"
# E2：构建所需文件显式检查后裸 cp（缺失即失败，不再 || true 静默——
# public/ 缺失时 vite 构建仍会成功、产物静默缺音效等文件）
for f in package.json index.html tsconfig.json tsconfig.app.json tsconfig.node.json vite.config.ts; do
    if [ ! -f "$REPO_ROOT/$f" ]; then
        echo "[fnos-pack] ❌ 缺少构建配置文件: $f" >&2
        exit 1
    fi
    cp "$REPO_ROOT/$f" "$TMP_WORK/"
done
# public/ 静态资源（音效 mp3 等）：不复制则 vite 构建产物缺文件
if [ ! -d "$REPO_ROOT/public" ]; then
    echo "[fnos-pack] ❌ 缺少 public/ 静态资源目录" >&2
    exit 1
fi
cp -r "$REPO_ROOT/public" "$TMP_WORK/public"

# patch 可能涉及的文件（其余依赖源码文件保持原样）

echo "[fnos-pack] 预检 patch（--dry-run）…"
if ! patch -p1 --dry-run -d "$TMP_WORK" < "$PATCH_FILE"; then
  echo "[fnos-pack] ❌ UI patch 预检失败：请更新 fnos-pack/patches/fnos-ui-patch.patch 以匹配当前 src/" >&2
  exit 1
fi
if ! patch -p1 --dry-run -d "$TMP_WORK" < "$FEATURE_PATCH_FILE"; then
  echo "[fnos-pack] ❌ feature patch 预检失败：请更新 fnos-pack/patches/fnos-feature-patch.patch 以匹配当前 src/" >&2
  exit 1
fi

echo "[fnos-pack] 应用 patch…"
patch -p1 -d "$TMP_WORK" < "$PATCH_FILE"
patch -p1 -d "$TMP_WORK" < "$FEATURE_PATCH_FILE"
echo "[fnos-pack] ✅ patch 应用成功（ui + feature）"

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

# 验证 feature patch 关键修复已应用（fnOS 专属逻辑存在性）
if ! grep -q "__FNOS__" "$TMP_WORK/src/components/pages/Settings/hooks/useBackgroundImage.ts"; then
  echo "[fnos-pack] ❌ feature patch 未生效：useBackgroundImage.ts 缺 __FNOS__" >&2
  exit 1
fi
if ! grep -q "replay" "$TMP_WORK/src/components/App/hooks/useTunnelNotifications.ts"; then
  echo "[fnos-pack] ❌ feature patch 未生效：useTunnelNotifications.ts 缺 replay" >&2
  exit 1
fi
echo "[fnos-pack] ✅ feature patch 验证通过（__FNOS__ / replay 存在）"

echo "[fnos-pack] 构建前端…"
(
  cd "$TMP_WORK"
  # 使用仓库的 node_modules（pnpm store 链接）
  ln -s "$REPO_ROOT/node_modules" node_modules
  node node_modules/typescript/bin/tsc -b tsconfig.app.json --force --pretty false
  # MSYS_NO_PATHCONV=1：防止 Git Bash 把 /app/chmlfrp/ 转成 Windows 路径
  MSYS_NO_PATHCONV=1 node node_modules/vite/bin/vite.js build --base=/app/chmlfrp/
)

echo "[fnos-pack] 注入 shim…"
node "$REPO_ROOT/fnos-shim/build-shim.mjs" --dist "$TMP_WORK/dist"

# 产物输出：原子交换（E1）——构建到临时目录后整体换入，失败回滚旧产物，
# 杜绝「rm→mkdir→cp 中途失败留下残缺 dist-fnos」被 --skip-frontend 复用
rm -rf "$OUTPUT_DIR.old" 2>/dev/null || true
if [ -d "$OUTPUT_DIR" ]; then
    mv "$OUTPUT_DIR" "$OUTPUT_DIR.old"
fi
mkdir -p "$OUTPUT_DIR"
if ! cp -r "$TMP_WORK/dist/." "$OUTPUT_DIR/"; then
    echo "[fnos-pack] ❌ 产物复制失败，回滚旧产物" >&2
    rm -rf "$OUTPUT_DIR"
    if [ -d "$OUTPUT_DIR.old" ]; then
        mv "$OUTPUT_DIR.old" "$OUTPUT_DIR"
    fi
    exit 1
fi
rm -rf "$OUTPUT_DIR.old" 2>/dev/null || true
# E8：构建完成戳记（build-fpk.sh --skip-frontend 据此判断产物是否新鲜完整）
touch "$OUTPUT_DIR/.build-ok"
echo "[fnos-pack] ✅ 完成：产物位于 $OUTPUT_DIR"
