// @vitest-environment jsdom
// fnos-shim 阶段 2b：dialog.open 选图 → daemon save_background_image → 托管 URL。
//
// ADR-0004：fnOS 壁纸从 base64 dataURL 改为文件路径。选图后 shim 把 dataURL
// 上传 daemon（save_background_image，解 base64 落盘 data_dir/backgrounds/），
// 返回 assets/backgrounds/<file> 相对托管 URL（网关前缀 /app/chmlfrp/ 由浏览器相对解析自动加）。
// 前端 getBackgroundType 识别非 data:/app:// 前缀直接当 src，形态兼容。

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
const SHIM_JS = ts.transpileModule(SHIM_SRC, {
  compilerOptions: { target: ts.ScriptTarget.ES2017 },
}).outputText;

function loadShim(): void {
  new Function(SHIM_JS)();
}

function internals() {
  return (
    window as unknown as {
      __TAURI_INTERNALS__: { invoke: (c: string, a: unknown) => Promise<unknown> };
    }
  ).__TAURI_INTERNALS__;
}

/** stub FileReader 返回 dataURL，模拟浏览器选图。 */
function stubFileReader(dataUrl: string): void {
  class MockFileReader {
    result: string | null = dataUrl;
    onload: (() => void) | null = null;
    onerror: (() => void) | null = null;
    readAsDataURL() {
      if (this.onload) this.onload();
    }
  }
  vi.stubGlobal("FileReader", MockFileReader);
}

describe("fnos-shim dialog.open 壁纸托管（阶段 2b）", () => {
  let acceptValue = "";

  beforeEach(() => {
    acceptValue = "";
    vi.restoreAllMocks();
    vi.stubGlobal("WebSocket", undefined);
    const origCreate = document.createElement.bind(document);
    vi.spyOn(document, "createElement").mockImplementation((tag: string) => {
      const el = origCreate(tag);
      if (tag === "input") {
        Object.defineProperty(el, "accept", {
          get: () => acceptValue,
          set: (v: string) => {
            acceptValue = v;
          },
        });
        Object.defineProperty(el, "click", {
          value: () => {
            /* 记录点击（断言点击发生在选图） */
          },
        });
      }
      return el;
    });
    loadShim();
  });

  it("选图后调 save_background_image 并返回托管 URL", async () => {
    const dataUrl = "data:image/png;base64,aGVsbG8=";
    stubFileReader(dataUrl);

    let captured: { cmd: string; args: unknown } | null = null;
    const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
      if (typeof url === "string" && url.endsWith("/api/invoke")) {
        const body = JSON.parse(String(init?.body)) as {
          cmd: string;
          args: unknown;
        };
        captured = body;
        return {
          ok: true,
          status: 200,
          json: async () => ({
            ok: true,
            data: "backgrounds/my-bg.png",
          }),
        };
      }
      throw new Error(`unexpected fetch: ${url}`);
    });
    vi.stubGlobal("fetch", fetchMock);

    // 模拟 fnOS 网关环境：页面 URL 在 /app/chmlfrp/ 前缀下
    const origHref = window.location.href;
    // jsdom 只读 location.href，用 history.replaceState 改路径
    window.history.replaceState({}, "", "/app/chmlfrp/");

    // 模拟选图：触发 shim 内部 input 的 onchange（需要拿到 shim 创建的 input）
    // ——shim 内部 input.click() 被 spy 记录，但 onchange 是内部闭包。改用
    // document 上被 mock 的 input 实例：先调 invoke 拿到 Promise，再手动触发
    const promise = internals().invoke("plugin:dialog|open", {
      options: {
        multiple: false,
        filters: [{ name: "媒体文件", extensions: ["png"] }],
      },
    });

    // shim 内部创建的 input 有 onchange；触发它（files 需要模拟）
    // —— mock createElement 返回的 input 记录了实例
    const mockInputs = vi.mocked(document.createElement).mock.results
      .map((r) => r.value)
      .filter((el) => (el as HTMLElement).tagName === "INPUT");
    const input = mockInputs[0] as HTMLInputElement & {
      files?: FileList;
    };
    Object.defineProperty(input, "files", {
      value: [{ name: "my-bg.png", size: 100 }],
    });
    (input as unknown as { onchange: (() => void) | null }).onchange?.();

    const result = await promise;
    // 返回含网关前缀的绝对 URL（/app/chmlfrp/assets/backgrounds/<file>）——
    // new URL(rel, location.href) 解析，存储后刷新/路由变化稳定可渲染
    expect(result).toBe("http://localhost:3000/app/chmlfrp/assets/backgrounds/my-bg.png");
    expect(captured).not.toBeNull();
    expect(captured!.cmd).toBe("save_background_image");
    const args = captured!.args as { dataUrl: string; fileName: string };
    expect(args.dataUrl).toBe(dataUrl);
    expect(args.fileName).toBe("my-bg.png");

    window.history.replaceState({}, "", origHref);
  });

  it("2MB 上限仍生效（拒绝超大文件）", async () => {
    const alertSpy = vi.spyOn(window, "alert").mockImplementation(() => {});
    stubFileReader("data:image/png;base64,AAAA");
    vi.stubGlobal("fetch", vi.fn());

    const promise = internals().invoke("plugin:dialog|open", {
      options: { multiple: false, filters: [{ name: "媒体文件", extensions: ["png"] }] },
    });
    const mockInputs = vi.mocked(document.createElement).mock.results
      .map((r) => r.value)
      .filter((el) => (el as HTMLElement).tagName === "INPUT");
    const input = mockInputs[0] as HTMLInputElement & { files?: FileList };
    Object.defineProperty(input, "files", {
      value: [{ name: "big.png", size: 3 * 1024 * 1024 }],
    });
    (input as unknown as { onchange: (() => void) | null }).onchange?.();

    const result = await promise;
    expect(alertSpy).toHaveBeenCalled();
    expect(result).toBeNull();
    alertSpy.mockRestore();
  });
});
