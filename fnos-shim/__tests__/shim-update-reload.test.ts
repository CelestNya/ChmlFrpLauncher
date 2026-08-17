// @vitest-environment jsdom
// B8：更新完成后 daemon 重启 → WS 断线重连 → 版本变化 → 自动刷新。
// 用户反馈：更新完提示「重启后完成更新」令人迷惑——不再需要手动操作。

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { loadShim } from "./helpers";

class MockWebSocket {
  static instances: MockWebSocket[] = [];
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: unknown }) => void) | null = null;
  onclose: (() => void) | null = null;
  onerror: (() => void) | null = null;
  url: string;

  constructor(url: string) {
    this.url = url;
    MockWebSocket.instances.push(this);
    (window as unknown as { __LAST_WS__?: MockWebSocket }).__LAST_WS__ = this;
  }
  close() {}
  emitMessage(frame: unknown) {
    this.onmessage?.({ data: JSON.stringify(frame) });
  }
}

describe("B8 更新完成后自动刷新", () => {
  const reloadMock = vi.fn();

  beforeEach(() => {
    vi.useFakeTimers();
    MockWebSocket.instances = [];
    reloadMock.mockClear();
    vi.stubGlobal("WebSocket", MockWebSocket);
    // shim 在 loadShim 时立即连 WS：location 必须先 mock（gatewayPrefix 读 pathname）
    Object.defineProperty(window, "location", {
      configurable: true,
      writable: true,
      value: {
        protocol: "http:",
        host: "127.0.0.1",
        pathname: "/app/chmlfrp/",
        reload: reloadMock,
      },
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("重连后发现版本变化 → 显示提示并自动刷新", async () => {
    // bootstrap 返回值：首次连接 1.5.3（基线），重连后 1.5.4（已升级）
    const versionResponses = ["1.5.3", "1.5.4"];
    const fetchMock = vi.fn().mockImplementation(() =>
      Promise.resolve({
        json: () =>
          Promise.resolve({ version: versionResponses.shift() ?? "1.5.4" }),
      }),
    );
    vi.stubGlobal("fetch", fetchMock);

    loadShim();
    const firstWs = MockWebSocket.instances[0];
    // 首次连接：记录基线，不刷新
    firstWs.onopen?.();
    await Promise.resolve(); // 等 fetch 微任务
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(reloadMock).not.toHaveBeenCalled();

    // 模拟 daemon 重启（更新应用）：WS 断开 → 退避重连
    firstWs.onclose?.();
    await vi.advanceTimersByTimeAsync(1000); // 第一次重连退避 1s
    const secondWs = MockWebSocket.instances[1];
    expect(secondWs).toBeDefined();
    secondWs.onopen?.();
    await Promise.resolve(); // 等 fetch 微任务
    await vi.advanceTimersByTimeAsync(500); // 覆盖重连比对 + reload 前的 400ms 延迟

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(reloadMock).toHaveBeenCalledTimes(1);
  });

  it("普通断线重连（版本未变）→ 不刷新", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue({ json: async () => ({ version: "1.5.3" }) }),
    );

    loadShim();
    const firstWs = MockWebSocket.instances[0];
    firstWs.onopen?.();
    await Promise.resolve();

    firstWs.onclose?.();
    await vi.advanceTimersByTimeAsync(1000);
    const secondWs = MockWebSocket.instances[1];
    secondWs.onopen?.();
    await Promise.resolve();

    expect(reloadMock).not.toHaveBeenCalled();
  });

  it("重连时 daemon 尚未就绪（fetch 失败）→ 忽略，等待下一轮重连", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce({ json: async () => ({ version: "1.5.3" }) })
      .mockRejectedValueOnce(new Error("daemon 重启中"))
      .mockResolvedValueOnce({ json: async () => ({ version: "1.5.4" }) });
    vi.stubGlobal("fetch", fetchMock);

    loadShim();
    const firstWs = MockWebSocket.instances[0];
    firstWs.onopen?.();
    await Promise.resolve();

    // 第一次重连：fetch 失败 → 不刷新，版本基线保持 1.5.3
    firstWs.onclose?.();
    await vi.advanceTimersByTimeAsync(1000);
    MockWebSocket.instances[1].onopen?.();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(500);
    expect(reloadMock).not.toHaveBeenCalled();

    // 第二次重连：daemon 就绪，版本变化 → 刷新
    MockWebSocket.instances[1].onclose?.();
    await vi.advanceTimersByTimeAsync(2000); // 退避 2s
    MockWebSocket.instances[2].onopen?.();
    await Promise.resolve();
    await vi.advanceTimersByTimeAsync(500);

    expect(reloadMock).toHaveBeenCalledTimes(1);
  });
});