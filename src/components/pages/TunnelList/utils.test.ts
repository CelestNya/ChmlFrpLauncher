// @vitest-environment jsdom
// B10 配套：restoreProgressFromLogs 纯函数——replay 帧的进度提取逻辑（TDD 目标函数）

import { describe, expect, it } from "vitest";
import { restoreProgressFromLogs } from "./utils";
import type { LogMessage } from "@/services/frpcManager";

function log(message: string): LogMessage {
  return { tunnel_id: 1, timestamp: "t", message };
}

describe("restoreProgressFromLogs 进度提取", () => {
  it("各进度关键词映射正确", () => {
    expect(restoreProgressFromLogs([log("frpc 进程已启动")]).get(1)?.progress).toBe(10);
    expect(restoreProgressFromLogs([log("从ChmlFrp API获取配置文件")]).get(1)?.progress).toBe(20);
    expect(restoreProgressFromLogs([log("已写入配置文件")]).get(1)?.progress).toBe(40);
    expect(restoreProgressFromLogs([log("成功登录至服务器")]).get(1)?.progress).toBe(60);
    expect(restoreProgressFromLogs([log("已启动隧道")]).get(1)?.progress).toBe(80);
    expect(restoreProgressFromLogs([log("映射启动成功")]).get(1)?.progress).toBe(100);
  });

  it("无关消息不产生进度", () => {
    expect(restoreProgressFromLogs([log("普通日志行")]).size).toBe(0);
    expect(restoreProgressFromLogs([]).size).toBe(0);
  });

  it("多条日志取最后一条有效进度", () => {
    const map = restoreProgressFromLogs([
      log("已启动隧道"),
      log("成功登录至服务器"),
    ]);
    expect(map.get(1)?.progress).toBe(80);
  });
});
