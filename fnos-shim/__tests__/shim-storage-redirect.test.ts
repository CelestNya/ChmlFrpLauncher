// @vitest-environment jsdom
// fnos-shim 阶段 1c：localStorage 白名单 key 重定向到 daemon（前后端分离重构）。
//
// 机制（ADR-0002/0003）：
// - shim 替换 window.localStorage 为代理；白名单 key（chmlfrp_user / frpc_proxy_config /
//   frpcLogLevel / bypassProxy / restartOnEdit）不落真实 localStorage，转发 daemon。
// - 首帧同步读由 daemon 注入的 __FNOS_BOOT__（credential + nodeSettings）注水内存缓存。
// - 非白名单 key（主题/音效等渲染类偏好）透传真实 localStorage。

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

interface InvokeCall {
  cmd: string;
  args: Record<string, unknown>;
}

/** stub fetch：记录所有 /api/invoke 调用，按需返回。 */
function stubInvoke(): {
  calls: InvokeCall[];
  respond: (cmd: string, data: unknown) => void;
} {
  const calls: InvokeCall[] = [];
  const responders = new Map<string, unknown>();
  const fetchMock = vi.fn(async (url: string, init?: RequestInit) => {
    if (typeof url === "string" && url.endsWith("/api/invoke")) {
      const body = JSON.parse(String(init?.body)) as {
        cmd: string;
        args: Record<string, unknown>;
      };
      calls.push({ cmd: body.cmd, args: body.args });
      const data = responders.get(body.cmd);
      return {
        ok: true,
        status: 200,
        json: async () =>
          data === undefined
            ? { ok: true, data: null }
            : { ok: true, data },
      };
    }
    throw new Error(`unexpected fetch: ${url}`);
  });
  vi.stubGlobal("fetch", fetchMock);
  return {
    calls,
    respond: (cmd: string, data: unknown) => responders.set(cmd, data),
  };
}

/** 设 boot blob（模拟 daemon 注入 index.html）。 */
function setBoot(credential: unknown, nodeSettings: unknown): void {
  (
    window as unknown as { __FNOS_BOOT__?: unknown }
  ).__FNOS_BOOT__ = {
    credential,
    nodeSettings,
  };
}

/** 调 shim 暴露的 invoke（经 stubInvoke 的 fetch 转发到 daemon）。 */
async function internalsInvoke(cmd: string, args: Record<string, unknown>) {
  const ta = (window as unknown as {
    __TAURI_INTERNALS__: { invoke: (c: string, a: unknown) => Promise<unknown> };
  }).__TAURI_INTERNALS__;
  return ta.invoke(cmd, args);
}

describe("fnos-shim localStorage 重定向（阶段 1c）", () => {
  // 真实 localStorage（jsdom 实例，shim 替换 window.localStorage 前保存）
  let realLS: Storage;

  beforeEach(() => {
    realLS = window.localStorage;
    realLS.clear();
    vi.restoreAllMocks();
    // 默认无 boot（每例按需 setBoot）
    (
      window as unknown as { __FNOS_BOOT__?: unknown }
    ).__FNOS_BOOT__ = undefined;
    // jsdom 无 WebSocket：connectEvents 直接返回
    vi.stubGlobal("WebSocket", undefined);
  });

  it("非白名单 key 透传真实 localStorage", () => {
    setBoot(null, null);
    loadShim();
    localStorage.setItem("theme", "dark");
    expect(localStorage.getItem("theme")).toBe("dark");
    expect(realLS.getItem("theme")).toBe("dark");
    expect(realLS.getItem("chmlfrp_user")).toBeNull();
  });

  it("boot 注水后 chmlfrp_user 同步可读（首帧不闪烁）", () => {
    setBoot(
      // daemon 注入格式：Rust Credential 序列化 → snake_case
      {
        username: "test_user",
        access_token: "access-token",
        refresh_token: "refresh-token",
        token_type: "Bearer",
        access_token_expires_at: 1750000000,
      },
      { log_level: "debug", bypass_proxy: true },
    );
    loadShim();

    const userStr = localStorage.getItem("chmlfrp_user");
    expect(userStr).not.toBeNull();
    const parsed = JSON.parse(userStr!) as Record<string, unknown>;
    expect(parsed.accessToken).toBe("access-token");
    expect(parsed.tokenType).toBe("Bearer");
    // 未登录时返回 null
    expect(localStorage.getItem("frpcLogLevel")).toBe("debug");
    expect(localStorage.getItem("bypassProxy")).toBe("true");
  });

  it("setItem(chmlfrp_user) 转发 save_credential 且不落真实存储", async () => {
    setBoot(null, null);
    const { calls } = stubInvoke();
    loadShim();

    localStorage.setItem(
      "chmlfrp_user",
      JSON.stringify({
        username: "u",
        usertoken: "legacy",
        accessToken: "access",
        refreshToken: "refresh",
        tokenType: "Bearer",
        accessTokenExpiresAt: 1750000000,
      }),
    );

    // 真实 localStorage 不含该 key（token 物理离开浏览器）
    expect(realLS.getItem("chmlfrp_user")).toBeNull();
    // 同步回读命中内存缓存
    const back = JSON.parse(localStorage.getItem("chmlfrp_user")!) as {
      accessToken: string;
    };
    expect(back.accessToken).toBe("access");

    // 异步转发 daemon
    await vi.waitFor(() => {
      expect(calls.some((c) => c.cmd === "save_credential")).toBe(true);
    });
    const save = calls.find((c) => c.cmd === "save_credential")!;
    const cred = save.args.credential as Record<string, unknown>;
    // daemon Credential 结构体字段为 snake_case（invoke.rs 无 rename_all）
    expect(cred.access_token).toBe("access");
    expect(cred.refresh_token).toBe("refresh");
  });

  it("setItem(frpc_proxy_config) 转发 node_settings 代理段", async () => {
    setBoot(null, null);
    const { calls } = stubInvoke();
    loadShim();

    localStorage.setItem(
      "frpc_proxy_config",
      JSON.stringify({
        enabled: true,
        type: "http",
        host: "proxy.example.com",
        port: 8080,
        username: "user",
        password: "secret",
        forceTls: false,
        kcpOptimization: true,
      }),
    );

    expect(realLS.getItem("frpc_proxy_config")).toBeNull();
    await vi.waitFor(() => {
      expect(calls.some((c) => c.cmd === "set_node_settings")).toBe(true);
    });
    const set = calls.find((c) => c.cmd === "set_node_settings")!;
    const ns = set.args.settings as Record<string, unknown>;
    expect(ns.proxy_enabled).toBe(true);
    expect(ns.proxy_host).toBe("proxy.example.com");
    expect(ns.proxy_password).toBe("secret");

    // 回读同步命中缓存（camelCase 还原）
    const back = JSON.parse(localStorage.getItem("frpc_proxy_config")!) as {
      host: string;
    };
    expect(back.host).toBe("proxy.example.com");
  });

  it("setItem(frpcLogLevel / bypassProxy / restartOnEdit) 转发 node_settings", async () => {
    setBoot(null, null);
    const { calls } = stubInvoke();
    loadShim();

    localStorage.setItem("frpcLogLevel", "debug");
    localStorage.setItem("bypassProxy", "false");
    localStorage.setItem("restartOnEdit", "true");

    await vi.waitFor(() => {
      expect(
        calls.filter((c) => c.cmd === "set_node_settings").length,
      ).toBeGreaterThanOrEqual(1);
    });
    const ns = calls
      .filter((c) => c.cmd === "set_node_settings")
      .at(-1)!.args.settings as Record<string, unknown>;
    expect(ns.log_level).toBe("debug");
    expect(ns.bypass_proxy).toBe(false);
    expect(ns.restart_on_edit).toBe(true);
  });

  it("removeItem(chmlfrp_user) 转发 clear_credential", async () => {
    setBoot(null, null);
    const { calls } = stubInvoke();
    loadShim();
    localStorage.setItem(
      "chmlfrp_user",
      JSON.stringify({ accessToken: "access" }),
    );

    localStorage.removeItem("chmlfrp_user");
    expect(localStorage.getItem("chmlfrp_user")).toBeNull();
    await vi.waitFor(() => {
      expect(calls.some((c) => c.cmd === "clear_credential")).toBe(true);
    });
  });

  it("非白名单 removeItem 透传真实存储", () => {
    setBoot(null, null);
    loadShim();
    localStorage.setItem("theme", "dark");
    localStorage.removeItem("theme");
    expect(realLS.getItem("theme")).toBeNull();
  });

  it("未登录 boot 时 chmlfrp_user 返回 null（登录态正确判定）", () => {
    setBoot({}, {});
    loadShim();
    expect(localStorage.getItem("chmlfrp_user")).toBeNull();
  });

  it("凭据完整往返：usergroup/tunnelCount 等用户画像字段不丢（review P2）", async () => {
    setBoot(null, null);
    const { calls } = stubInvoke();
    loadShim();

    // 前端 saveStoredUser 写入的完整 StoredUser（含 usergroup）
    localStorage.setItem(
      "chmlfrp_user",
      JSON.stringify({
        username: "u",
        usergroup: "free",
        userimg: "img.png",
        accessToken: "access",
        refreshToken: "refresh",
        tokenType: "Bearer",
        accessTokenExpiresAt: 1750000000,
        tunnelCount: 3,
        tunnel: 2,
      }),
    );

    // daemon 收到完整字段（snake_case）
    await vi.waitFor(() => {
      expect(calls.some((c) => c.cmd === "save_credential")).toBe(true);
    });
    const save = calls.find((c) => c.cmd === "save_credential")!;
    const cred = save.args.credential as Record<string, unknown>;
    expect(cred.usergroup).toBe("free");
    expect(cred.tunnel_count).toBe(3);

    // 回读同步命中缓存 → camelCase 还原 usergroup（会员门控关键字段）
    const back = JSON.parse(localStorage.getItem("chmlfrp_user")!) as {
      usergroup: string;
      tunnelCount: number;
    };
    expect(back.usergroup).toBe("free");
    expect(back.tunnelCount).toBe(3);
  });

  it("登出顺序：先停隧道再清凭据（ADR-0002 定死）", async () => {
    // 模拟 logout.ts performLogout：先 stopAllRunningTunnels（stop_frpc），
    // 再 clearStoredUser（shim 拦截转 clear_credential）
    setBoot(null, null);
    const { calls } = stubInvoke();
    loadShim();

    // 先注入凭据（模拟已登录）
    localStorage.setItem(
      "chmlfrp_user",
      JSON.stringify({ accessToken: "access" }),
    );
    await vi.waitFor(() => {
      expect(calls.some((c) => c.cmd === "save_credential")).toBe(true);
    });

    // 登出编排：先停隧道（stop_frpc）
    await internalsInvoke("stop_frpc", { tunnel_id: 1 });
    // 再清凭据（shim 拦截 removeItem → clear_credential）
    localStorage.removeItem("chmlfrp_user");

    // 顺序断言：stop_frpc 必须先于 clear_credential
    const stopIdx = calls.findIndex((c) => c.cmd === "stop_frpc");
    const clearIdx = calls.findIndex((c) => c.cmd === "clear_credential");
    expect(stopIdx).toBeGreaterThanOrEqual(0);
    expect(clearIdx).toBeGreaterThanOrEqual(0);
    expect(stopIdx).toBeLessThan(clearIdx);
  });

  it("clear_credential 后 chmlfrp_user 同步返回 null", () => {
    setBoot(
      { access_token: "access" },
      {},
    );
    loadShim();
    expect(localStorage.getItem("chmlfrp_user")).not.toBeNull();

    // 登出：shim 拦截 removeItem → 清内存缓存
    localStorage.removeItem("chmlfrp_user");
    expect(localStorage.getItem("chmlfrp_user")).toBeNull();
  });
});
