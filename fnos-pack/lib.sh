#!/usr/bin/env bash
# fnos-pack/lib.sh
# 构建脚本共享的纯函数（source 后无任何副作用），供 apply-patches.sh /
# build-fpk.sh / gen-patch.sh 复用，并支持独立单测（tests/lib.test.sh）。
# 批 E 引入：把可测逻辑从脚本主体抽离（TDD 的可测性要求）。

# E1：构建输出目录边界校验。
# 要求绝对路径，且不在危险名单（仓库根 / src/ / 根目录）——apply-patches.sh
# 会对该目录执行 rm -rf，误设为仓库根会删掉整个仓库。
# 用法：validate_output_dir <dir> <repo_root>；失败输出 stderr 并返回 1。
validate_output_dir() {
    local dir="$1"
    local repo_root="$2"
    case "$dir" in
        /*) : ;; # 绝对路径
        *)
            echo "❌ 输出目录必须是绝对路径: $dir" >&2
            return 1
            ;;
    esac
    local real_dir real_root
    real_dir=$(readlink -f "$dir" 2>/dev/null || echo "$dir")
    real_root=$(readlink -f "$repo_root" 2>/dev/null || echo "$repo_root")
    if [ "$real_dir" = "$real_root" ] || [ "$real_dir" = "$real_root/src" ] || [ "$real_dir" = "/" ]; then
        echo "❌ 拒绝删除危险输出目录: $dir" >&2
        return 1
    fi
    return 0
}

# E4：版本一致性校验——fpk manifest 与 daemon Cargo.toml 必须一致（自更新 bundle
# 用 manifest 版本命名，daemon 用自身版本匹配，漂移 = 自更新通道静默失效）。
# package.json 是上游前端版本（上游自管发版），不在此强制，仅由调用方提示。
# 用法：check_version_consistency <manifest> <cargo_toml>；失败返回 1。
check_version_consistency() {
    local manifest="$1"
    local cargo_toml="$2"
    local manifest_ver cargo_ver
    manifest_ver=$(grep -E '^version' "$manifest" 2>/dev/null | head -1 | sed -E 's/^version[[:space:]]*=[[:space:]]*//; s/[[:space:]]*$//')
    cargo_ver=$(grep -E '^version' "$cargo_toml" 2>/dev/null | head -1 | sed -E 's/^version[[:space:]]*=[[:space:]]*"//; s/"[[:space:]]*$//')
    if [ -z "$manifest_ver" ] || [ -z "$cargo_ver" ]; then
        echo "❌ 无法解析版本号（manifest: '$manifest_ver', Cargo.toml: '$cargo_ver'）" >&2
        return 1
    fi
    if [ "$manifest_ver" != "$cargo_ver" ]; then
        echo "❌ 版本不一致：manifest=$manifest_ver，Cargo.toml=$cargo_ver（发版前须同步）" >&2
        return 1
    fi
    return 0
}
