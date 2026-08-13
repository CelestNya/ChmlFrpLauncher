#!/usr/bin/env bash
# fnos-pack/tests/lib.test.sh
# lib.sh 纯函数单测（批 E）。运行：bash fnos-pack/tests/lib.test.sh
set -u
cd "$(dirname "$0")/.."
FAILURES=0

assert_fail() {
    if "$@" >/dev/null 2>&1; then
        echo "✗ 应失败但通过: $*"
        FAILURES=$((FAILURES + 1))
    else
        echo "✓ 正确拒绝: $*"
    fi
}

assert_ok() {
    if "$@" >/dev/null 2>&1; then
        echo "✓ 通过: $*"
    else
        echo "✗ 应通过但失败: $*"
        FAILURES=$((FAILURES + 1))
    fi
}

source ./lib.sh
REPO="$(pwd)"

# ---- E1 validate_output_dir ----
assert_ok validate_output_dir "/tmp/dist-fnos" "$REPO"
assert_ok validate_output_dir "$REPO/dist-fnos" "$REPO"
assert_fail validate_output_dir "$REPO" "$REPO"
assert_fail validate_output_dir "$REPO/src" "$REPO"
assert_fail validate_output_dir "/" "$REPO"
assert_fail validate_output_dir "relative/path" "$REPO"

# ---- E4 check_version_consistency ----
TMPD="$(mktemp -d)"
trap 'rm -rf "$TMPD"' EXIT
printf 'version = 0.7.5\n' >"$TMPD/manifest"
printf 'version = "0.7.5"\n' >"$TMPD/Cargo.toml"
assert_ok check_version_consistency "$TMPD/manifest" "$TMPD/Cargo.toml"

printf 'version = 0.8.0\n' >"$TMPD/manifest"
assert_fail check_version_consistency "$TMPD/manifest" "$TMPD/Cargo.toml"

printf 'version = "0.7.5"\n' >"$TMPD/Cargo.toml"
printf 'version = 0.8.0\n' >"$TMPD/manifest"
printf 'version = x\n' >"$TMPD/Cargo.toml"
assert_fail check_version_consistency "$TMPD/manifest" "$TMPD/Cargo.toml"

if [ "$FAILURES" -gt 0 ]; then
    echo "共 $FAILURES 项失败" >&2
    exit 1
fi
echo "✅ lib.sh 单测全部通过"
