// @vitest-environment jsdom
// fnos-shim 测试加载器冒烟测试（批 0：验证 jsdom 执行 IIFE 的测试设施可用）。
//
// tauri-shim.ts 是无 import/export 的 IIFE，不能 import；通过读取源码在全局
// 作用域执行（new Function），每次执行生成全新实例（内部回调表/监听表重置）。

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { beforeEach, describe, expect, it, vi } from "vitest";

const SHIM_SRC = readFileSync(
  path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "../tauri-shim.ts",
  ),
  "utf-8",
);

// shim 是 TypeScript（含类型标注），new Function 无法直接解析 → TS 转译
// （typescript 是直接 devDependency，pnpm 严格布局下可解析；esbuild 是传递依赖不行）
const SHIM_JS = ts.transpileModule(SHIM_SRC, {
  compilerOptions: { target: ts.ScriptTarget.ES2017 },
}).outputText;

/** 在全局作用域执行一次 shim IIFE（全新实例）。 */
function loadShim(): void {
  new Function(SHIM_JS)();
}

interface ShimInternals {
  invoke: (cmd: string, args?: unknown) => Promise<unknown>;
}

function internals(): ShimInternals {
  return (
    window as unknown as {
      __TAURI_INTERNALS__: ShimInternals;
    }
  ).__TAURI_INTERNALS__;
}

describe("fnos-shim 测试加载器（批 0 设施冒烟）", () => {
  beforeEach(() => {
    // jsdom 无 WebSocket：connectEvents 应立即返回（tauri-shim.ts:57 守卫）
    loadShim();
  });

  it("执行后暴露 __FNOS__ / __TAURI__ / __TAURI_INTERNALS__", () => {
    const w = window as unknown as Record<string, unknown>;
    expect(w.__FNOS__).toBe(true);
    expect(w.__TAURI__).toBe(true);
    expect(internals().invoke).toBeTypeOf("function");
  });

  it("plugin:app|version 从 /api/bootstrap 取版本号", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      json: async () => ({ version: "0.7.5" }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(internals().invoke("plugin:app|version")).resolves.toBe(
      "0.7.5",
    );
    expect(fetchMock).toHaveBeenCalledWith("/api/bootstrap");
  });

  it("透传命令 POST /api/invoke 并解包 {ok,data}", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: async () => ({ ok: true, data: "running" }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(
      internals().invoke("is_frpc_running", { tunnel_id: 1 }),
    ).resolves.toBe("running");
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/invoke",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({
          cmd: "is_frpc_running",
          args: { tunnel_id: 1 },
        }),
      }),
    );
  });
});
