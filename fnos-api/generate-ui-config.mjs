#!/usr/bin/env node
// fnos-api/generate-ui-config.mjs
// 从 adapter manifest 的 uiConfig.values 生成前端配置模块（src/fnos-ui-config.ts）。
// 构建链（apply-patches.sh）在应用 patch 前调用本脚本，manifest 是唯一权威；
// patcher 分支上的手写版本仅用于本地开发编译，值必须与 manifest 一致（verify-adapter.sh 校验）。
//
// 用法: node fnos-api/generate-ui-config.mjs [--adapter v0.7.5] [--out <path>]

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const argOf = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const adapter = argOf("--adapter") ?? "v0.7.5";
const out = argOf("--out") ?? resolve(repoRoot, "src/fnos-ui-config.ts");

const manifestPath = resolve(repoRoot, `fnos-api/adapters/${adapter}/manifest.json`);
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
const values = manifest.uiConfig?.values;
if (!values || typeof values !== "object") {
  console.error(`[generate-ui-config] ❌ ${adapter} manifest 缺少 uiConfig.values`);
  process.exit(1);
}

const lines = Object.entries(values)
  .map(([key, value]) => `  ${key}: ${JSON.stringify(value)},`)
  .join("\n");
const content = `// 由 fnos-api/generate-ui-config.mjs 从 fnos-api/adapters/${adapter}/manifest.json 生成，勿手改
export const uiConfig = {
${lines}
} as const;
`;

mkdirSync(dirname(out), { recursive: true });
writeFileSync(out, content);
console.log(`[generate-ui-config] ✅ 已生成 ${out}（${Object.keys(values).length} 个配置项，来自 ${adapter}）`);
