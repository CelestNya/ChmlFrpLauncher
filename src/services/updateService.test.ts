// @vitest-environment jsdom
// 更新进度归一化：fnOS（shim）载荷带 percentage/stage，桌面形态回退字节换算；
// verifying/applying 定格 100%（下载已完成，进度语义归 UI 层）。

import { beforeEach, describe, expect, it, vi } from "vitest";
import { check } from "@tauri-apps/plugin-updater";
import { updateService } from "./updateService";

vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: vi.fn() }));

const checkMock = check as unknown as ReturnType<typeof vi.fn>;

interface MockEv {
  event: string;
  data?: unknown;
}

function mockInstall(events: MockEv[]): void {
  checkMock.mockResolvedValue({
    available: true,
    version: "1.0.1",
    date: "2026-01-01",
    body: "x",
    downloadAndInstall: vi.fn((cb: (ev: MockEv) => void) => {
      for (const e of events) cb(e);
      return Promise.resolve();
    }),
  });
}

beforeEach(() => {
  checkMock.mockReset();
});

describe("installUpdate 进度归一化", () => {
  it("fnOS 载荷：percentage/stage 透传给 onProgress", async () => {
    mockInstall([
      { event: "Started", data: { contentLength: 100, stage: "connecting" } },
      {
        event: "Progress",
        data: { chunkLength: 5, contentLength: 100, percentage: 5, stage: "downloading" },
      },
    ]);
    const calls: Array<[number, string]> = [];
    await updateService.installUpdate((p, s) => calls.push([p, s ?? ""]));
    expect(calls).toEqual([
      [0, "connecting"],
      [5, "downloading"],
    ]);
  });

  it("verifying/applying 阶段进度定格 100%", async () => {
    mockInstall([
      { event: "Started", data: { contentLength: 100, stage: "connecting" } },
      {
        event: "Progress",
        data: { chunkLength: 100, contentLength: 100, percentage: 100, stage: "verifying" },
      },
      {
        event: "Progress",
        data: { chunkLength: 0, contentLength: 100, percentage: 0, stage: "applying" },
      },
    ]);
    const calls: Array<[number, string]> = [];
    await updateService.installUpdate((p, s) => calls.push([p, s ?? ""]));
    // applying 阶段 daemon 百分比为 0，但语义上应用发生在下载后 → 显示 100
    expect(calls).toEqual([
      [0, "connecting"],
      [100, "verifying"],
      [100, "applying"],
    ]);
  });

  it("桌面形态（无 percentage/stage）：按 chunkLength 累计换算百分比", async () => {
    mockInstall([
      { event: "Started", data: { contentLength: 100 } },
      { event: "Progress", data: { chunkLength: 5, contentLength: 100 } },
      { event: "Progress", data: { chunkLength: 5, contentLength: 100 } },
    ]);
    const calls: Array<[number, string]> = [];
    await updateService.installUpdate((p, s) => calls.push([p, s ?? ""]));
    expect(calls).toEqual([
      [0, "connecting"],
      [5, "downloading"],
      [10, "downloading"],
    ]);
  });

  it("无可用更新时抛错", async () => {
    checkMock.mockResolvedValue({ available: false });
    await expect(updateService.installUpdate()).rejects.toThrow(
      "没有可用的更新",
    );
  });

  it("下载失败错误信息透传", async () => {
    checkMock.mockResolvedValue({
      available: true,
      version: "1.0.1",
      date: "2026-01-01",
      body: "x",
      downloadAndInstall: vi.fn(() =>
        Promise.reject(new Error("所有下载源均失败: a 返回 500；b 连接失败")),
      ),
    });
    await expect(updateService.installUpdate()).rejects.toThrow(
      "安装更新失败: 所有下载源均失败",
    );
  });
});
