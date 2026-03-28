import { useCallback } from "react";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import {
  Accordion,
  AccordionContent,
  AccordionItem,
  AccordionTrigger,
} from "@/components/ui/accordion";
import { toast } from "sonner";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { NodeInfo } from "@/services/api";
import type { PortCheckResult } from "@/services/ports";

export interface TunnelFormData {
  tunnelName: string;
  localIp: string;
  portType: string;
  localPort: string;
  remotePort: string;
  domain: string;
  encryption: boolean;
  compression: boolean;
  extraParams: string;
}

interface TunnelFormProps {
  formData: TunnelFormData;
  onChange: (data: Partial<TunnelFormData>) => void;
  nodeInfo: NodeInfo | null;
  disabled?: boolean;
  portStatus?: (PortCheckResult & { checking: boolean; checkedPort: string }) | null;
  portStatusError?: string | null;
}

export function TunnelForm({
  formData,
  onChange,
  nodeInfo,
  disabled = false,
  portStatus = null,
  portStatusError = null,
}: TunnelFormProps) {
  const handleCopyNodeIp = useCallback(async (ip: string) => {
    try {
      await navigator.clipboard.writeText(ip);
      toast.success("节点 IP 已复制");
    } catch (error) {
      console.error("Failed to copy IP:", error);
      toast.error("复制失败");
    }
  }, []);

  const handleOpenCnameDoc = useCallback(async () => {
    try {
      await openUrl("https://docs.chmlfrp.net/docs/dns/cname.html");
    } catch (error) {
      console.error("Failed to open URL:", error);
      toast.error("打开链接失败");
    }
  }, []);

  const isHttpProtocol =
    formData.portType === "HTTP" || formData.portType === "HTTPS";

  const currentPort = formData.localPort.trim();
  const hasMatchedPortStatus = portStatus?.checkedPort === currentPort;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto pr-4 transition-all duration-300 ease-in-out">
      <div className="space-y-4 pb-3">
        {nodeInfo && isHttpProtocol && (
          <div className="flex items-start gap-2 rounded-lg border border-blue-200 bg-blue-50 p-3 dark:border-blue-800/50 dark:bg-blue-950/30">
            <svg
              className="mt-0.5 h-4 w-4 flex-shrink-0 text-blue-600 dark:text-blue-400"
              fill="none"
              viewBox="0 0 24 24"
              stroke="currentColor"
            >
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M13 16h-1v-4h-1m1-4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
              />
            </svg>
            <p className="text-xs leading-relaxed text-blue-700 dark:text-blue-300">
              使用 {formData.portType} 隧道时，需要将
              <span className="mx-1 font-mono font-semibold">
                {formData.domain || "您的域名"}
              </span>
              通过
              <button
                type="button"
                onClick={handleOpenCnameDoc}
                className="mx-1 rounded px-0.5 underline decoration-dotted underline-offset-2 hover:decoration-solid focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-1"
              >
                CNAME 解析
              </button>
              指向
              <button
                type="button"
                onClick={() => handleCopyNodeIp(nodeInfo.ip)}
                className="mx-1 cursor-pointer rounded px-0.5 font-mono font-semibold underline decoration-dotted underline-offset-2 hover:decoration-solid focus:outline-none focus:ring-2 focus:ring-blue-500 focus:ring-offset-1"
                title="点击复制"
              >
                {nodeInfo.ip}
              </button>
              才能正常访问。
            </p>
          </div>
        )}

        <div className="space-y-2">
          <Label htmlFor="tunnelName" className="text-sm font-medium">
            隧道名称
          </Label>
          <Input
            id="tunnelName"
            value={formData.tunnelName}
            onChange={(e) => onChange({ tunnelName: e.target.value })}
            placeholder="为您的隧道起一个名字"
            required
            disabled={disabled}
            className="h-10 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
          />
        </div>

        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="localIp" className="text-sm font-medium">
              本地地址
            </Label>
            <Input
              id="localIp"
              value={formData.localIp}
              onChange={(e) => onChange({ localIp: e.target.value })}
              placeholder="127.0.0.1"
              required
              disabled={disabled}
              className="h-10 font-mono shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
            />
          </div>

          <div className="space-y-2">
            <Label htmlFor="localPort" className="text-sm font-medium">
              本地端口
            </Label>
            <Input
              id="localPort"
              type="number"
              value={formData.localPort}
              onChange={(e) => onChange({ localPort: e.target.value })}
              required
              disabled={disabled}
              className="h-10 font-mono shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
            />
            {currentPort && (
              <p
                className={cn(
                  "text-xs",
                  portStatusError || portStatus?.occupied
                    ? "text-destructive"
                    : portStatus?.checking
                      ? "text-muted-foreground"
                      : "text-emerald-600",
                )}
              >
                {portStatus?.checking && "正在检查端口占用..."}
                {!portStatus?.checking &&
                  !portStatusError &&
                  hasMatchedPortStatus &&
                  portStatus.occupied &&
                  `端口已被占用：${portStatus.process || "未知进程"} (PID ${portStatus.pid || "未知"})`}
                {!portStatus?.checking &&
                  !portStatusError &&
                  hasMatchedPortStatus &&
                  !portStatus.occupied &&
                  "端口当前可用"}
                {!portStatus?.checking && portStatusError && portStatusError}
              </p>
            )}
          </div>
        </div>

        <div className="grid grid-cols-2 gap-4">
          <div className="space-y-2">
            <Label htmlFor="portType" className="text-sm font-medium">
              协议类型
            </Label>
            <Select
              options={[
                { value: "TCP", label: "TCP" },
                { value: "UDP", label: "UDP" },
                { value: "HTTP", label: "HTTP" },
                { value: "HTTPS", label: "HTTPS" },
              ]}
              value={formData.portType}
              onChange={(value) => onChange({ portType: value as string })}
            />
          </div>

          {isHttpProtocol ? (
            <div className="space-y-2">
              <Label htmlFor="domain" className="text-sm font-medium">
                域名
              </Label>
              <Input
                id="domain"
                type="text"
                value={formData.domain}
                onChange={(e) => onChange({ domain: e.target.value })}
                placeholder="example.com"
                required
                disabled={disabled}
                className="h-10 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
              />
            </div>
          ) : (
            <div className="space-y-2">
              <Label htmlFor="remotePort" className="text-sm font-medium">
                远程端口
              </Label>
              <Input
                id="remotePort"
                type="number"
                value={formData.remotePort}
                onChange={(e) => onChange({ remotePort: e.target.value })}
                required
                disabled={disabled}
                className="h-10 font-mono shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
              />
            </div>
          )}
        </div>

        <Accordion type="single" collapsible className="rounded-lg border">
          <AccordionItem value="advanced" className="border-0">
            <AccordionTrigger className="px-4 py-3 hover:no-underline">
              <span className="text-sm font-medium">高级选项</span>
            </AccordionTrigger>
            <AccordionContent className="px-4 pb-4">
              <div className="space-y-4">
                <div className="space-y-2">
                  <Label htmlFor="extraParams" className="text-sm font-medium">
                    额外参数
                    <span className="ml-1.5 text-xs font-normal text-muted-foreground">
                      （可选）
                    </span>
                  </Label>
                  <Input
                    id="extraParams"
                    value={formData.extraParams}
                    onChange={(e) => onChange({ extraParams: e.target.value })}
                    placeholder="额外的配置参数"
                    disabled={disabled}
                    className="h-10 shadow-none focus-visible:ring-0 focus-visible:ring-offset-0"
                  />
                </div>

                <div className="flex items-center gap-6">
                  <label className="group flex cursor-pointer items-center gap-2.5 text-sm transition-colors hover:text-primary">
                    <input
                      type="checkbox"
                      checked={formData.encryption}
                      onChange={(e) =>
                        onChange({ encryption: e.target.checked })
                      }
                      disabled={disabled}
                      className="h-4 w-4 cursor-pointer rounded border-input transition-colors checked:border-primary checked:bg-primary focus:ring-2 focus:ring-ring focus:ring-offset-2"
                    />
                    <span className="font-medium">加密传输</span>
                  </label>
                  <label className="group flex cursor-pointer items-center gap-2.5 text-sm transition-colors hover:text-primary">
                    <input
                      type="checkbox"
                      checked={formData.compression}
                      onChange={(e) =>
                        onChange({ compression: e.target.checked })
                      }
                      disabled={disabled}
                      className="h-4 w-4 cursor-pointer rounded border-input transition-colors checked:border-primary checked:bg-primary focus:ring-2 focus:ring-ring focus:ring-offset-2"
                    />
                    <span className="font-medium">数据压缩</span>
                  </label>
                </div>
              </div>
            </AccordionContent>
          </AccordionItem>
        </Accordion>
      </div>
    </div>
  );
}
