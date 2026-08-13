#!/usr/bin/env bash
# fnos-pack/build-fpk.sh
# 构建 ChmlFrp fnOS .fpk 安装包（官方 fnpack 工具），并可额外生成自更新 bundle（--bundle）。
#
# 流程：前端（apply-patches.sh：patch → tsc → vite → shim）→ daemon musl 静态编译 →
#       组装 fnpack 项目目录（app/ + cmd/ + config/ + wizard/ + manifest + 图标）→ fnpack build 出 .fpk
#
# ⚠️ daemon 必须 musl 静态编译：fnOS 基于 Debian 12（glibc 2.36），gnu 动态链接二进制
#    若要求更高 GLIBC 版本会在真机加载即崩（白屏/闪退）。已实测验证。
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
#   sudo apt install musl-tools gcc-aarch64-linux-gnu
#   rustup target add aarch64-unknown-linux-musl
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
        echo "[fnpack] 下载 fnpack 1.2.3 ($os_arch)…" >&2
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

# 纯函数库（版本一致性校验等，可独立单测：bash fnos-pack/tests/lib.test.sh）
source "$REPO_ROOT/fnos-pack/lib.sh"

# E4：版本一致性——manifest == Cargo.toml（自更新 bundle 命名与 daemon 匹配，
# 漂移 = 自更新通道静默失效）；package.json 是上游前端版本，仅提示不强制
check_version_consistency "$PKG_TEMPLATE/manifest" "$REPO_ROOT/fnos-daemon/Cargo.toml" || exit 1
PKG_VER=$(grep '"version"' "$REPO_ROOT/package.json" 2>/dev/null | head -1 | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')
if [ -n "$PKG_VER" ] && [ "$PKG_VER" != "$VERSION" ]; then
    echo "[fnos-pack] ⚠️ 前端 package.json 版本 ($PKG_VER) 与 fnOS 版 ($VERSION) 不一致（上游前端版本独立管理，仅提示）"
fi

mkdir -p "$OUT_DIR"

# ---------- 1. 前端产物（patch + build + shim） ----------
if [ "$SKIP_FRONTEND" = "1" ]; then
    # E8：校验构建完成戳记——仅 index.html 存在不足以防复用陈旧/残缺产物
    [ -f "$REPO_ROOT/dist-fnos/.build-ok" ] || { echo "[fnos-pack] ❌ --skip-frontend 但 dist-fnos 无构建完成戳记，请先跑 apply-patches.sh" >&2; exit 1; }
    [ -f "$REPO_ROOT/dist-fnos/index.html" ] || { echo "[fnos-pack] ❌ --skip-frontend 但 dist-fnos 不存在，请先构建前端" >&2; exit 1; }
    echo "[fnos-pack] ① 跳过前端构建，复用 dist-fnos/"
else
    echo "[fnos-pack] ① 构建前端（patch + tsc + vite + shim）…"
    OUTPUT_DIR="$REPO_ROOT/dist-fnos" bash "$REPO_ROOT/fnos-pack/apply-patches.sh"
fi

# ---------- 2. 编译 daemon（musl 静态，兼容 fnOS glibc 2.36） ----------
echo "[fnos-pack] ② 编译 daemon ($RUST_TARGET, musl 静态)…"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$REPO_ROOT/fnos-daemon/target}"
case "$RUST_TARGET" in
    x86_64-unknown-linux-gnu)
        MUSL_TARGET="x86_64-unknown-linux-musl"
        # cargo 对 x86_64 musl 会自动找 x86_64-linux-musl-gcc（musl-tools 提供）
        ;;
    aarch64-unknown-linux-gnu)
        MUSL_TARGET="aarch64-unknown-linux-musl"
        # 交叉 musl 需要 aarch64-linux-musl-gcc（如 musl.cc 工具链）
        export CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER="${CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER:-aarch64-linux-musl-gcc}"
        ;;
esac
rustup target add "$MUSL_TARGET" 2>/dev/null || true
cargo build --release --target "$MUSL_TARGET" --manifest-path "$REPO_ROOT/fnos-daemon/Cargo.toml"
DAEMON_BIN="$CARGO_TARGET_DIR/$MUSL_TARGET/release/chmlfrp-daemon"
# 校验静态链接（防止误用 gnu 产物；新版 rustc 默认 static-pie，两者均无 glibc 依赖）
if ! file "$DAEMON_BIN" | grep -Eq "statically linked|static-pie linked"; then
    echo "[fnos-pack] ❌ daemon 非静态链接，拒绝打包" >&2
    exit 1
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

# fnOS 市场升级（install-fpk）会保留「777 可写目录」不覆盖（视为应用可写数据），
# 导致 dist/ 在升级时不被替换（前端停留旧版，daemon 与前端版本错配）。
# vite 产物目录默认 755 但 CI/拷贝可能带 group/other 写位——显式收紧：
# 目录/文件去掉 group+other 写位（775→755、664/666→644），保留原有 owner 写位与执行位。
find "$PKG_DIR/app/dist" -type d -exec chmod go-w {} \;
find "$PKG_DIR/app/dist" -type f -exec chmod go-w {} \;

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
# E3：Pillow ≥10 移除了 Image.LANCZOS——用 getattr 兼容新旧版本
resample = getattr(Image, "Resampling", Image).LANCZOS
img.resize((64, 64), resample).save(f"{pkg}/ICON.PNG", "PNG")
img.resize((256, 256), resample).save(f"{pkg}/ICON_256.PNG", "PNG")
img.resize((256, 256), resample).save(f"{pkg}/app/ui/images/icon_256.png", "PNG")
img.resize((64, 64), resample).save(f"{pkg}/app/ui/images/icon_64.png", "PNG")
print("[fnos-pack] 图标生成完成")
PY
else
    # E7：图标缺失不再「仅警告继续」——.fpk 必须含 ICON*.PNG，静默降级产出
    # 不合规安装包（与 daemon 静态链接校验的严格度对齐）
    echo "[fnos-pack] ❌ PIL 不可用，无法生成图标（.fpk 必须含 ICON*.PNG）" >&2
    exit 1
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
