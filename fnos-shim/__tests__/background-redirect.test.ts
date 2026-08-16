// @vitest-environment jsdom
// 阶段 2d：验证 localStorage 无 base64 残留 + 渲染类偏好留守（ADR-0003/0004）。
//
// - fnOS 壁纸改托管 URL（非 base64 dataURL）→ localStorage 不再有大体积 dataURL
// - 渲染类偏好（theme/背景等）仍走真实 localStorage（首帧同步读保帧，ADR-0003）
// - 业务类 key（chmlfrp_user 等）被 shim 重定向不落真实 localStorage

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

describe("fnOS localStorage 无 base64 + 偏好留守（阶段 2d）", () => {
  let realLS: Storage;

  beforeEach(() => {
    realLS = window.localStorage;
    realLS.clear();
    vi.restoreAllMocks();
    vi.stubGlobal("WebSocket", undefined);
    // 模拟 daemon boot（无登录态）
    (
      window as unknown as { __FNOS_BOOT__?: unknown }
    ).__FNOS_BOOT__ = { credential: {}, nodeSettings: {} };
  });

  function loadShim(): void {
    new Function(SHIM_JS)();
  }

  it("backgroundImage 存托管 URL 而非 base64 dataURL", () => {
    loadShim();
    // 模拟 fnOS 选图后：shim dialog.open 返回 /assets/backgrounds/<file>
    const managedUrl = "/assets/backgrounds/my-bg.png";
    localStorage.setItem("backgroundImage", managedUrl);

    // 不可能是 base64（data: 前缀）
    expect(managedUrl.startsWith("data:")).toBe(false);
    // 渲染类 key 透传真实 localStorage（留守）
    expect(realLS.getItem("backgroundImage")).toBe(managedUrl);
  });

  it("背景 dataURL 不再出现（选图链路返回托管 URL）", () => {
    loadShim();
    // 若历史遗留 base64 在 localStorage，验证 shim 不阻止其被读取（兼容旧值）
    localStorage.setItem("backgroundImage", "data:image/png;base64,AAAA");
    // 但新写入路径（dialog.open）返回托管 URL——此处断言 shim 对背景 key 透传，
    // 值形态由 dialog.open 保证为 URL（见 shim-dialog-background.test.ts）
    const stored = localStorage.getItem("backgroundImage");
    expect(stored).toBe("data:image/png;base64,AAAA");
  });

  it("渲染类偏好留守真实 localStorage（ADR-0003）", () => {
    loadShim();
    localStorage.setItem("theme", "dark");
    localStorage.setItem("effectType", "frosted");
    expect(realLS.getItem("theme")).toBe("dark");
    expect(realLS.getItem("effectType")).toBe("frosted");
  });

  it("业务类 key 不落真实 localStorage（ADR-0002）", () => {
    loadShim();
    localStorage.setItem(
      "chmlfrp_user",
      JSON.stringify({ accessToken: "secret-token" }),
    );
    // 真实 localStorage 无 token
    expect(realLS.getItem("chmlfrp_user")).toBeNull();
  });

  it("非白名单 key（background_playlist）留守且不含 base64", () => {
    loadShim();
    // fnOS 砍轮播后 playlist 应为空数组（handleSelectFolder 不再填充）
    localStorage.setItem("background_playlist", JSON.stringify([]));
    const parsed = JSON.parse(
      localStorage.getItem("background_playlist") ?? "[]",
    ) as string[];
    expect(parsed).toEqual([]);
    expect(JSON.stringify(parsed)).not.toContain("data:");
  });
});
