// vitest.config.ts
// fnOS 移植 TDD 测试配置（patcher 分支独有，不进 gen-patch 清单 → main 保持 0 侵入）。
//
// 运行：pnpm exec vitest run
// 说明：
// - 显式 import vitest（上游测试风格，无 globals）
// - environment: jsdom（shim 与 React hook 测试需要 DOM）
// - include 覆盖 src/ 与 fnos-shim/ 下的 *.test.ts
// - @ alias 与 vite.config.ts 保持一致
// - 测试文件不被任何入口 import，不会进 vite 构建产物；但会进 tsc -b / eslint 范围

import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.ts", "src/**/*.test.tsx", "fnos-shim/**/*.test.ts"],
  },
});
