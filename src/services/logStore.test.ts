// @vitest-environment jsdom
// B7 去重窗口（replay 先到/实时后到双向）/ B8 桌面回归门控 / B9 clearLogs 联动

import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { frpcManager } from "./frpcManager";
import { logStore } from "./logStore";
import type { LogMessage } from "./frpcManager";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("./frpcManager", () => ({
  frpcManager: { listenToLogs: vi.fn() },
}));

const invokeMock = invoke as unknown as ReturnType<typeof vi.fn>;
const listenToLogsMock = frpcManager.listenToLogs as unknown as ReturnType<
  typeof vi.fn
>;

function setFnos(value: boolean): void {
  (window as unknown as { __FNOS__?: boolean }).__FNOS__ = value;
}

function resetStore(): void {
  const s = logStore as unknown as {
    logs: LogMessage[];
    replaySeen: Set<string>;
    isListening: boolean;
  };
  s.logs = [];
  s.replaySeen = new Set();
  s.isListening = false;
}

function frame(overrides: Partial<LogMessage> = {}): LogMessage {
  return { tunnel_id: 1, timestamp: "t", message: "m", ...overrides };
}

let logCb: ((log: LogMessage) => void) | null = null;

beforeEach(() => {
  invokeMock.mockReset();
  listenToLogsMock.mockReset();
  listenToLogsMock.mockImplementation(
    async (cb: (log: LogMessage) => void) => {
      logCb = cb;
    },
  );
  setFnos(true);
  resetStore();
});

describe("B7 去重窗口（双向）", () => {
  it("replay 先到、实时副本后到：只显示一条", async () => {
    await logStore.startListening();
    logCb!({ ...frame(), replay: true });
    logCb!(frame());
    expect(logStore.getLogs().filter((l) => l.message === "m")).toHaveLength(1);
  });

  it("实时先到、replay 后到：只显示一条", async () => {
    await logStore.startListening();
    logCb!(frame());
    logCb!({ ...frame(), replay: true });
    expect(logStore.getLogs().filter((l) => l.message === "m")).toHaveLength(1);
  });

  it("不同消息不去重", async () => {
    await logStore.startListening();
    logCb!(frame());
    logCb!(frame({ message: "other" }));
    expect(logStore.getLogs()).toHaveLength(2);
  });

  it("不同隧道同内容不去重", async () => {
    await logStore.startListening();
    logCb!(frame({ tunnel_id: 1 }));
    logCb!(frame({ tunnel_id: 2 }));
    expect(logStore.getLogs()).toHaveLength(2);
  });
});

describe("B8 桌面回归门控", () => {
  it("桌面环境：addLog 不去重（与上游原版一致）", () => {
    setFnos(false);
    logStore.addLog(frame());
    logStore.addLog(frame());
    expect(logStore.getLogs()).toHaveLength(2);
  });

  it("fnOS 环境：addLog 同秒同内容去重（两组件重复生成 launcher 日志防护）", () => {
    setFnos(true);
    logStore.addLog(frame());
    logStore.addLog(frame());
    expect(logStore.getLogs()).toHaveLength(1);
  });

  it("桌面环境：listenToLogs 不去重、replaySeen 不增长", async () => {
    setFnos(false);
    await logStore.startListening();
    logCb!(frame());
    logCb!({ ...frame(), replay: true });
    expect(logStore.getLogs()).toHaveLength(2);
    const seen = (logStore as unknown as { replaySeen: Set<string> }).replaySeen;
    expect(seen.size).toBe(0);
  });
});

describe("B9 clearLogs 联动 daemon 缓冲", () => {
  it("fnOS：daemon 清空成功返回 true 并调用命令", async () => {
    setFnos(true);
    invokeMock.mockResolvedValue(undefined);
    await expect(logStore.clearLogs()).resolves.toBe(true);
    expect(invokeMock).toHaveBeenCalledWith("clear_log_history");
  });

  it("fnOS：daemon 清空失败返回 false（UI 据此提示）", async () => {
    setFnos(true);
    invokeMock.mockRejectedValue(new Error("down"));
    await expect(logStore.clearLogs()).resolves.toBe(false);
  });

  it("桌面：不调用命令，返回 true", async () => {
    setFnos(false);
    await expect(logStore.clearLogs()).resolves.toBe(true);
    expect(invokeMock).not.toHaveBeenCalled();
  });
});
