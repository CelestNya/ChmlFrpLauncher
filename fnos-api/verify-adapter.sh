#!/usr/bin/env bash
# fnos-api/verify-adapter.sh
# 校验 adapter 是否覆盖当前 patch 需求（用户要求：确保能力在当前 patcher 上够用）。
#
# 逻辑：
#   1. 命令面：扫描 src/ 实际 invoke 的命令名，与 adapter 的 implemented + noop 合并集比对
#   2. UI 配置面：patch 代码引用的 uiConfig 键 ⊆ manifest uiConfig.values 键；
#      patcher 手写版 src/fnos-ui-config.ts 与 manifest 生成版必须一致（manifest 唯一权威）
#
# 用法：bash fnos-api/verify-adapter.sh <adapter-manifest>
# 依赖：jq（或 python）

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ADAPTER_MANIFEST="${1:-$REPO_ROOT/fnos-api/adapters/v0.7.5/manifest.json}"

# 裁剪补丁（ui-crop.patch）由 adapter 分支持有（2026-08-16 决策：裁剪归 adapter，
# patch 零裁剪代码）。路径从 manifest 的 uiCropPatch 声明读；组合构建态下在工作区可读。
# Windows 下 python 不认 Git Bash 的 /d/ 路径 → cygpath 转成 D:\（与 PY_MANIFEST 同款）。
PY_CROP_MANIFEST="$ADAPTER_MANIFEST"
if command -v cygpath >/dev/null 2>&1; then
    PY_CROP_MANIFEST=$(cygpath -w "$ADAPTER_MANIFEST")
fi
CROP_PATCH=$(python -c "import json,sys; m=json.load(open(r'$PY_CROP_MANIFEST')); print(m.get('uiCropPatch',''))" | tr -d '\r')
if [ -n "$CROP_PATCH" ]; then
    CROP_PATCH="$REPO_ROOT/fnos-api/$CROP_PATCH"
fi

echo "[verify-adapter] 校验 adapter: $ADAPTER_MANIFEST"

# 1. 扫描调用命令（排除 plugin: 前缀，仅业务命令）
#    形态：invoke<string>("cmd", args) 或 invoke("cmd", args)
#    扫描对象 = src/（patcher 开发态：含 fnOS 改动）+ fnos-pack/patches/（组合构建态：src 干净，
#    fnOS 改动在 patch 文件里，只统计新增行）
USED_COMMANDS=$(find "$REPO_ROOT/src" \( -name "*.ts" -o -name "*.tsx" \) \
    -exec grep -hoP 'invoke(?:<[^>]*>)?\(\s*["'"'"']\K[a-z_]+' {} + 2>/dev/null || true)
if [ -d "$REPO_ROOT/fnos-pack/patches" ]; then
    USED_COMMANDS="$USED_COMMANDS
$(find "$REPO_ROOT/fnos-pack/patches" -name "*.patch" \
    -exec grep -hoP '^\+.*invoke(?:<[^>]*>)?\(\s*["'"'"']\K[a-z_]+' {} + 2>/dev/null || true)"
fi
if [ -n "$CROP_PATCH" ] && [ -f "$CROP_PATCH" ]; then
    USED_COMMANDS="$USED_COMMANDS
$(grep -hoP '^\+.*invoke(?:<[^>]*>)?\(\s*["'"'"']\K[a-z_]+' "$CROP_PATCH" 2>/dev/null || true)"
fi
USED_COMMANDS=$(echo "$USED_COMMANDS" | sort -u)
echo "=== 实际 invoke 的命令（$(echo "$USED_COMMANDS" | wc -l) 个）==="
echo "$USED_COMMANDS"

# 2. adapter 能力面（implemented + noop）
# Windows 下 python 不认 Git Bash 的 /d/ 路径 → cygpath 转成 D:\；
# python stdout 在 Windows 输出 CRLF → tr -d '\r' 统一清洗（否则 grep -qx pattern 带 \r 匹配失败）
PY_MANIFEST="$ADAPTER_MANIFEST"
if command -v cygpath >/dev/null 2>&1; then
    PY_MANIFEST=$(cygpath -w "$ADAPTER_MANIFEST")
fi
IMPLEMENTED=$(python -c "import json,sys; m=json.load(open(r'$PY_MANIFEST')); print('\n'.join(m['capabilities']['commands']['implemented']))" | tr -d '\r')
NOOP=$(python -c "import json,sys; m=json.load(open(r'$PY_MANIFEST')); print('\n'.join(m['capabilities']['commands']['noop']))" | tr -d '\r')
ADAPTER_CMDS=$(echo -e "$IMPLEMENTED\n$NOOP" | sort -u)

# 3. 比对：前端调用的命令是否都在 adapter 面内
MISSING=0
for cmd in $USED_COMMANDS; do
    if ! echo "$ADAPTER_CMDS" | grep -qx "$cmd"; then
        echo "❌ 命令 '$cmd' 不在 adapter 能力面（implemented + noop）" >&2
        MISSING=1
    fi
done

if [ $MISSING -eq 0 ]; then
    echo ""
    echo "✅ 全部调用命令在 adapter 能力面内（能力覆盖当前 patcher）"
else
    echo ""
    echo "❌ 存在能力缺口：以上命令需加入 adapter 的 implemented 或 noop" >&2
    exit 1
fi

# 4. UI 配置面：patch 代码引用的 uiConfig 键 ⊆ manifest uiConfig.values 键
MANIFEST_KEYS=$(python -c "import json,sys; m=json.load(open(r'$PY_MANIFEST')); print('\n'.join(m.get('uiConfig',{}).get('values',{}).keys()))" | tr -d '\r')
if [ -z "$MANIFEST_KEYS" ]; then
    echo "❌ adapter manifest 缺少 uiConfig.values" >&2
    exit 1
fi
# 扫描 uiConfig.xxx 引用（src 开发态 + patches 组合态，排除生成文件本身）
USED_KEYS=$(find "$REPO_ROOT/src" \( -name "*.ts" -o -name "*.tsx" \) \
    ! -name "fnos-ui-config.ts" \
    -exec grep -hoP 'uiConfig\.\K[a-zA-Z0-9_]+' {} + 2>/dev/null || true)
if [ -d "$REPO_ROOT/fnos-pack/patches" ]; then
    USED_KEYS="$USED_KEYS
$(find "$REPO_ROOT/fnos-pack/patches" -name "*.patch" \
    -exec grep -hoP '^\+.*uiConfig\.\K[a-zA-Z0-9_]+' {} + 2>/dev/null || true)"
fi
if [ -n "$CROP_PATCH" ] && [ -f "$CROP_PATCH" ]; then
    USED_KEYS="$USED_KEYS
$(grep -hoP '^\+.*uiConfig\.\K[a-zA-Z0-9_]+' "$CROP_PATCH" 2>/dev/null || true)"
fi
USED_KEYS=$(echo "$USED_KEYS" | sort -u)
MISSING_KEY=0
for key in $USED_KEYS; do
    if ! echo "$MANIFEST_KEYS" | grep -qx "$key"; then
        echo "❌ uiConfig 键 '$key' 未在 adapter manifest 定义（patch 引用了未包圆的配置）" >&2
        MISSING_KEY=1
    fi
done
UNUSED_KEYS=0
for key in $MANIFEST_KEYS; do
    if ! echo "$USED_KEYS" | grep -qx "$key"; then
        echo "⚠️  uiConfig 键 '$key' 未被 patch 引用（adapter 包圆但当前无消费方，正常）" >&2
    fi
done
if [ $MISSING_KEY -eq 0 ]; then
    echo "✅ 全部 uiConfig 引用键在 adapter manifest 内（统一格式包圆）"
else
    echo ""
    echo "❌ 存在 uiConfig 缺口：以上键需加入 adapter manifest 的 uiConfig.values" >&2
    exit 1
fi

# 5. 一致性：patcher 手写版 src/fnos-ui-config.ts 必须与 manifest 生成版一致（manifest 唯一权威）。
#    组合构建态下 src/ 无该文件（构建时由 generate-ui-config 生成）→ 跳过
if [ -f "$REPO_ROOT/src/fnos-ui-config.ts" ]; then
    TMP_GEN="$(mktemp)"
    trap 'rm -f "$TMP_GEN"' EXIT
    node "$REPO_ROOT/fnos-api/generate-ui-config.mjs" --out "$TMP_GEN" >/dev/null
    if ! diff -q "$TMP_GEN" "$REPO_ROOT/src/fnos-ui-config.ts" >/dev/null; then
        echo "❌ src/fnos-ui-config.ts 与 manifest 生成版不一致（manifest 是唯一权威，请用 generate-ui-config.mjs 重新生成）" >&2
        diff "$TMP_GEN" "$REPO_ROOT/src/fnos-ui-config.ts" | head -20 >&2
        exit 1
    fi
    echo "✅ src/fnos-ui-config.ts 与 manifest 生成版一致"
else
    echo "⚠️ src/fnos-ui-config.ts 不存在（组合构建态，构建时由 manifest 生成）"
fi

# 6. patch 依赖校验：每个 patch 的 requires.api 必须 ≤ adapter 的 apiVersion（semver 粗筛）
PATCH_MANIFEST="$REPO_ROOT/fnos-pack/patches/manifest.json"
if [ -f "$PATCH_MANIFEST" ]; then
    echo ""
    echo "=== patch 依赖校验（requires.api ≤ adapter apiVersion）==="
    PY_PATCH_MANIFEST="$PATCH_MANIFEST"
    if command -v cygpath >/dev/null 2>&1; then
        PY_PATCH_MANIFEST=$(cygpath -w "$PATCH_MANIFEST")
    fi
    python - "$PY_MANIFEST" "$PY_PATCH_MANIFEST" <<'PYEOF'
import json, sys, re

def ver(v):
    return tuple(int(x) for x in v.strip().split("."))

adapter = json.load(open(sys.argv[1]))
api_version = ver(adapter["apiVersion"])
patch_manifest = json.load(open(sys.argv[2]))
patches = patch_manifest["patches"]

# patchSetVersion 格式校验：架构.功能.修复（三段数字，2026-08-16 用户决策）
psv = patch_manifest.get("patchSetVersion")
if not psv or not re.fullmatch(r"\d+\.\d+\.\d+", psv):
    print(f"❌ patchSetVersion '{psv}' 非法（须为 架构.功能.修复 三段数字，如 1.5.2）", file=sys.stderr)
    sys.exit(1)
print(f"✅ patchSetVersion {psv}（架构.功能.修复）")

ok = True
for p in patches:
    req = ver(p["requires"]["api"].replace(">=", ""))
    if req <= api_version:
        print(f"✅ {p['file']}（featureVersion {p['featureVersion']}，requires.api {p['requires']['api']}）≤ apiVersion {adapter['apiVersion']}")
    else:
        print(f"❌ {p['file']} requires.api {p['requires']['api']} 超出 adapter apiVersion {adapter['apiVersion']}（需要先 bump API）", file=sys.stderr)
        ok = False
sys.exit(0 if ok else 1)
PYEOF
else
    echo "⚠️ 未找到 $PATCH_MANIFEST，跳过 patch 依赖校验"
fi
