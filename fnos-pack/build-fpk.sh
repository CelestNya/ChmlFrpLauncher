#!/usr/bin/env bash
# fnos-pack/build-fpk.sh
# 构建 ChmlFrp fnOS .fpk 安装包（默认），并可额外生成自更新 bundle（--bundle）。
#
# 流程：前端（apply-patches.sh：patch → tsc → vite → shim）→ daemon release 编译 → 组装 app.tgz → 组装 .fpk
# .fpk 本质是 tar.gz（manifest INI + app.tgz + cmd/ + config/ + ui/ + 图标），格式见 docs/fnos-developer-guide.md
#
# 用法（Linux / WSL / CI）：
#   bash fnos-pack/build-fpk.sh                # 自动检测架构（x86_64→x86，aarch64→arm）
#   bash fnos-pack/build-fpk.sh --arch x86_64  # 强制指定
#   bash fnos-pack/build-fpk.sh --bundle       # 额外生成自更新 bundle（B5）
#
# 交叉编译 aarch64 需要：
#   sudo apt install gcc-aarch64-linux-gnu
#   rustup target add aarch64-unknown-linux-gnu
#   （daemon 已用 rustls，无需交叉 openssl）
#
# 产物：dist-fpk/chmlfrp_<version>_<platform>.fpk
#       dist-fpk/chmlfrp-fnos-<version>-<platform>.tar.gz（--bundle，含 manifest.json + daemon + dist）
# 依赖：cargo / node / pnpm / patch / tar / md5sum / sha256sum / Python3(PIL 可选，图标缩放)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PKG_TEMPLATE="$REPO_ROOT/fnos-pack/fpk"
OUT_DIR="$REPO_ROOT/dist-fpk"

# ---------- 参数 ----------
ARCH="${ARCH:-$(uname -m)}"
SKIP_FRONTEND=0
MAKE_BUNDLE=0
while [ $# -gt 0 ]; do
    case "$1" in
        --arch) ARCH="$2"; shift 2 ;;
        --skip-frontend) SKIP_FRONTEND=1; shift ;;
        --bundle) MAKE_BUNDLE=1; shift ;;
        *) echo "未知参数: $1" >&2; exit 1 ;;
    esac
done

case "$ARCH" in
    x86_64|amd64)  RUST_TARGET="x86_64-unknown-linux-gnu"; PLATFORM="x86" ;;
    aarch64|arm64) RUST_TARGET="aarch64-unknown-linux-gnu"; PLATFORM="arm" ;;
    *) echo "不支持的架构: $ARCH" >&2; exit 1 ;;
esac

# ---------- 版本号（来自 manifest） ----------
VERSION=$(grep "^version" "$PKG_TEMPLATE/manifest" | awk -F'=' '{print $2}' | tr -d ' ')

echo "[fnos-pack] ===== 构建 ChmlFrp fnOS .fpk ====="
echo "[fnos-pack] 版本: $VERSION | 平台: $PLATFORM ($RUST_TARGET)"

mkdir -p "$OUT_DIR"

# ---------- 1. 前端产物（patch + build + shim） ----------
if [ "$SKIP_FRONTEND" = "1" ]; then
    [ -f "$REPO_ROOT/dist-fnos/index.html" ] || { echo "[fnos-pack] ❌ --skip-frontend 但 dist-fnos 不存在，请先构建前端" >&2; exit 1; }
    echo "[fnos-pack] ① 跳过前端构建，复用 dist-fnos/"
else
    echo "[fnos-pack] ① 构建前端（patch + tsc + vite + shim）…"
    OUTPUT_DIR="$REPO_ROOT/dist-fnos" bash "$REPO_ROOT/fnos-pack/apply-patches.sh"
fi

# ---------- 2. 编译 daemon ----------
echo "[fnos-pack] ② 编译 daemon ($RUST_TARGET, release)…"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/fnos-daemon/target}"
if [ "$RUST_TARGET" = "x86_64-unknown-linux-gnu" ] && [ "$(uname -m)" = "x86_64" ]; then
    # 原生编译
    cargo build --release --manifest-path "$REPO_ROOT/fnos-daemon/Cargo.toml"
    DAEMON_BIN="$CARGO_TARGET_DIR/release/chmlfrp-daemon"
else
    # 交叉编译（需 rustup target add + 系统交叉工具链）
    rustup target add "$RUST_TARGET" 2>/dev/null || true
    # rustls 已消除 openssl 依赖，仅需交叉 C 链接器（gcc-aarch64-linux-gnu）
    export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER:-aarch64-linux-gnu-gcc}"
    cargo build --release --target "$RUST_TARGET" --manifest-path "$REPO_ROOT/fnos-daemon/Cargo.toml"
    DAEMON_BIN="$CARGO_TARGET_DIR/$RUST_TARGET/release/chmlfrp-daemon"
fi
[ -f "$DAEMON_BIN" ] || { echo "[fnos-pack] ❌ daemon 二进制未生成: $DAEMON_BIN" >&2; exit 1; }

# ---------- 3. 组装 app.tgz（target/ 目录内容） ----------
echo "[fnos-pack] ③ 组装 app.tgz…"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

APP_DIR="$WORK_DIR/app"
mkdir -p "$APP_DIR/target/dist"
cp "$DAEMON_BIN" "$APP_DIR/target/chmlfrp-daemon"
cp -r "$REPO_ROOT/dist-fnos/." "$APP_DIR/target/dist/"

# 图标：项目 512x512 → 64 / 256
echo "[fnos-pack] ④ 生成图标…"
mkdir -p "$APP_DIR/ui/images"
if command -v python3 >/dev/null 2>&1 && python3 -c "import PIL" 2>/dev/null; then
    python3 - "$REPO_ROOT/src-tauri/icons/icon.png" "$APP_DIR" <<'PY'
import sys
from PIL import Image
src, outdir = sys.argv[1], sys.argv[2]
img = Image.open(src).convert("RGBA")
img.resize((64, 64), Image.LANCZOS).save(f"{outdir}/ICON.PNG", "PNG")
img.resize((256, 256), Image.LANCZOS).save(f"{outdir}/ICON_256.PNG", "PNG")
img.resize((256, 256), Image.LANCZOS).save(f"{outdir}/ui/images/256.png", "PNG")
img.resize((64, 64), Image.LANCZOS).save(f"{outdir}/ui/images/64.png", "PNG")
print("[fnos-pack] 图标生成完成")
PY
else
    echo "[fnos-pack] ⚠️ PIL 不可用，跳过图标缩放（.fpk 将缺 ICON*.PNG）" >&2
fi

# ---------- 4. 组装 fpk 目录 ----------
echo "[fnos-pack] ⑤ 组装 fpk 包结构…"
PKG_DIR="$WORK_DIR/pkg"
mkdir -p "$PKG_DIR/cmd" "$PKG_DIR/config" "$PKG_DIR/ui"

# app.tgz = target 目录压缩
( cd "$APP_DIR/target" && tar -czf "$WORK_DIR/app.tgz" * )
# 模板文件
cp "$WORK_DIR/app.tgz" "$PKG_DIR/app.tgz"
cp "$PKG_TEMPLATE/manifest" "$PKG_DIR/manifest"
cp "$PKG_TEMPLATE/cmd/main" "$PKG_DIR/cmd/main"
cp -a "$PKG_TEMPLATE/config/." "$PKG_DIR/config/"
cp -a "$PKG_TEMPLATE/ui/config" "$PKG_DIR/ui/config"
cp "$PKG_TEMPLATE/ChmlFrp.sc" "$PKG_DIR/ChmlFrp.sc"
[ -f "$APP_DIR/ICON.PNG" ] && cp "$APP_DIR/ICON.PNG" "$PKG_DIR/ICON.PNG"
[ -f "$APP_DIR/ICON_256.PNG" ] && cp "$APP_DIR/ICON_256.PNG" "$PKG_DIR/ICON_256.PNG"
[ -f "$APP_DIR/ui/images/256.png" ] && cp -r "$APP_DIR/ui/images" "$PKG_DIR/ui/images"

# manifest：platform / checksum / 版本
CHECKSUM=$(md5sum "$WORK_DIR/app.tgz" | cut -d' ' -f1)
sed -i "s/^platform.*/platform        = ${PLATFORM}/" "$PKG_DIR/manifest"
sed -i "s/^version.*/version         = ${VERSION}/" "$PKG_DIR/manifest"
sed -i "s/^checksum.*/checksum        = ${CHECKSUM}/" "$PKG_DIR/manifest"

# ---------- 5. 出包 ----------
echo "[fnos-pack] ⑥ 打包 .fpk…"
FPK_NAME="chmlfrp_${VERSION}_${PLATFORM}.fpk"
( cd "$PKG_DIR" && tar -czf "$OUT_DIR/$FPK_NAME" * )

echo "[fnos-pack] ✅ 完成: $OUT_DIR/$FPK_NAME ($(du -h "$OUT_DIR/$FPK_NAME" | cut -f1))"
echo "[fnos-pack]     app.tgz md5: $CHECKSUM"

# ---------- 6. 自更新 bundle（B5，可选 --bundle） ----------
if [ "$MAKE_BUNDLE" = "1" ]; then
    echo "[fnos-pack] ⑦ 生成自更新 bundle…"
    BUNDLE_DIR="$WORK_DIR/bundle"
    mkdir -p "$BUNDLE_DIR"
    cp "$APP_DIR/target/chmlfrp-daemon" "$BUNDLE_DIR/chmlfrp-daemon"
    cp -r "$APP_DIR/target/dist" "$BUNDLE_DIR/dist"

    # manifest.json：{version, platform, files: {相对路径: sha256}}
    python3 - "$BUNDLE_DIR" "$VERSION" "$PLATFORM" <<'PY'
import hashlib, json, os, sys
root, version, platform = sys.argv[1], sys.argv[2], sys.argv[3]
files = {}
for d, _, fnames in os.walk(root):
    for f in fnames:
        if f == "manifest.json":
            continue
        p = os.path.join(d, f)
        rel = os.path.relpath(p, root).replace(os.sep, "/")
        files[rel] = hashlib.sha256(open(p, "rb").read()).hexdigest()
with open(os.path.join(root, "manifest.json"), "w") as fh:
    json.dump({"version": version, "platform": platform, "files": files}, fh, indent=2)
PY

    BUNDLE_NAME="chmlfrp-fnos-${VERSION}-${PLATFORM}.tar.gz"
    ( cd "$BUNDLE_DIR" && tar -czf "$OUT_DIR/$BUNDLE_NAME" * )
    echo "[fnos-pack] ✅ 完成: $OUT_DIR/$BUNDLE_NAME ($(du -h "$OUT_DIR/$BUNDLE_NAME" | cut -f1))"
fi
