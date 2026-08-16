// src/services/api.test.ts
// 掉登录修复单测（e051411 移植验证，2026-08-15）：
// 并发请求共享 token 刷新 Promise——refresh 端点只调用 1 次，所有请求拿到同一结果。
// 修复前：每个并发请求各自刷新 → refresh token 轮换踩踏 → 误清登录态（掉登录）。
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fetchTunnels, saveStoredUser, type StoredUser } from "./api";

function makeExpiredUser(): StoredUser {
  return {
    username: "test-user",
    usergroup: "free",
    accessToken: "expired-access-token",
    refreshToken: "refresh-token-1",
    accessTokenExpiresAt: Date.now() - 60_000, // 已过期 → 下次请求触发刷新
    tokenType: "Bearer",
  };
}

describe("token 并发刷新（掉登录修复）", () => {
  let refreshCalls = 0;
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    refreshCalls = 0;
    localStorage.clear();
    fetchMock = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      if (url.includes("/oauth2/token")) {
        refreshCalls += 1;
        return new Response(
          JSON.stringify({
            access_token: "new-access-token",
            refresh_token: "new-refresh-token",
            expires_in: 3600,
            token_type: "Bearer",
          }),
          { status: 200 },
        );
      }
      if (url.includes("/tunnel")) {
        return new Response(JSON.stringify({ code: 200, data: [] }), {
          status: 200,
        });
      }
      return new Response("not found", { status: 404 });
    });
    vi.stubGlobal("fetch", fetchMock);
    saveStoredUser(makeExpiredUser());
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("并发请求共享同一个刷新 Promise——refresh 只调用 1 次", async () => {
    const results = await Promise.all([
      fetchTunnels(),
      fetchTunnels(),
      fetchTunnels(),
    ]);

    expect(refreshCalls).toBe(1);
    expect(results).toHaveLength(3);
  });

  it("刷新成功后保存新 token，后续请求不再重复刷新", async () => {
    await fetchTunnels();
    expect(refreshCalls).toBe(1);

    // token 已刷新且未过期 → 直接使用，不再触发刷新
    await fetchTunnels();
    expect(refreshCalls).toBe(1);
  });
});
