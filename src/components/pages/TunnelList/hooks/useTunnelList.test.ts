// @vitest-environment jsdom

import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Tunnel } from "@/services/api";
import { tunnelListCache } from "../cache";
import type { UnifiedTunnel } from "../types";

const fetchTunnels = vi.fn();
const getCustomTunnels = vi.fn();

vi.mock("@/services/api", () => ({
  fetchTunnels,
}));

vi.mock("@/services/frpcManager", () => ({
  frpcManager: {
    isTunnelRunning: vi.fn().mockResolvedValue(false),
  },
}));

vi.mock("@/services/customTunnelService", () => ({
  customTunnelService: {
    getCustomTunnels,
    isCustomTunnelRunning: vi.fn().mockResolvedValue(false),
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    onFocusChanged: vi.fn().mockResolvedValue(() => {}),
  }),
}));

const { useTunnelList } = await import("./useTunnelList");

const cachedTunnel = {
  type: "api",
  data: { id: 1, name: "缓存隧道" } as Tunnel,
} satisfies UnifiedTunnel;

const freshTunnel = {
  id: 2,
  name: "最新隧道",
} as Tunnel;

describe("useTunnelList cache-first loading", () => {
  beforeEach(() => {
    tunnelListCache.tunnels = [];
    tunnelListCache.hasLoaded = false;
    fetchTunnels.mockReset();
    getCustomTunnels.mockReset();
    getCustomTunnels.mockResolvedValue([]);
  });

  it("shows cached tunnels while refreshing in the background", async () => {
    tunnelListCache.tunnels = [cachedTunnel];
    tunnelListCache.hasLoaded = true;
    let resolveRefresh: ((value: typeof freshTunnel[]) => void) | undefined;
    fetchTunnels.mockReturnValue(
      new Promise<typeof freshTunnel[]>((resolve) => {
        resolveRefresh = resolve;
      }),
    );

    const { result } = renderHook(() => useTunnelList());

    expect(result.current.tunnels).toEqual([cachedTunnel]);
    expect(result.current.loading).toBe(true);

    await act(async () => {
      resolveRefresh?.([freshTunnel]);
    });

    await waitFor(() => {
      expect(result.current.loading).toBe(false);
      expect(result.current.tunnels).toEqual([
        { type: "api", data: freshTunnel },
      ]);
    });
  });

  it("remembers that an empty tunnel list has already loaded", async () => {
    fetchTunnels.mockResolvedValue([]);

    const firstRender = renderHook(() => useTunnelList());
    expect(firstRender.result.current.loading).toBe(true);
    await waitFor(() => expect(firstRender.result.current.loading).toBe(false));
    firstRender.unmount();

    let resolveRefresh: ((value: never[]) => void) | undefined;
    fetchTunnels.mockReturnValue(
      new Promise<never[]>((resolve) => {
        resolveRefresh = resolve;
      }),
    );
    const secondRender = renderHook(() => useTunnelList());

    expect(secondRender.result.current.tunnels).toEqual([]);
    expect(tunnelListCache.hasLoaded).toBe(true);
    resolveRefresh?.([]);
  });

  it("ends the initial loading state when the first request fails", async () => {
    fetchTunnels.mockRejectedValue(new Error("登录信息已过期"));

    const { result } = renderHook(() => useTunnelList());

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.initialLoading).toBe(false);
    expect(result.current.error).toBe("登录信息已过期");
  });
});
