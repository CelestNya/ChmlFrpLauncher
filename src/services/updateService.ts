import { check, type DownloadEvent } from "@tauri-apps/plugin-updater";
import { getVersion } from "@tauri-apps/api/app";
import { flushSync } from "react-dom";

export interface UpdateInfo {
  version: string;
  date?: string;
  body?: string;
}

export class UpdateService {
  /**
   * 检查是否有可用更新
   * @returns 如果有更新返回 Update 对象，否则返回 null
   */
  async checkUpdate(): Promise<{
    available: boolean;
    version?: string;
    date?: string;
    body?: string;
  }> {
    try {
      const update = await check({
        headers: {},
      });

      if (update?.available) {
        return {
          available: true,
          version: update.version,
          date: update.date,
          body: update.body,
        };
      }

      return { available: false };
    } catch (error) {
      console.error("检查更新失败:", error);
      const errorMsg = error instanceof Error ? error.message : String(error);
      throw new Error(`检查更新失败: ${errorMsg}`);
    }
  }

  /**
   * 安装更新
   * @param onProgress 下载进度回调函数（fnOS 额外携带阶段 stage）
   */
  async installUpdate(
    onProgress?: (progress: number, stage?: string) => void,
  ): Promise<void> {
    try {
      const update = await check({
        headers: {},
      });

      if (update?.available) {
        let contentLength = 0;
        let downloadedBytes = 0;

        await update.downloadAndInstall((progressEvent: DownloadEvent) => {
          // fnOS（shim 转发）：data 额外携带 daemon 的 stage / percentage；
          // 桌面插件形态只有 contentLength / chunkLength，回退字节换算。
          const data = (progressEvent as typeof progressEvent & {
            data?: {
              contentLength?: number;
              chunkLength?: number;
              stage?: string;
              percentage?: number;
            };
          }).data;
          const stage = data?.stage;
          if (progressEvent.event === "Started") {
            contentLength = data?.contentLength || 0;
            if (onProgress) {
              flushSync(() => {
                onProgress(0, stage || "connecting");
              });
            }
          } else if (progressEvent.event === "Progress" && onProgress) {
            downloadedBytes += data?.chunkLength || 0;
            let percentage: number;
            if (stage === "verifying" || stage === "applying") {
              percentage = 100; // 校验/应用发生在下载完成后
            } else if (typeof data?.percentage === "number") {
              percentage = data.percentage; // fnOS：daemon 权威百分比
            } else if (contentLength > 0) {
              percentage = (downloadedBytes / contentLength) * 100;
            } else {
              percentage = 0;
            }
            flushSync(() => {
              onProgress(
                Math.min(Math.max(percentage, 0), 100),
                stage || "downloading",
              );
            });
          }
        });
      } else {
        throw new Error("没有可用的更新");
      }
    } catch (error) {
      const errorMsg = error instanceof Error ? error.message : String(error);
      throw new Error(`安装更新失败: ${errorMsg}`);
    }
  }

  /**
   * 获取自动检测更新设置
   */
  getAutoCheckEnabled(): boolean {
    if (typeof window === "undefined") return true;
    const stored = localStorage.getItem("autoCheckUpdate");
    return stored !== "false"; // 默认为 true
  }

  /**
   * 设置自动检测更新
   */
  setAutoCheckEnabled(enabled: boolean): void {
    if (typeof window === "undefined") return;
    localStorage.setItem("autoCheckUpdate", enabled ? "true" : "false");
  }

  /**
   * 获取当前应用版本
   */
  async getCurrentVersion(): Promise<string> {
    try {
      return await getVersion();
    } catch (error) {
      console.error("获取版本失败:", error);
      return "未知";
    }
  }
}

export const updateService = new UpdateService();
