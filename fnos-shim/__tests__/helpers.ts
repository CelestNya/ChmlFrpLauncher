// fnos-shim 测试共享工具（非测试文件，vitest include 不匹配 *.test.ts）。
// 读取 tauri-shim.ts 源码，经 typescript.transpileModule 转译后在全局作用域执行
// （shim 是无 import/export 的 IIFE，每次执行生成全新实例）。

import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const SHIM_SRC = readFileSync(
  path.resolve(
    path.dirname(fileURLToPath(import.meta.url)),
    "../tauri-shim.ts",
  ),
  "utf-8",
);

const SHIM_JS = ts.transpileModule(SHIM_SRC, {
  compilerOptions: { target: ts.ScriptTarget.ES2017 },
}).outputText;

export interface ShimInternals {
  invoke: (
    cmd: string,
    args?: unknown,
    options?: Record<string, unknown>,
  ) => Promise<unknown>;
  transformCallback: (cb: (data: unknown) => void, once?: boolean) => number;
  unregisterCallback: (id: number) => void;
  convertFileSrc: (filePath: string) => string;
}

/** 在全局作用域执行一次 shim IIFE（全新实例：回调表/监听表/帧缓冲重置）。 */
export function loadShim(): void {
  new Function(SHIM_JS)();
}

/** 获取 shim 暴露的 __TAURI_INTERNALS__。 */
export function internals(): ShimInternals {
  return (
    window as unknown as { __TAURI_INTERNALS__: ShimInternals }
  ).__TAURI_INTERNALS__;
}

/** 设置/清除 fnOS 环境标记（shim 会置 true；桌面环境测试需清除）。 */
export function setFnos(value: boolean): void {
  (window as unknown as { __FNOS__?: boolean }).__FNOS__ = value;
}

/** 捕获 shim 创建的 WebSocket 实例（需在 loadShim 前 stub WebSocket）。 */
export interface MockWebSocketLike {
  onopen: (() => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  onclose: (() => void) | null;
  onerror: (() => void) | null;
  url: string;
  close: () => void;
  emitMessage: (frame: unknown) => void;
}

export function getLastWebSocket(): MockWebSocketLike {
  const w = window as unknown as {
    __LAST_WS__?: MockWebSocketLike;
  };
  if (!w.__LAST_WS__) throw new Error("无 WebSocket 实例（需先 stub WebSocket 再 loadShim）");
  return w.__LAST_WS__;
}
