#!/usr/bin/env bash
# fnos-api/verify-adapter.sh
# 校验 adapter 能力是否覆盖当前 patch 需求（用户要求：确保能力在当前 patcher 上够用）。
#
# 逻辑：
#   1. 扫描 src/ 实际 invoke 的命令名（业务代码直接调用）
#   2. 与 adapter manifest 的 implemented + noop 合并集比对
#   3. 任何调用点不在 adapter 能力面 → 失败（能力不足）
#
# 用法：bash fnos-api/verify-adapter.sh <adapter-manifest>
# 依赖：jq（或 python）

set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ADAPTER_MANIFEST="${1:-$REPO_ROOT/fnos-api/adapters/v0.7.5/manifest.json}"

echo "[verify-adapter] 校验 adapter: $ADAPTER_MANIFEST"

# 1. 扫描 src/ 实际调用的命令（排除 plugin: 前缀，仅业务命令）
#    形态：invoke<string>("cmd", args) 或 invoke("cmd", args)
USED_COMMANDS=$(find "$REPO_ROOT/src" \( -name "*.ts" -o -name "*.tsx" \) \
    -exec grep -hoP 'invoke(?:<[^>]*>)?\(\s*["'"'"']\K[a-z_]+' {} + 2>/dev/null | sort -u)
echo "=== 前端实际 invoke 的命令（$(echo "$USED_COMMANDS" | wc -l) 个）==="
echo "$USED_COMMANDS"

# 2. adapter 能力面（implemented + noop）
# Windows 下 python 不认 Git Bash 的 /d/ 路径 → cygpath 转成 D:\
PY_MANIFEST="$ADAPTER_MANIFEST"
if command -v cygpath >/dev/null 2>&1; then
    PY_MANIFEST=$(cygpath -w "$ADAPTER_MANIFEST")
fi
IMPLEMENTED=$(python -c "import json,sys; m=json.load(open(r'$PY_MANIFEST')); print('\n'.join(m['capabilities']['commands']['implemented']))")
NOOP=$(python -c "import json,sys; m=json.load(open(r'$PY_MANIFEST')); print('\n'.join(m['capabilities']['commands']['noop']))")
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
