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
  // ---- 帧暂存（B2 修复）：daemon 连接建立即全量补发，React 挂载前无 listener 的
  // 帧会被丢弃——暂存后首次 listen 注册时以 replay:true 重放。上限 600（> daemon
  // 补发量 512，防丢帧）。 ----
  const MAX_PENDING_FRAMES = 600;
  let pendingFrames: Array<{ type: string; payload: unknown }> = [];
  // ---- updater 下载进度 Channel id（B6：download-progress 帧转发目标） ----
  let updateChannelId: number | null = null;
  // ---- B9：异步下载结果事件等待上限（daemon 下载最长约 120s/源，兜底 10 分钟） ----
  const DOWNLOAD_RESULT_TIMEOUT_MS = 10 * 60 * 1000;
  // ---- B8：下载阶段转发状态（Started 事件 + chunkLength 增量，见 ws.onmessage） ----
  let progressStarted = false;
  let progressLastDownloaded = 0;
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

  // 扩展名 → MIME（dialog.open 的 accept 属性；未知扩展回退 .ext 原样）
  function extToMime(ext: string): string {
    const normalized = ext.replace(/^\./, "").toLowerCase();
    switch (normalized) {
      case "png":
        return "image/png";
      case "jpg":
      case "jpeg":
        return "image/jpeg";
      case "gif":
        return "image/gif";
      case "bmp":
        return "image/bmp";
      case "svg":
        return "image/svg+xml";
      case "webp":
        return "image/webp";
      case "mp4":
        return "video/mp4";
      case "webm":
        return "video/webm";
      case "mp3":
        return "audio/mpeg";
      default:
        return `.${normalized}`;
    }
  }

  function connectEvents(): void {
    if (typeof WebSocket === "undefined") return;

    let ws: WebSocket | null = null;
    let retry = 0;
    // ---- B8：更新完成后 daemon 重启，WS 重连成功比对版本，变化则自动刷新 ----
    let knownVersion: string | null = null;

    const checkVersionOnOpen = () => {
      void fetch(`${gatewayPrefix()}/api/bootstrap`)
        .then((r) => r.json())
        .then((d: { version?: string }) => {
          const v = d.version || "";
          if (!v) return;
          if (knownVersion === null) {
            // 首次连接：记录基线（页面加载时 daemon 正常态）
            knownVersion = v;
            return;
          }
          if (v !== knownVersion) {
            // 更新已应用：提示后自动刷新（旧前端代码连的是旧 daemon 语义）
            const overlay = document.createElement("div");
            overlay.id = "fnos-update-reload-overlay";
            overlay.style.cssText =
              "position:fixed;inset:0;z-index:99999;background:rgba(10,10,14,.78);" +
              "color:#fff;display:flex;align-items:center;justify-content:center;" +
              "font:15px/1.6 system-ui,sans-serif;letter-spacing:.5px;";
            overlay.textContent = "更新完成，正在刷新…";
            document.body.appendChild(overlay);
            setTimeout(() => location.reload(), 400);
          }
        })
        .catch(() => {
          // daemon 重启窗口内 fetch 失败：忽略，等下一轮重连再比对
        });
    };

    const open = () => {
      if (ws) ws.close();
      const proto = location.protocol === "https:" ? "wss:" : "ws:";
      const wsUrl =
        (window as unknown as { __FNOS_WS_URL__?: string }).__FNOS_WS_URL__ ||
        `${proto}//${location.host}${gatewayPrefix()}/ws/logs`;
      ws = new WebSocket(wsUrl);

      ws.onopen = () => {
        retry = 0;
        checkVersionOnOpen();
      };

      ws.onmessage = (ev) => {
        let frame: { type?: string; payload?: unknown; replay?: boolean };
        try {
          frame = JSON.parse(ev.data as string);
        } catch {
          return;
        }
        if (!frame.type) return;
        // updater 下载进度：转发给 download_and_install 时登记的 Channel。
        // B8：daemon 载荷（{downloaded,total,percentage,stage}）→ 桌面插件形态——
        // 首次帧先发 Started（补 contentLength，前端据此才算得出百分比），
        // 后续 Progress 携带 chunkLength=本帧增量（前端 downloadedBytes += chunkLength）。
        // 此前直接透传 daemon 载荷：前端读 chunkLength 得 undefined → NaN → 进度恒 0。
        if (frame.type === "download-progress" && updateChannelId !== null) {
          const p = (frame.payload ?? {}) as {
            downloaded?: number;
            total?: number;
            percentage?: number;
            stage?: string;
          };
          const downloaded = p.downloaded ?? 0;
          const total = p.total ?? 0;
          if (!progressStarted) {
            progressStarted = true;
            progressLastDownloaded = 0;
            runCallback(updateChannelId, {
              event: "Started",
              data: {
                contentLength: total,
                stage: p.stage ?? "connecting",
              },
            });
          }
          const chunkLength = Math.max(downloaded - progressLastDownloaded, 0);
          progressLastDownloaded = downloaded;
          runCallback(updateChannelId, {
            event: "Progress",
            data: {
              chunkLength,
              contentLength: total,
              percentage: p.percentage ?? 0,
              stage: p.stage ?? "downloading",
            },
          });
        }
        const ids = eventListeners.get(frame.type);
        if (!ids) {
          // B2：暂无 listener（React 未挂载 / 页面刚加载）→ 环形暂存，
          // 首次 listen 注册时重放
          pendingFrames.push({ type: frame.type, payload: frame.payload });
          if (pendingFrames.length > MAX_PENDING_FRAMES) {
            pendingFrames.shift();
          }
          return;
        }
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
  // B4 修复：无超时（daemon 重启时前端永久卡 loading）+ 不检查 HTTP 状态
  const INVOKE_TIMEOUT_MS = 60_000;
  const LONG_INVOKE_COMMANDS = new Set(["download_frpc"]); // 下载类命令给更长时限

  function withTimeout<T>(
    promise: Promise<T>,
    ms: number,
    cmd: string,
  ): Promise<T> {
    return new Promise((resolve, reject) => {
      const timer = setTimeout(
        () => reject(new Error(`invoke ${cmd} 超时（${Math.round(ms / 1000)}s）`)),
        ms,
      );
      promise.then(
        (v) => {
          clearTimeout(timer);
          resolve(v);
        },
        (e) => {
          clearTimeout(timer);
          reject(e);
        },
      );
    });
  }

  async function invokeCommand(cmd: string, args: Record<string, unknown>) {
    const timeoutMs = LONG_INVOKE_COMMANDS.has(cmd) ? 300_000 : INVOKE_TIMEOUT_MS;
    const task = (async () => {
      const response = await fetch(`${gatewayPrefix()}/api/invoke`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ cmd, args }),
      });
      if (!response.ok) {
        throw new Error(`invoke ${cmd} 失败 (HTTP ${response.status})`);
      }
      let resp: {
        ok: boolean;
        data?: unknown;
        error?: string;
      };
      try {
        resp = (await response.json()) as typeof resp;
      } catch {
        throw new Error(`invoke ${cmd} 响应不是合法 JSON`);
      }
      if (resp.ok) return resp.data;
      throw new Error(resp.error || `invoke ${cmd} 失败`);
    })();
    return withTimeout(task, timeoutMs, cmd);
  }

  // ---- 插件命令降级 ----
  // B1/B3 修复：Tauri 插件命令的第三参数 options（headers 通道 / options 嵌套）
  // 此前被整体丢弃。invokePlugin 签名改为 (cmd, args, options)。
  function invokePlugin(
    cmd: string,
    args: Record<string, unknown>,
    options?: Record<string, unknown>,
  ) {
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
      // B2：重放注册前暂存的帧（带 replay:true，前端去重/跳过通知），随后清空
      const toReplay = pendingFrames.filter((f) => f.type === event);
      if (toReplay.length > 0) {
        for (const f of toReplay) {
          runCallback(handler, {
            event,
            id: Date.now(),
            replay: true,
            payload: f.payload,
          });
        }
        for (let i = pendingFrames.length - 1; i >= 0; i--) {
          if (pendingFrames[i]!.type === event) pendingFrames.splice(i, 1);
        }
      }
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
      // B5：同步清理 callbacks 表，否则闭包（捕获 React 状态）永久滞留
      unregisterCallback(eventId);
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
    // download_and_install → 异步下载（B9）：POST 立即返回，daemon 后台下载完成后经
    // download-result 事件推送结果，收到 ok 后再触发 apply（网关 504 实测修复——
    // fnOS 网关对应用请求有转发超时，同步等待下载必然超时）。
    if (plugin === "updater" && action === "check") {
      // B10：主动检查绕过 daemon 5 分钟缓存（?force=1）——缓存旧提示会误导
      // 「立即更新」（下载后才发现版本不一致），用户主动检查即拿最新状态。
      return fetch(`${gatewayPrefix()}/api/update/check?force=1`)
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
      // B6：登记进度 Channel id——daemon 下载时广播 download-progress 帧，
      // onmessage 转发给该 Channel（进度条不再恒 0）
      updateChannelId = (args.onEvent as number | null) ?? null;
      // B8：每次下载从零开始累计增量（多条帧间增量，防进度回跳）
      progressStarted = false;
      progressLastDownloaded = 0;
      // B9：先注册 download-result 监听（先于 POST，杜绝结果帧先到的竞态），
      // 清理上次残留的监听与暂存帧
      eventListeners.delete("download-result");
      pendingFrames = pendingFrames.filter((f) => f.type !== "download-result");
      const resultPromise = new Promise<null>((resolve, reject) => {
        const timer = setTimeout(
          () => reject(new Error("下载更新超时（daemon 无响应），请重试")),
          DOWNLOAD_RESULT_TIMEOUT_MS,
        );
        const resultId = transformCallback(
          (data: unknown) => {
            clearTimeout(timer);
            eventListeners.delete("download-result");
            const payload = (
              (data as { payload?: { ok?: boolean; error?: string } })?.payload ?? {}
            ) as { ok?: boolean; error?: string };
            if (payload.ok) {
              fetch(`${gatewayPrefix()}/api/update/apply`, { method: "POST" })
                .then((r) => r.json())
                .then((resp: { ok?: boolean; error?: string }) => {
                  if (!resp.ok) throw new Error(resp.error || "应用更新失败");
                  resolve(null);
                })
                .catch(reject);
            } else {
              reject(new Error(payload.error || "下载更新失败"));
            }
          },
          true,
        );
        eventListeners.set("download-result", new Set([resultId]));
      });
      return fetch(`${gatewayPrefix()}/api/update/download`, { method: "POST" })
        .then((r) => r.json())
        .then((resp: { ok?: boolean; error?: string }) => {
          if (!resp.ok) {
            eventListeners.delete("download-result");
            throw new Error(resp.error || "下载更新失败");
          }
          return resultPromise;
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
      // B1 修复：plugin-fs@2.5 真实形态 = invoke(cmd, Uint8Array(内容),
      // {headers:{path: encodeURIComponent(...), options}})；shim 曾把二进制当
      // args、丢弃第三参 → path 落空、内容为空（导出日志下载空文件）
      const rawPath = (
        options?.headers as { path?: string } | undefined
      )?.path;
      const filePath = rawPath
        ? decodeURIComponent(rawPath)
        : ((args as { path?: string }).path || "chmlfrp.log");
      let contents: string;
      // ArrayBuffer.isView 而非 instanceof Uint8Array：跨 realm 环境（测试 jsdom
      // 的 TextEncoder 来自 Node realm）下 instanceof 恒 false
      if (ArrayBuffer.isView(args)) {
        contents = new TextDecoder().decode(args as Uint8Array);
      } else {
        contents = String(
          (args as { contents?: unknown; data?: unknown }).contents ??
            (args as { data?: unknown }).data ??
            "",
        );
      }
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
      // ADR-0004（阶段 2b）：选图后把 dataURL 上传 daemon（save_background_image），
      // 返回托管相对路径（backgrounds/<file>）。前端 getBackgroundType 识别
      // 非 data:/app:// 前缀直接当 src，渲染链路经 daemon 静态托管 + 浏览器缓存。
      // 限制 2MB：dataURL base64 膨胀 ~33%；原 dataURL 不再存 localStorage。
      const opts = (args.options ?? {}) as {
        directory?: boolean;
        filters?: Array<{ name?: string; extensions?: string[] }>;
      };
      if (opts.directory === true) {
        // 文件夹选择（轮播源）fnOS 已砍（ADR-0004），保持取消语义
        return Promise.resolve(null);
      }
      const extensions = opts.filters?.[0]?.extensions ?? [];
      const accept = extensions.length > 0 ? extensions.map(extToMime).join(",") : "image/*";
      return new Promise((resolve) => {
        const input = document.createElement("input");
        input.type = "file";
        input.accept = accept;
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
          reader.onload = async () => {
            const dataUrl = (reader.result as string) || "";
            if (!dataUrl) {
              finish(null);
              return;
            }
            try {
              // 上传 daemon：dataURL → backgrounds/<file>，返回托管相对路径
              const rel = (await invokeCommand("save_background_image", {
                dataUrl,
                fileName: file.name || `bg-${Date.now()}.png`,
              })) as string;
              // 用 new URL(rel, location.href) 解析出含网关前缀的绝对 URL：
              // 页面 URL 是 /app/chmlfrp/...，相对 assets/... 解析到
              // /app/chmlfrp/assets/backgrounds/<file>（走网关 → daemon 托管）。
              // ⚠️ 不能直接存相对路径——localStorage 渲染类留守后刷新/路由变化
              // 相对解析可能错；绝对 URL 含 /app/chmlfrp/ 前缀稳定可渲染。review P3。
              finish(rel.startsWith("backgrounds/")
                ? new URL(`assets/${rel}`, location.href).href
                : rel);
            } catch {
              // daemon 不可达：退回 dataURL（前端仍可渲染，仅不持久化）
              finish(dataUrl);
            }
          };
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
      // B3 修复：读 options.defaultPath（此前读 args.defaultPath 恒 undefined）
      const opts = (args.options ?? {}) as { defaultPath?: string };
      const defaultPath =
        opts.defaultPath || `chmlfrp-${Date.now()}.log`;
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
      options?: Record<string, unknown>,
    ) => {
      if (cmd.startsWith("plugin:")) return invokePlugin(cmd, args, options);
      return invokeCommand(cmd, args);
    },
    transformCallback,
    unregisterCallback,
    // Tauri 原签名 (filePath, protocol)：protocol 在 fnOS 网关下无意义，多余实参 JS 自动忽略
    convertFileSrc: (filePath: string) => filePath,
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { label: "main" },
    },
  };

  // ---- 阶段 1c：localStorage 白名单 key 重定向到 daemon（ADR-0002/0003） ----
  // token/明文代理密码物理离开浏览器 localStorage：替换 window.localStorage 为
  // 代理，白名单 key 读写转发 daemon（credential.json / node_settings.json，0600）。
  // 首帧同步读：daemon 托管 index.html 时注入 __FNOS_BOOT__（credential+nodeSettings），
  // shim 启动读入内存缓存，业务代码同步 getItem 命中缓存，不闪烁。
  // 非白名单 key（主题/音效等渲染类偏好）透传真实 localStorage（ADR-0003）。
  const realLocalStorage = window.localStorage;

  type JsonObject = Record<string, unknown>;

  // 内存缓存（boot 注水 + 本地 setItem 维护；daemon 是最终权威）
  let credentialCache: JsonObject | null = null;
  let nodeSettingsCache: JsonObject | null = null;

  const bootData = (window as unknown as { __FNOS_BOOT__?: JsonObject })
    .__FNOS_BOOT__;
  if (bootData && typeof bootData === "object") {
    credentialCache =
      (bootData.credential as JsonObject | null | undefined) ?? null;
    nodeSettingsCache =
      (bootData.nodeSettings as JsonObject | null | undefined) ?? null;
  }

  const CREDENTIAL_KEY = "chmlfrp_user";
  const PROXY_CONFIG_KEY = "frpc_proxy_config";
  const LOG_LEVEL_KEY = "frpcLogLevel";
  const BYPASS_PROXY_KEY = "bypassProxy";
  const RESTART_ON_EDIT_KEY = "restartOnEdit";
  const WHITELIST = new Set([
    CREDENTIAL_KEY,
    PROXY_CONFIG_KEY,
    LOG_LEVEL_KEY,
    BYPASS_PROXY_KEY,
    RESTART_ON_EDIT_KEY,
  ]);

  // 凭据判定：任一 token 存在即已登录（与 daemon Credential::is_logged_in 一致）
  function credentialLoggedIn(c: JsonObject): boolean {
    return (
      c.usertoken !== undefined ||
      c.access_token !== undefined ||
      c.refresh_token !== undefined
    );
  }

  // daemon snake_case ↔ 前端 camelCase（StoredUser 原始格式）
  // ⚠️ 必须完整往返：丢 usergroup 会破坏会员门控（NodeSelector/EditTunnelDialog
  // 用 user?.usergroup 决定免费/会员，免费用户可绕过 VIP 节点拦截）。review P2。
  function credentialToStoredUser(c: JsonObject): JsonObject {
    const out: JsonObject = {};
    if (c.username !== undefined) out.username = c.username;
    if (c.usertoken !== undefined) out.usertoken = c.usertoken;
    if (c.access_token !== undefined) out.accessToken = c.access_token;
    if (c.refresh_token !== undefined) out.refreshToken = c.refresh_token;
    if (c.access_token_expires_at !== undefined) {
      out.accessTokenExpiresAt = c.access_token_expires_at;
    }
    if (c.token_type !== undefined) out.tokenType = c.token_type;
    if (c.usergroup !== undefined) out.usergroup = c.usergroup;
    if (c.userimg !== undefined) out.userimg = c.userimg;
    if (c.tunnel_count !== undefined) out.tunnelCount = c.tunnel_count;
    if (c.tunnel !== undefined) out.tunnel = c.tunnel;
    return out;
  }

  function storedUserToCredential(s: JsonObject): JsonObject {
    const out: JsonObject = {};
    if (s.username !== undefined) out.username = s.username;
    if (s.usertoken !== undefined) out.usertoken = s.usertoken;
    if (s.accessToken !== undefined) out.access_token = s.accessToken;
    if (s.refreshToken !== undefined) out.refresh_token = s.refreshToken;
    if (s.accessTokenExpiresAt !== undefined) {
      out.access_token_expires_at = s.accessTokenExpiresAt;
    }
    if (s.tokenType !== undefined) out.token_type = s.tokenType;
    if (s.usergroup !== undefined) out.usergroup = s.usergroup;
    if (s.userimg !== undefined) out.userimg = s.userimg;
    if (s.tunnelCount !== undefined) out.tunnel_count = s.tunnelCount;
    if (s.tunnel !== undefined) out.tunnel = s.tunnel;
    return out;
  }

  // NodeSettings proxy 段 ↔ 前端 proxy config 原始格式
  function nodeSettingsToProxyConfig(n: JsonObject): JsonObject {
    const out: JsonObject = {};
    if (n.proxy_enabled !== undefined) out.enabled = n.proxy_enabled;
    if (n.proxy_type !== undefined) out.type = n.proxy_type;
    if (n.proxy_host !== undefined) out.host = n.proxy_host;
    if (n.proxy_port !== undefined) out.port = n.proxy_port;
    if (n.proxy_username !== undefined) out.username = n.proxy_username;
    if (n.proxy_password !== undefined) out.password = n.proxy_password;
    if (n.proxy_force_tls !== undefined) out.forceTls = n.proxy_force_tls;
    if (n.proxy_kcp_optimization !== undefined) {
      out.kcpOptimization = n.proxy_kcp_optimization;
    }
    return out;
  }

  function proxyConfigToNodeSettings(p: JsonObject): JsonObject {
    const out: JsonObject = {};
    if (p.enabled !== undefined) out.proxy_enabled = p.enabled;
    if (p.type !== undefined) out.proxy_type = p.type;
    if (p.host !== undefined) out.proxy_host = p.host;
    if (p.port !== undefined) out.proxy_port = p.port;
    if (p.username !== undefined) out.proxy_username = p.username;
    if (p.password !== undefined) out.proxy_password = p.password;
    if (p.forceTls !== undefined) out.proxy_force_tls = p.forceTls;
    if (p.kcpOptimization !== undefined) {
      out.proxy_kcp_optimization = p.kcpOptimization;
    }
    return out;
  }

  // 异步转发（fire-and-forget：daemon 不可达时静默，本地缓存仍可用）
  function saveCredentialRemote(credential: JsonObject): void {
    invokeCommand("save_credential", { credential }).catch(() => {});
  }

  function saveNodeSettingsRemote(): void {
    if (nodeSettingsCache === null) return;
    invokeCommand("set_node_settings", { settings: nodeSettingsCache }).catch(
      () => {},
    );
  }

  function updateNodeSettings(patch: JsonObject): void {
    nodeSettingsCache = { ...(nodeSettingsCache ?? {}), ...patch };
    saveNodeSettingsRemote();
  }

  function storageGetItem(key: string): string | null {
    if (!WHITELIST.has(key)) {
      return realLocalStorage.getItem(key);
    }
    if (key === CREDENTIAL_KEY) {
      if (credentialCache && credentialLoggedIn(credentialCache)) {
        return JSON.stringify(credentialToStoredUser(credentialCache));
      }
      return null;
    }
    if (key === PROXY_CONFIG_KEY) {
      if (
        nodeSettingsCache &&
        nodeSettingsCache.proxy_enabled !== undefined
      ) {
        return JSON.stringify(nodeSettingsToProxyConfig(nodeSettingsCache));
      }
      return null;
    }
    if (key === LOG_LEVEL_KEY) {
      return nodeSettingsCache && nodeSettingsCache.log_level !== undefined
        ? String(nodeSettingsCache.log_level)
        : null;
    }
    if (key === BYPASS_PROXY_KEY) {
      return nodeSettingsCache && nodeSettingsCache.bypass_proxy !== undefined
        ? String(nodeSettingsCache.bypass_proxy)
        : null;
    }
    if (key === RESTART_ON_EDIT_KEY) {
      return nodeSettingsCache && nodeSettingsCache.restart_on_edit !== undefined
        ? String(nodeSettingsCache.restart_on_edit)
        : null;
    }
    return null; // WHITELIST 内但未命中具体 key（理论不可达）
  }

  function storageSetItem(key: string, value: string): void {
    if (!WHITELIST.has(key)) {
      realLocalStorage.setItem(key, value);
      return;
    }
    if (key === CREDENTIAL_KEY) {
      let parsed: JsonObject;
      try {
        parsed = JSON.parse(value) as JsonObject;
      } catch {
        return;
      }
      credentialCache = storedUserToCredential(parsed);
      saveCredentialRemote(credentialCache);
      return;
    }
    if (key === PROXY_CONFIG_KEY) {
      let parsed: JsonObject;
      try {
        parsed = JSON.parse(value) as JsonObject;
      } catch {
        return;
      }
      updateNodeSettings(proxyConfigToNodeSettings(parsed));
      return;
    }
    if (key === LOG_LEVEL_KEY) {
      updateNodeSettings({ log_level: value });
      return;
    }
    if (key === BYPASS_PROXY_KEY) {
      updateNodeSettings({ bypass_proxy: value === "true" });
      return;
    }
    if (key === RESTART_ON_EDIT_KEY) {
      updateNodeSettings({ restart_on_edit: value === "true" });
      return;
    }
    return; // WHITELIST 内但未命中具体 key（理论不可达）
  }

  function storageRemoveItem(key: string): void {
    if (!WHITELIST.has(key)) {
      realLocalStorage.removeItem(key);
      return;
    }
    if (key === CREDENTIAL_KEY) {
      credentialCache = null;
      invokeCommand("clear_credential", {}).catch(() => {});
      return;
    }
    if (key === PROXY_CONFIG_KEY) {
      updateNodeSettings({
        proxy_enabled: undefined,
        proxy_type: undefined,
        proxy_host: undefined,
        proxy_port: undefined,
        proxy_username: undefined,
        proxy_password: undefined,
        proxy_force_tls: undefined,
        proxy_kcp_optimization: undefined,
      });
      return;
    }
    if (key === LOG_LEVEL_KEY) {
      updateNodeSettings({ log_level: undefined });
      return;
    }
    if (key === BYPASS_PROXY_KEY) {
      updateNodeSettings({ bypass_proxy: undefined });
      return;
    }
    if (key === RESTART_ON_EDIT_KEY) {
      updateNodeSettings({ restart_on_edit: undefined });
      return;
    }
    return; // WHITELIST 内但未命中具体 key（理论不可达）
  }

  function storageClear(): void {
    // 白名单 key 不在真实存储（本就转发 daemon）；清真实存储的渲染类偏好
    realLocalStorage.clear();
    credentialCache = null;
    nodeSettingsCache = null;
  }

  const storageProxy: Storage = {
    getItem: storageGetItem,
    setItem: storageSetItem,
    removeItem: storageRemoveItem,
    clear: storageClear,
    key: (index: number) => realLocalStorage.key(index),
    get length(): number {
      return realLocalStorage.length;
    },
  };

  try {
    Object.defineProperty(window, "localStorage", {
      value: storageProxy,
      configurable: true,
    });
  } catch {
    // 环境不允许替换（极端情况）时放弃拦截，退化为原 localStorage
  }

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
