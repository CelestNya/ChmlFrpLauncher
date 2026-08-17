// @vitest-environment jsdom
// B9：异步下载流程（fnOS 网关 504 实测修复）——
// POST download 立即返回，daemon 后台下载完成后经 download-result 事件推送，
// shim 收到 ok 后再 POST apply；错误经事件带出。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getLastWebSocket, internals, loadShim } from "./helpers";

class MockWS {
  static instances: MockWS[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  url: string;
  constructor(url: string) {
    this.url = url;
    MockWS.instances.push(this);
    (window as unknown as { __LAST_WS__?: MockWS }).__LAST_WS__ = this;
  }
  close() {}
  emitMessage(frame: unknown) {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
}

function fetchMockByUrl(routes: Record<string, unknown>) {
  return vi.fn((url: string) => {
    const hit = Object.entries(routes).find(([k]) => url.includes(k));
    if (!hit) return Promise.reject(new Error(`no mock for ${url}`));
    return Promise.resolve({ json: async () => hit[1] });
  });
}

describe("B9 异步下载流程", () => {
  beforeEach(() => {
    MockWS.instances = [];
    vi.stubGlobal("WebSocket", MockWS);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("download 立即返回 → download-result ok → 调 apply 后 resolve", async () => {
    const fetchMock = fetchMockByUrl({
      "/api/update/download": { ok: true, data: "下载已开始" },
      "/api/update/apply": { ok: true, data: "更新已应用" },
    });
    vi.stubGlobal("fetch", fetchMock);
    loadShim();

    const installing = internals().invoke("plugin:updater|download_and_install", {
      onEvent: 1,
      rid: 1,
    });
    await Promise.resolve();
    // POST download 已发出且立即返回（不等下载）
    const calls = fetchMock.mock.calls.map((c) => c[0] as string);
    expect(calls.filter((u) => u.includes("download")).length).toBe(1);
    expect(calls.some((u) => u.includes("apply"))).toBe(false);

    // daemon 完成：事件推送
    getLastWebSocket().emitMessage({
      type: "download-result",
      payload: { ok: true, staged: "/x/staged", error: null },
    });
    await installing;
    await Promise.resolve();

    const after = fetchMock.mock.calls.map((c) => c[0] as string);
    expect(after.some((u) => u.includes("/api/update/apply"))).toBe(true);
  });

  it("download-result error → reject 带聚合错误信息", async () => {
    vi.stubGlobal(
      "fetch",
      fetchMockByUrl({
        "/api/update/download": { ok: true, data: "下载已开始" },
      }),
    );
    loadShim();

    const installing = internals().invoke("plugin:updater|download_and_install", {
      onEvent: 1,
      rid: 1,
    });
    await Promise.resolve();
    getLastWebSocket().emitMessage({
      type: "download-result",
      payload: {
        ok: false,
        staged: null,
        error: "所有下载源均失败: a 返回 500；b 连接失败: unexpected end of file",
      },
    });
    await expect(installing).rejects.toThrow("所有下载源均失败");
  });

  it("download POST 立即失败 → reject 且不等待事件", async () => {
    vi.stubGlobal(
      "fetch",
      fetchMockByUrl({
        "/api/update/download": { ok: false, error: "下载更新失败: 源不可达" },
      }),
    );
    loadShim();

    const installing = internals().invoke("plugin:updater|download_and_install", {
      onEvent: 1,
      rid: 1,
    });
    await expect(installing).rejects.toThrow("下载更新失败: 源不可达");
  });
});