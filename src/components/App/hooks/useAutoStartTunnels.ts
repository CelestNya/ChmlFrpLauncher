import { useEffect, useRef } from "react";
import { fetchTunnels, type StoredUser, type Tunnel } from "@/services/api";
import { frpcManager } from "@/services/frpcManager";
import { customTunnelService } from "@/services/customTunnelService";
import { autoStartTunnelsService } from "@/services/autoStartTunnelsService";

export function useAutoStartTunnels(user: StoredUser | null) {
  const hasRunRef = useRef(false);

  useEffect(() => {
    if (hasRunRef.current) return;
    if (!user?.usertoken) return;

    hasRunRef.current = true;

    const run = async () => {
      try {
        const [apiTunnels, customTunnels] = await Promise.all([
          fetchTunnels().catch(() => [] as Tunnel[]),
          customTunnelService.getCustomTunnels().catch(() => []),
        ]);

        await Promise.all(
          apiTunnels.map(async (t) => {
            try {
              const enabled = await autoStartTunnelsService.isTunnelEnabled(
                "api",
                t.id,
              );
              if (!enabled) return;
              if (t.nodestate && t.nodestate !== "online") return;
              const running = await frpcManager
                .isTunnelRunning(t.id)
                .catch(() => false);
              if (running) return;
              await frpcManager.startTunnel(t, user.usertoken!);
            } catch (error) {
              console.error(`自动启动隧道失败 (api ${t.id}):`, error);
            }
          }),
        );

        await Promise.all(
          customTunnels.map(async (t) => {
            try {
              const enabled = await autoStartTunnelsService.isTunnelEnabled(
                "custom",
                t.id,
              );
              if (!enabled) return;
              const running = await customTunnelService
                .isCustomTunnelRunning(t.id)
                .catch(() => false);
              if (running) return;
              await customTunnelService.startCustomTunnel(t.id);
            } catch (error) {
              console.error(`自动启动自定义隧道失败 (${t.id}):`, error);
            }
          }),
        );
      } catch (error) {
        console.error("自动启动隧道失败:", error);
      }
    };

    void run();
  }, [user?.usertoken]);
}
