// fnos-shim/build-shim.ts
// 构建期脚本：把 tauri-shim.ts 编译为 IIFE JS，注入 dist/index.html。
//
// 用法：pnpm build 后执行 `node fnos-shim/build-shim.ts [--dist <目录>]`
// 仅在 fnOS 构建链中调用；桌面版构建不经过本脚本（src/ 与 src-tauri/ 零改动）。
//
// 实现说明：
// - shim 经 esbuild 编译（pnpm store 中 esbuild 不 hoist 到顶层，且 bin link 未建立，
//   故直接定位 .pnpm 中的 esbuild 包 CLI，避免 `pnpm exec` 触发依赖检查安装）
// - 注入为 body 末尾的普通 <script>：普通脚本同步执行，早于 type=module（defer）的
//   app bundle，保证前端 import @tauri-apps/api 时 __TAURI_INTERNALS__ 已就绪
// - 幂等：已注入过则跳过（避免 CI 重复注入）

import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const ROOT = process.cwd();
const args = process.argv.slice(2);
const distIndex = args.indexOf("--dist");
const distDir =
  distIndex >= 0 && args[distIndex + 1]
    ? path.resolve(ROOT, args[distIndex + 1])
    : path.join(ROOT, "dist");
const shimSrc = path.join(ROOT, "fnos-shim", "tauri-shim.ts");

const MARKER = "<!-- fnos-shim -->";

/** 平台名：win32-x64 / linux-x64 / darwin-arm64 等（对应 esbuild 平台包目录名）。 */
function platformName() {
  return `${process.platform}-${process.arch}`;
}

/**
 * 在 pnpm store 中定位 esbuild **平台原生二进制**（@esbuild/<platform>/bin/esbuild）。
 * 直接 execFileSync 运行，不经 node——node 20 在 Linux 上运行 bin JS 包装会
 * 因模块语法解析失败（SyntaxError），CI 已实测踩坑。
 * pnpm 平台包目录带版本号（@esbuild+linux-x64@0.27.2），需扫描而非硬拼。
 * 找不到时退回 esbuild JS 包装（node 运行）。
 */
function findEsbuildBin() {
  const pnpmDir = path.join(ROOT, "node_modules", ".pnpm");
  const plat = platformName();
  try {
    const nativeDir = fs
      .readdirSync(pnpmDir)
      .filter((d) => d.startsWith(`@esbuild+${plat}@`))
      .sort()
      .reverse()[0];
    if (nativeDir) {
      const nativeBin = path.join(
        pnpmDir,
        nativeDir,
        "node_modules",
        "@esbuild",
        plat,
        "bin",
        process.platform === "win32" ? "esbuild.exe" : "esbuild",
      );
      if (fs.existsSync(nativeBin)) {
        return { bin: nativeBin, useNode: false };
      }
    }
  } catch {
    // 继续走 JS 包装兜底
  }
  // 退回 JS 包装（本机 store 结构异常时）
  try {
    const candidates = fs
      .readdirSync(pnpmDir)
      .filter((d) => d.startsWith("esbuild@") && !d.includes("+"))
      .sort()
      .reverse();
    for (const dir of candidates) {
      const cli = path.join(
        pnpmDir,
        dir,
        "node_modules",
        "esbuild",
        "bin",
        "esbuild",
      );
      if (fs.existsSync(cli)) return { bin: cli, useNode: true };
    }
  } catch {
    // 兜底环境变量里的 esbuild
  }
  return { bin: "esbuild", useNode: false };
}

const ESBUILD = findEsbuildBin();

function compileShim() {
  // 无 import/export 的 IIFE 文件，--format=esm 原样输出（仅做 TS→JS + minify）
  const args = [
    ESBUILD.bin,
    shimSrc,
    "--format=esm",
    "--minify",
    "--target=es2017",
  ];
  return execFileSync(ESBUILD.useNode ? process.execPath : ESBUILD.bin, args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  }).trim();
}

function injectShim(html, shimJs) {
  if (html.includes(MARKER)) return html;
  const tag = `${MARKER}\n<script>${shimJs}\n</script>`;
  const bodyEnd = "</body>";
  if (html.includes(bodyEnd)) {
    return html.replace(bodyEnd, `${tag}\n${bodyEnd}`);
  }
  return `${html}\n${tag}`;
}

if (!fs.existsSync(distDir)) {
  console.error(`dist 目录不存在: ${distDir}`);
  process.exit(1);
}

const shimJs = compileShim();
if (!shimJs) {
  console.error("shim 编译失败");
  process.exit(1);
}

const htmlFiles = fs
  .readdirSync(distDir)
  .filter((f) => f.endsWith(".html"));

for (const htmlFile of htmlFiles) {
  const htmlPath = path.join(distDir, htmlFile);
  const before = fs.readFileSync(htmlPath, "utf-8");
  const after = injectShim(before, shimJs);
  if (after !== before) {
    fs.writeFileSync(htmlPath, after);
    console.log(`✅ 注入 shim 到 ${htmlFile}`);
  } else {
    console.log(`⏭️  ${htmlFile} 已注入过，跳过`);
  }
}

console.log("fnOS shim 注入完成");
