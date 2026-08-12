// fnos-shim/tauri-shim.ts
// fnOS 版 Tauri 2 运行时模拟层（shim）。
//
// 形态 C：本文件经 esbuild 编译为 IIFE 注入 dist/index.html，
// 模拟 window.__TAURI_INTERNALS__，使未修改的前端（@tauri-apps/api v2）
// 在浏览器 / fnOS iframe 中可运行，后端语义由 fnos-daemon 提供。
//
// 契约（与 Tauri 2 / daemon 对齐）：
// - invoke("cmd", args) → POST /api/invoke {cmd,args} → {ok,data,error}，解包为 Promise
// - listen("event", cb) → WebSocket /ws/logs，按事件名分发（frpc-log / download-progress / tunnel-auto-restarted）
// - 插件命令（app/window/dialog/fs/opener/updater/event）按 fnOS 形态降级
//
// 覆盖面以 src/ 实际 import 为准：core / event / window / app / plugin-dialog /
// plugin-opener / plugin-fs / plugin-updater。新增 Tauri 调用需同步补本 shim。
//
// 注意：本文件不得包含顶层 import/export（esbuild bundle 后注入）。

(() => {
  "use strict";

  type Callback = (data: unknown) => void;

  // ---- 回调表：transformCallback 注册的 JS 回调 ----
  const callbacks = new Map<number, Callback>();
  // ---- 事件监听：事件名 → callback id 集合（WS 分发目标）----
  const eventListeners = new Map<string, Set<number>>();
  let nextId = 1;

  function transformCallback(callback: Callback, once = false): number {
    const id = nextId++;
    callbacks.set(id, (data: unknown) => {
      if (once) callbacks.delete(id);
      callback(data);
    });
    return id;
  }

  function unregisterCallback(id: number): void {
    callbacks.delete(id);
  }

  function runCallback(id: number, data: unknown): void {
    const cb = callbacks.get(id);
    if (cb) cb(data);
  }

  // ---- WebSocket 事件桥接 ----
  // SPA 由 daemon 静态托管（fnOS 网关 /app/chmlfrp 前缀），WS 与页面同源；断线指数退避重连。
  // 网关前缀从 location.pathname 推导（fnOS 下 /app/chmlfrp/...，本地开发 /），
  // 使 API/WS 路径在两种访问方式下都正确。
  function gatewayPrefix(): string {
    const m = location.pathname.match(/^(\/app\/[^/]+)/);
    return m ? m[1] : "";
  }

  function connectEvents(): void {
    if (typeof WebSocket === "undefined") return;

    let ws: WebSocket | null = null;
    let retry = 0;

    const open = () => {
      if (ws) ws.close();
      const proto = location.protocol === "https:" ? "wss:" : "ws:";
      const wsUrl =
        (window as unknown as { __FNOS_WS_URL__?: string }).__FNOS_WS_URL__ ||
        `${proto}//${location.host}${gatewayPrefix()}/ws/logs`;
      ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        retry = 0;
      };

      ws.onmessage = (ev) => {
        let frame: { type?: string; payload?: unknown; replay?: boolean };
        try {
          frame = JSON.parse(ev.data as string);
        } catch {
          return;
        }
        if (!frame.type) return;
        const ids = eventListeners.get(frame.type);
        if (!ids) return;
        const eventData = {
          event: frame.type,
          id: Date.now(),
          // daemon 补发帧带 replay 标记：前端通知逻辑据此跳过（日志照常显示）
          replay: frame.replay === true,
          payload: frame.payload,
        };
        ids.forEach((id) => runCallback(id, eventData));
      };

      ws.onclose = () => {
        const delay = Math.min(1000 * 2 ** retry, 15000);
        retry += 1;
        setTimeout(open, delay);
      };
      ws.onerror = () => ws?.close();
    };

    open();
  }

  // ---- 常规命令：POST /api/invoke 透传 ----
  async function invokeCommand(cmd: string, args: Record<string, unknown>) {
    const response = await fetch(`${gatewayPrefix()}/api/invoke`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ cmd, args }),
    });
    const resp = (await response.json()) as {
      ok: boolean;
      data?: unknown;
      error?: string;
    };
    if (resp.ok) return resp.data;
    throw new Error(resp.error || `invoke ${cmd} 失败`);
  }

  // ---- 插件命令降级 ----
  function invokePlugin(cmd: string, args: Record<string, unknown>) {
    // 形如 plugin:event|listen
    const match = /^plugin:([^|]+)\|(.+)$/.exec(cmd);
    const plugin = match?.[1];
    const action = match?.[2];

    // 事件监听（@tauri-apps/api/event 的 listen/once）
    if (plugin === "event" && action === "listen") {
      const event = args.event as string;
      // ⚠️ 必须用 args.handler（transformCallback 注册到 callbacks 表的 ID）作为
      // 事件监听 ID：WS 帧到达时 runCallback(该 ID) 才能查到回调。曾误用自增
      // eventId 导致帧被静默丢弃（浏览器收到帧但前端无日志）。
      const handler = args.handler as number;
      if (!eventListeners.has(event)) eventListeners.set(event, new Set());
      eventListeners.get(event)!.add(handler);
      return Promise.resolve(handler);
    }

    if (plugin === "event" && action === "unlisten") {
      const event = args.event as string;
      const eventId = args.eventId as number;
      const set = eventListeners.get(event);
      if (set) {
        set.delete(eventId);
        if (set.size === 0) eventListeners.delete(event);
      }
      return Promise.resolve();
    }

    // 插件 Channel 注册（updater 下载进度等），返回 channelId 即可
    if (
      plugin === "event" &&
      (action === "register_listener" || action === "registerListener")
    ) {
      return Promise.resolve(nextId++);
    }
    if (plugin === "event" && action === "remove_listener") {
      return Promise.resolve();
    }

    // 事件广播：fnOS 下事件由 daemon 直接推，前端 emit 无意义
    if (plugin === "event" && (action === "emit" || action === "emit_to")) {
      return Promise.resolve();
    }

    // 窗口 listen 型（on_resized / on_moved / listen）：返回 eventId 即可，
    // 前端会把它包成 unlisten 函数（随后再次 invoke 的 unlisten 调用走下方兜底）
    if (
      plugin === "window" &&
      (action === "listen" || action === "on_resized" || action === "on_moved")
    ) {
      return Promise.resolve(nextId++);
    }

    // 窗口状态查询：布尔一律 false，尺寸/位置给合理默认
    if (plugin === "window") {
      if (
        action === "is_maximized" ||
        action === "is_minimized" ||
        action === "is_focused" ||
        action === "is_visible" ||
        action === "is_fullscreen" ||
        action === "is_decorated" ||
        action === "is_resizable" ||
        action === "is_closable" ||
        action === "is_enabled" ||
        action === "is_always_on_top" ||
        action === "is_always_on_bottom"
      ) {
        return Promise.resolve(false);
      }
      if (action === "scale_factor") return Promise.resolve(1);
      if (action === "inner_size" || action === "outer_size") {
        return Promise.resolve({ width: 1280, height: 800 });
      }
      if (
        action === "inner_position" ||
        action === "outer_position" ||
        action === "cursor_position"
      ) {
        return Promise.resolve({ x: 0, y: 0 });
      }
      // 其余窗口动作（minimize/maximize/close/hide/show/set_focus/start_dragging/set_title…）no-op
      return Promise.resolve();
    }

    // 应用信息
    if (plugin === "app" && action === "version") {
      return fetch(`${gatewayPrefix()}/api/bootstrap`)
        .then((r) => r.json())
        .then((d: { version?: string }) => d.version || "0.0.0")
        .catch(() => "0.0.0");
    }
    if (plugin === "app" && (action === "name" || action === "get_version")) {
      return invokePlugin("plugin:app|version", args);
    }

    // 更新器：fnOS 自更新由 daemon 承载（B5）。
    // check → daemon /api/update/check（返回 {available, version, url, size}，无更新返回 null）；
    // download_and_install → 依次触发 daemon 下载与应用（apply 后服务重启，前端提示"重启后生效"）。
    if (plugin === "updater" && action === "check") {
      return fetch(`${gatewayPrefix()}/api/update/check`)
        .then((r) => r.json())
        .then((resp: { ok?: boolean; data?: { available?: boolean } }) => {
          if (resp.ok && resp.data?.available) return resp.data;
          return null;
        })
        .catch(() => null);
    }
    if (
      plugin === "updater" &&
      (action === "download_and_install" || action === "downloadAndInstall")
    ) {
      return fetch(`${gatewayPrefix()}/api/update/download`, { method: "POST" })
        .then((r) => r.json())
        .then((resp: { ok?: boolean; error?: string }) => {
          if (!resp.ok) throw new Error(resp.error || "下载更新失败");
          return fetch(`${gatewayPrefix()}/api/update/apply`, { method: "POST" }).then((r) =>
            r.json(),
          );
        })
        .then((resp: { ok?: boolean; error?: string }) => {
          if (!resp.ok) throw new Error(resp.error || "应用更新失败");
          return null;
        });
    }
    if (plugin === "updater") {
      return Promise.resolve();
    }

    // 打开外部 URL：浏览器新窗口（fnOS 网关内由浏览器处理）
    if (plugin === "opener" && action === "open_url") {
      const url = args.url as string;
      if (url) window.open(url, "_blank", "noopener");
      return Promise.resolve();
    }

    // 文件写入：浏览器无真实文件系统，降级为下载
    if (plugin === "fs" && action === "write_text_file") {
      const filePath = (args.path as string) || "chmlfrp.log";
      const contents = (args.contents ?? args.data ?? "") as string;
      const blob = new Blob([contents], { type: "text/plain" });
      const objectUrl = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = objectUrl;
      a.download = filePath.split(/[\\/]/).pop() || "chmlfrp.log";
      a.click();
      URL.revokeObjectURL(objectUrl);
      return Promise.resolve();
    }

    // 文件对话框
    if (plugin === "dialog" && action === "open") {
      // fnOS 浏览器环境：用浏览器原生文件选框替代 tauri 对话框。
      // 返回 dataURL 字符串（前端 getBackgroundType 原生识别 data:image/ 前缀，
      // 渲染链路无前缀时直接当 src 使用），前端 useBackgroundImage 有 __FNOS__
      // 分支跳过 daemon 拷贝。
      // 限制 2MB：dataURL base64 膨胀 ~33%，且存 localStorage（Chrome 每项 5MB）。
      if (args.directory === true) {
        // 文件夹选择暂不支持（浏览器无法返回目录路径），保持取消语义
        return Promise.resolve(null);
      }
      return new Promise((resolve) => {
        const input = document.createElement("input");
        input.type = "file";
        input.accept = "image/*";
        // settled 保证只 resolve 一次（onchange / cancel / focus 兜底竞争）
        let settled = false;
        const finish = (value: string | null) => {
          if (settled) return;
          settled = true;
          resolve(value);
        };
        input.onchange = () => {
          const file = input.files?.[0];
          if (!file) {
            finish(null);
            return;
          }
          if (file.size > 2 * 1024 * 1024) {
            window.alert("图片过大（>2MB），请选择更小的图片");
            finish(null);
            return;
          }
          const reader = new FileReader();
          reader.onload = () => finish((reader.result as string) || null);
          reader.onerror = () => finish(null);
          reader.readAsDataURL(file);
        };
        // 用户取消（Esc / 点击关闭文件框）：Chromium 113+ 触发 cancel 事件。
        // ⚠️ 不要用 window focus 做兜底：文件框关闭时 focus 先于 change 触发，
        // 会把正常选中的图片也 cancel 掉（实测翻车）。
        input.addEventListener("cancel", () => finish(null));
        input.click();
      });
    }
    if (plugin === "dialog" && action === "save") {
      // 返回占位路径，write_text_file 降级为下载
      const defaultPath =
        (args.defaultPath as string) || `chmlfrp-${Date.now()}.log`;
      return Promise.resolve(defaultPath);
    }
    if (
      plugin === "dialog" &&
      (action === "message" || action === "ask" || action === "confirm")
    ) {
      const msg = (args.message as string) || "";
      const kind = args.kind;
      if (action === "message" || kind === "warning" || kind === "error") {
        window.alert(msg);
      } else {
        return Promise.resolve(window.confirm(msg));
      }
      return Promise.resolve();
    }

    // 未识别的插件命令：静默降级（避免前端因未处理而白屏）
    return Promise.resolve();
  }

  // ---- 对外暴露 __TAURI_INTERNALS__ ----
  const TAURI_INTERNALS = {
    invoke: (
      cmd: string,
      args: Record<string, unknown> = {},
      _options?: unknown,
    ) => {
      if (cmd.startsWith("plugin:")) return invokePlugin(cmd, args);
      return invokeCommand(cmd, args);
    },
    transformCallback,
    unregisterCallback,
    convertFileSrc: (filePath: string, _protocol = "asset") => filePath,
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main" },
    },
  };

  const w = window as unknown as Record<string, unknown>;
  w.__TAURI_INTERNALS__ = TAURI_INTERNALS;
  w.__TAURI__ = true;
  // fnOS 专属标记：桌面版 Tauri 也有 __TAURI__/__TAURI_INTERNALS__，
  // 前端需要区分 fnOS 浏览器环境时用本标记（如背景图 dataURL 分支）
  w.__FNOS__ = true;
  w.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener: (event: string, eventId: number) => {
      const set = eventListeners.get(event);
      if (set) {
        set.delete(eventId);
        if (set.size === 0) eventListeners.delete(event);
      }
    },
  };

  connectEvents();
})();
