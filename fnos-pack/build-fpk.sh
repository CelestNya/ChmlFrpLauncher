#!/usr/bin/env bash
# fnos-pack/build-fpk.sh
# 构建 ChmlFrp fnOS .fpk 安装包（官方 fnpack 工具，替代手写 tar），并可额外生成自更新 bundle（--bundle）。
#
# 流程：前端（apply-patches.sh：patch → tsc → vite → shim）→ daemon release 编译 →
#       组装 fnpack 项目目录（app/ + cmd/ + config/ + wizard/ + manifest + 图标）→ fnpack build 出 .fpk
#
# 用法（Linux / WSL / CI）：
#   bash fnos-pack/build-fpk.sh                # 自动检测架构（x86_64→x86，aarch64→arm）
#   bash fnos-pack/build-fpk.sh --arch x86_64  # 强制指定
#   bash fnos-pack/build-fpk.sh --bundle       # 额外生成自更新 bundle（B5）
#
# fnpack 二进制：
#   - 已安装：自动使用 PATH 中的 fnpack
#   - 未安装：自动下载官方 static2.fnnas.com 二进制到 .fnpack-cache/（首次联网）
#   - CI：建议用 mengzhuo/setup-fnpack action 提供 fnpack
#
# 交叉编译 aarch64 需要：
#   sudo apt install gcc-aarch64-linux-gnu
#   rustup target add aarch64-unknown-linux-gnu
#   （daemon 已用 rustls，无需交叉 openssl）
#
# 产物：dist-fpk/chmlfrp_<version>_<platform>.fpk
#       dist-fpk/chmlfrp-fnos-<version>-<platform>.tar.gz（--bundle，含 manifest.json + daemon + dist）
# 依赖：cargo / node / pnpm / patch / Python3(PIL 可选，图标缩放)

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

# ---------- fnpack 定位（PATH 优先，其次自动下载） ----------
ensure_fnpack() {
    if command -v fnpack >/dev/null 2>&1; then
        echo "fnpack"
        return
    fi
    local cache="$REPO_ROOT/.fnpack-cache"
    mkdir -p "$cache"
    local bin="$cache/fnpack"
    if [ ! -x "$bin" ]; then
        local os_arch
        case "$(uname -s)-$(uname -m)" in
            Linux-x86_64) os_arch="linux-amd64" ;;
            Linux-aarch64) os_arch="linux-arm64" ;;
            Darwin-x86_64) os_arch="darwin-amd64" ;;
            Darwin-arm64) os_arch="darwin-arm64" ;;
            *) echo "不支持的 fnpack 平台: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
        esac
        echo "[fnpack] 下载 fnpack 1.2.3 ($os_arch)…"
        for _ in 1 2 3; do
            curl -fsSL -o "$bin" "https://static2.fnnas.com/fnpack/fnpack-1.2.3-${os_arch}" && [ -s "$bin" ] && break
            echo "[fnpack] 下载失败，重试…" >&2
            rm -f "$bin"
            sleep 2
        done
        [ -s "$bin" ] || { echo "[fnpack] ❌ 下载 fnpack 失败" >&2; exit 1; }
        chmod +x "$bin"
    fi
    echo "$bin"
}

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

# ---------- 3. 组装 fnpack 项目目录 ----------
echo "[fnos-pack] ③ 组装 fnpack 项目目录…"
WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
PKG_DIR="$WORK_DIR/pkg"

mkdir -p "$PKG_DIR/app/ui/images" "$PKG_DIR/cmd" "$PKG_DIR/config" "$PKG_DIR/wizard"

# app/ 内容：daemon + dist（fnpack 会把 app/ 平铺进 TRIM_APPDEST）
cp "$DAEMON_BIN" "$PKG_DIR/app/chmlfrp-daemon"
cp -r "$REPO_ROOT/dist-fnos" "$PKG_DIR/app/dist"

# 模板文件
cp "$PKG_TEMPLATE/manifest" "$PKG_DIR/manifest"
cp "$PKG_TEMPLATE/app/ui/config" "$PKG_DIR/app/ui/config"
cp "$PKG_TEMPLATE/cmd/"* "$PKG_DIR/cmd/" 2>/dev/null || true
cp "$PKG_TEMPLATE/config/privilege" "$PKG_DIR/config/privilege"
cp "$PKG_TEMPLATE/config/resource" "$PKG_DIR/config/resource"
touch "$PKG_DIR/wizard/.gitkeep"

# 平台改写 manifest
sed -i "s/^platform.*/platform              = ${PLATFORM}/" "$PKG_DIR/manifest"

# 图标：项目 512x512 → ICON.PNG(64) / ICON_256.PNG(256) / app/ui/images/icon_{64,256}.png
echo "[fnos-pack] ④ 生成图标…"
if command -v python3 >/dev/null 2>&1 && python3 -c "import PIL" 2>/dev/null; then
    python3 - "$REPO_ROOT/src-tauri/icons/icon.png" "$PKG_DIR" <<'PY'
import sys
from PIL import Image
src, pkg = sys.argv[1], sys.argv[2]
img = Image.open(src).convert("RGBA")
img.resize((64, 64), Image.LANCZOS).save(f"{pkg}/ICON.PNG", "PNG")
img.resize((256, 256), Image.LANCZOS).save(f"{pkg}/ICON_256.PNG", "PNG")
img.resize((256, 256), Image.LANCZOS).save(f"{pkg}/app/ui/images/icon_256.png", "PNG")
img.resize((64, 64), Image.LANCZOS).save(f"{pkg}/app/ui/images/icon_64.png", "PNG")
print("[fnos-pack] 图标生成完成")
PY
else
    echo "[fnos-pack] ⚠️ PIL 不可用，跳过图标缩放（.fpk 将缺 ICON*.PNG）" >&2
fi

# ---------- 4. fnpack build ----------
echo "[fnos-pack] ⑤ fnpack build…"
FNPPACK="$(ensure_fnpack)"
( cd "$PKG_DIR" && "$FNPPACK" build )
FPK_NAME="chmlfrp_${VERSION}_${PLATFORM}.fpk"
cp "$PKG_DIR/chmlfrp.fpk" "$OUT_DIR/$FPK_NAME"
echo "[fnos-pack] ✅ 完成: $OUT_DIR/$FPK_NAME ($(du -h "$OUT_DIR/$FPK_NAME" | cut -f1))"

# ---------- 5. 自更新 bundle（B5，可选 --bundle） ----------
if [ "$MAKE_BUNDLE" = "1" ]; then
    echo "[fnos-pack] ⑥ 生成自更新 bundle…"
    BUNDLE_DIR="$WORK_DIR/bundle"
    mkdir -p "$BUNDLE_DIR"
    cp "$PKG_DIR/app/chmlfrp-daemon" "$BUNDLE_DIR/chmlfrp-daemon"
    cp -r "$PKG_DIR/app/dist" "$BUNDLE_DIR/dist"

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
