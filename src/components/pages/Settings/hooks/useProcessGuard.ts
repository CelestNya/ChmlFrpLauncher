import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

export function useProcessGuard() {
  const [guardEnabled, setGuardEnabled] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    // fnOS 适配：守护默认开启（与 useAppInitialization 一致）；桌面版默认关闭
    const isFnos =
      (window as unknown as { __FNOS__?: boolean }).__FNOS__ === true;
    const stored = localStorage.getItem("processGuardEnabled");
    return stored === "true" || (isFnos && stored === null);
  });

  const [guardLoading, setGuardLoading] = useState(false);

  useEffect(() => {
    // 同步状态到后端
    const syncGuardState = async () => {
      try {
        await invoke("set_process_guard_enabled", {
          enabled: guardEnabled,
        });
      } catch (error) {
        console.error("Failed to sync process guard state:", error);
      }
    };

    syncGuardState();
  }, [guardEnabled]);

  const handleToggleGuard = async (enabled: boolean) => {
    setGuardLoading(true);
    try {
      await invoke("set_process_guard_enabled", { enabled });
      setGuardEnabled(enabled);
      localStorage.setItem("processGuardEnabled", enabled.toString());
      toast.success(enabled ? "守护进程已启用" : "守护进程已禁用");
    } catch (error) {
      toast.error(`设置守护进程失败: ${error}`);
      console.error("Failed to toggle process guard:", error);
    } finally {
      setGuardLoading(false);
    }
  };

  return {
    guardEnabled,
    guardLoading,
    handleToggleGuard,
  };
}
