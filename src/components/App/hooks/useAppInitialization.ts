import { useEffect } from "react";
import { logStore } from "@/services/logStore";

/**
 * 应用初始化逻辑 hook
 * 处理日志监听、进程守护等初始化工作
 */
export function useAppInitialization() {
  useEffect(() => {
    logStore.startListening();

    const initProcessGuard = async () => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        // fnOS 适配：守护是核心安全特性（进程意外退出自动拉起），默认开启。
        // 桌面版默认关闭（用户手动开），fnOS 环境首次使用（localStorage 无记录）时
        // 默认 true；用户手动切换后尊重其选择。
        const isFnos =
          typeof window !== "undefined" &&
          (window as unknown as { __FNOS__?: boolean }).__FNOS__ === true;
        const stored = localStorage.getItem("processGuardEnabled");
        const guardEnabled =
          stored === "true" || (isFnos && stored === null);
        await invoke("set_process_guard_enabled", { enabled: guardEnabled });
      } catch (error) {
        console.error("Failed to initialize process guard:", error);
      }
    };

    initProcessGuard();
  }, []);

  useEffect(() => {
    const handleContextMenu = (e: MouseEvent) => {
      e.preventDefault();
      return false;
    };

    document.addEventListener("contextmenu", handleContextMenu);

    return () => {
      document.removeEventListener("contextmenu", handleContextMenu);
    };
  }, []);
}
