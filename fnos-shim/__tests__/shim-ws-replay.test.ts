// @vitest-environment jsdom
// B2：页面加载即连 WS，daemon 立即全量补发，此时 React 尚未挂载 listener → 帧全丢。
// 修复：shim 对无 listener 的帧做环形暂存，首次 listen 注册时以 replay:true 重放。
// （审查 shim H2：违背 daemon 声明的「页面重载后补发历史日志会重新显示」特性）

import { beforeEach, describe, expect, it, vi } from "vitest";
import { getLastWebSocket, internals, loadShim } from "./helpers";

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

describe("B2 首连补发帧不丢弃", () => {
  beforeEach(() => {
    MockWebSocket.instances = [];
    vi.stubGlobal("WebSocket", MockWebSocket);
    loadShim();
  });

  it("listener 注册前到达的帧，注册时以 replay:true 重放", async () => {
    const ws = getLastWebSocket();
    ws.emitMessage({
      type: "frpc-log",
      payload: { tunnel_id: 1, message: "第一条", timestamp: "t" },
    });
    ws.emitMessage({
      type: "frpc-log",
      payload: { tunnel_id: 1, message: "第二条", timestamp: "t" },
    });

    const received: unknown[] = [];
    const handler = internals().transformCallback((data) => received.push(data));
    await internals().invoke("plugin:event|listen", {
      event: "frpc-log",
      handler,
    });

    expect(received).toHaveLength(2);
    expect((received[0] as { replay?: boolean }).replay).toBe(true);
    expect((received[1] as { replay?: boolean }).replay).toBe(true);
    expect((received[0] as { payload: { message: string } }).payload.message).toBe(
      "第一条",
    );
  });

  it("重放只针对新注册的事件，其他事件帧不受影响", async () => {
    const ws = getLastWebSocket();
    ws.emitMessage({
      type: "download-progress",
      payload: { downloaded: 5, total: 100 },
    });

    const received: unknown[] = [];
    const handler = internals().transformCallback((data) => received.push(data));
    await internals().invoke("plugin:event|listen", {
      event: "frpc-log",
      handler,
    });
    // 注册的是 frpc-log：download-progress 的暂存帧不应被重放
    expect(received).toHaveLength(0);

    // 之后 frpc-log 帧正常实时分发
    ws.emitMessage({
      type: "frpc-log",
      payload: { tunnel_id: 1, message: "实时帧", timestamp: "t" },
    });
    expect(received).toHaveLength(1);
    expect((received[0] as { replay?: boolean }).replay).toBe(false);
  });

  it("暂存帧重放后清空，不会重复重放", async () => {
    const ws = getLastWebSocket();
    ws.emitMessage({
      type: "frpc-log",
      payload: { tunnel_id: 1, message: "x", timestamp: "t" },
    });

    const received: unknown[] = [];
    const handler = internals().transformCallback((data) => received.push(data));
    await internals().invoke("plugin:event|listen", { event: "frpc-log", handler });
    await internals().invoke("plugin:event|listen", { event: "frpc-log", handler });
    expect(received).toHaveLength(1);
  });
});
