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

/** 在 pnpm store 中定位 esbuild 包的 CLI 脚本（无 bin link 时可用）。 */
function findEsbuildCli(): string {
  const pnpmDir = path.join(ROOT, "node_modules", ".pnpm");
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
      if (fs.existsSync(cli)) return cli;
    }
  } catch {
    // 找不到则退回环境变量里的 esbuild
  }
  return "esbuild";
}

const ESBUILD_CLI = findEsbuildCli();

function compileShim(): string {
  // 无 import/export 的 IIFE 文件，--format=esm 原样输出（仅做 TS→JS + minify）
  return execFileSync(
    process.execPath,
    [
      ESBUILD_CLI,
      shimSrc,
      "--format=esm",
      "--minify",
      "--target=es2017",
    ],
    { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] },
  ).trim();
}

function injectShim(html: string, shimJs: string): string {
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
