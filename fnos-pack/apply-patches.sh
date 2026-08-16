#!/usr/bin/env bash
# fnos-pack/apply-patches.sh
# fnOS 构建链：在临时副本上应用 feature patch（patch 分支）+ crop patch（adapter 分支），
# 随后构建前端并注入 shim。UI 裁剪归 adapter（2026-08-16 决策）：crop 路径从
# adapter manifest 的 uiCropPatch 字段读，构建期应用，patch 分支零裁剪代码。
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
# 功能 patch（patch 分支持有）
FEATURE_PATCH_FILE="$REPO_ROOT/fnos-pack/patches/fnos-feature-patch.patch"
# UI 裁剪 patch（adapter 分支持有；路径从 manifest 读）
ADAPTER_MANIFEST="$REPO_ROOT/fnos-api/adapters/v0.7.5/manifest.json"
if [ ! -f "$ADAPTER_MANIFEST" ]; then
    echo "[fnos-pack] ❌ 缺少 adapter manifest（组合态缺失：请先 checkout adapter 分支的 fnos-api/）" >&2
    exit 1
fi
# node -e 内嵌代码里的 /d/ 路径不走 MSYS 参数转换 → cygpath 转 Windows 路径（Windows 下）
ADAPTER_MANIFEST_ABS="$ADAPTER_MANIFEST"
if command -v cygpath >/dev/null 2>&1; then
    ADAPTER_MANIFEST_ABS=$(cygpath -w "$ADAPTER_MANIFEST")
fi
CROP_PATCH=$(node -e "console.log(require(process.argv[1]).uiCropPatch || '')" "$ADAPTER_MANIFEST_ABS")
if [ -z "$CROP_PATCH" ]; then
    echo "[fnos-pack] ❌ adapter manifest 未声明 uiCropPatch" >&2
    exit 1
fi
PATCH_FILE="$REPO_ROOT/fnos-api/$CROP_PATCH"
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
if ! patch -p1 --dry-run -d "$TMP_WORK" < "$FEATURE_PATCH_FILE"; then
  echo "[fnos-pack] ❌ feature patch 预检失败：请更新 fnos-pack/patches/fnos-feature-patch.patch 以匹配当前 src/" >&2
  exit 1
fi
if ! patch -p1 --dry-run -d "$TMP_WORK" < "$PATCH_FILE"; then
  echo "[fnos-pack] ❌ crop patch 预检失败：请更新 adapter 分支的 $CROP_PATCH 以匹配当前 src/" >&2
  exit 1
fi

echo "[fnos-pack] 应用 patch…"
patch -p1 -d "$TMP_WORK" < "$FEATURE_PATCH_FILE"
patch -p1 -d "$TMP_WORK" < "$PATCH_FILE"
echo "[fnos-pack] ✅ patch 应用成功（feature + crop）"

# 生成 fnOS UI 配置（adapter manifest 唯一权威；覆盖 patcher 手写版，保证构建产物与契约一致）
echo "[fnos-pack] 生成 fnOS UI 配置（adapter manifest）…"
node "$REPO_ROOT/fnos-api/generate-ui-config.mjs" --out "$TMP_WORK/src/fnos-ui-config.ts"
if [ ! -s "$TMP_WORK/src/fnos-ui-config.ts" ]; then
  echo "[fnos-pack] ❌ fnos-ui-config.ts 生成失败" >&2
  exit 1
fi

# 验证：UI 裁剪已配置化（条件渲染由 adapter manifest 生成的 uiConfig 驱动）
if ! grep -q "uiConfig" "$TMP_WORK/src/App.tsx"; then
  echo "[fnos-pack] ❌ App.tsx 未引用 uiConfig（UI 裁剪配置化未生效）" >&2
  exit 1
fi
if grep -q "titleBar: true" "$TMP_WORK/src/fnos-ui-config.ts"; then
  echo "[fnos-pack] ❌ uiConfig.titleBar 为 true（TitleBar 未裁剪）" >&2
  exit 1
fi
if grep -q "antivirusWarningDialog: true" "$TMP_WORK/src/fnos-ui-config.ts"; then
  echo "[fnos-pack] ❌ uiConfig.antivirusWarningDialog 为 true（杀软警告未裁剪）" >&2
  exit 1
fi
echo "[fnos-pack] ✅ UI 精简验证通过（uiConfig 配置驱动：TitleBar / 杀软警告已裁剪）"

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
