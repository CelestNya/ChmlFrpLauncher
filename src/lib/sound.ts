export const playTunnelSound = (
  type: "success" | "error",
  enabled: boolean,
): void => {
  if (!enabled) return;

  try {
    // fnOS 网关前缀：浏览器环境下静态资源在 /app/chmlfrp/ 下，
    // 桌面版 location.pathname 无 /app/ 前缀，base 为空串，行为不变
    const base =
      typeof window !== "undefined"
        ? (location.pathname.match(/^(\/app\/[^/]+)/)?.[1] ?? "")
        : "";
    const audio = new Audio(
      `${base}${type === "success" ? "/run_tunnel.mp3" : "/stop_tunnel.mp3"}`,
    );
    audio.volume = 0.3;
    audio.play().catch((err) => {
      console.error("播放音效失败:", err);
    });
  } catch (err) {
    console.error("创建音效失败:", err);
  }
};
