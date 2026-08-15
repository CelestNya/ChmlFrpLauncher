import { invoke } from "@tauri-apps/api/core";
import { frpcManager, type LogMessage } from "./frpcManager";

type LogListener = (logs: LogMessage[]) => void;

/** 是否 fnOS 浏览器环境（shim 注入的标记；桌面版 Tauri 无此标记） */
const isFnos = (): boolean =>
  typeof window !== "undefined" &&
  (window as unknown as { __FNOS__?: boolean }).__FNOS__ === true;

class LogStore {
  private logs: LogMessage[] = [];
  private listeners: Set<LogListener> = new Set();
  private isListening = false;
  private maxLogs = 5000;
  /** fnOS：已收到的日志帧指纹集合，WS 重连全量补发时双向去重 */
  private replaySeen = new Set<string>();

  async startListening() {
    if (this.isListening) {
      return;
    }

    this.isListening = true;

    await frpcManager.listenToLogs((log: LogMessage) => {
      // fnOS：同一段日志以「实时帧」到达后，WS 重连会以「补发帧（replay）」再来一遍；
      // 订阅与快照之间的窗口内还可能出现「replay 先到、实时副本后到」的反序。
      // 双向都查指纹集合才能彻底去重（桌面版无补发，跳过整套逻辑保持上游行为）
      const key = `${log.tunnel_id}|${log.timestamp}|${log.message}`;
      if (isFnos()) {
        if (this.replaySeen.has(key)) {
          return;
        }
        this.replaySeen.add(key);
      }
      this.logs.push(log);
      this.trimLogs();
      this.notifyListeners();
    });
  }

  subscribe(listener: LogListener): () => void {
    this.listeners.add(listener);
    listener([...this.logs]);

    return () => {
      this.listeners.delete(listener);
    };
  }

  getLogs(): LogMessage[] {
    return [...this.logs];
  }

  /** 清空日志：fnOS 下同步清空 daemon 补发缓冲，返回是否成功（UI 据此提示）。 */
  clearLogs(): Promise<boolean> {
    this.logs = [];
    this.replaySeen.clear();
    this.notifyListeners();
    // fnOS：不清 daemon 缓冲的话，刷新页面后 WS 重连会补发旧日志「复活」
    //（桌面版无此命令，不调用）
    if (isFnos()) {
      return invoke("clear_log_history").then(
        () => true,
        () => false,
      );
    }
    return Promise.resolve(true);
  }

  addLog(log: LogMessage) {
    // fnOS：两个组件（useTunnelNotifications / useTunnelProgress）会重复生成同一
    // launcher 日志，尾部比对去重；桌面版无此场景，保持上游原行为（B8 门控）
    if (isFnos() && this.isDuplicate(log)) {
      return;
    }
    this.logs.push(log);
    this.trimLogs();
    this.notifyListeners();
  }

  /** 与尾部最近 100 条比对（补发帧必在近期；全量比对 5000 条成本高且无必要） */
  private isDuplicate(log: LogMessage): boolean {
    const start = Math.max(0, this.logs.length - 100);
    for (let i = this.logs.length - 1; i >= start; i--) {
      const prev = this.logs[i];
      if (
        prev.tunnel_id === log.tunnel_id &&
        prev.timestamp === log.timestamp &&
        prev.message === log.message
      ) {
        return true;
      }
    }
    return false;
  }

  private notifyListeners() {
    const logsCopy = [...this.logs];
    this.listeners.forEach((listener) => {
      listener(logsCopy);
    });
  }

  private trimLogs() {
    if (this.logs.length > this.maxLogs) {
      this.logs = this.logs.slice(-this.maxLogs);
    }
  }
}

// 导出单例
export const logStore = new LogStore();
