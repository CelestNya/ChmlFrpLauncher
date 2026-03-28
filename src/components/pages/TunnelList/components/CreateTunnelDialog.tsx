import { useState, useCallback, useEffect } from "react";
import { cn } from "@/lib/utils";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { toast } from "sonner";
import {
  fetchNodeInfo,
  createTunnel,
  getStoredUser,
  type Node,
  type NodeInfo,
  type StoredUser,
} from "@/services/api";
import { checkLocalPort, type PortCheckResult } from "@/services/ports";
import { CustomTunnelDialog } from "./CustomTunnelDialog";
import { NodeSelector } from "./shared/NodeSelector";
import { NodeDetails } from "./shared/NodeDetails";
import { TunnelForm, type TunnelFormData } from "./shared/TunnelForm";

import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { RefreshCw } from "lucide-react";

interface CreateTunnelDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSuccess: () => void;
  preloadedNodes: Node[] | null;
  user?: StoredUser | null;
}

type PortStatus = PortCheckResult & {
  checking: boolean;
  checkedPort: string;
};

type PortUsage = {
  port: number;
  pid: number;
  process: string;
  protocol?: string;
};

function PortListTable({
                         title,
                         data,
                         loading,
                         onRefresh,
                       }: {
  title: string;
  data: PortUsage[];
  loading?: boolean;
  onRefresh?: () => void;
}) {
  if (!data.length) return null;

  return (
      <div className="mb-3 rounded-2xl border bg-background p-3 shadow-sm">
        <div className="mb-2 flex items-center justify-between">
          <div className="text-sm font-semibold">{title}</div>

          {onRefresh && (
              <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={onRefresh}
                  disabled={loading}
                  className="gap-2"
              >
                <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
                刷新
              </Button>
          )}
        </div>

        <div className="max-h-[300px] overflow-auto rounded-xl border">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-[80px]">端口</TableHead>
                <TableHead className="w-[80px]">PID</TableHead>
                <TableHead>进程</TableHead>
                <TableHead className="w-[90px]">协议</TableHead>
              </TableRow>
            </TableHeader>

            <TableBody>
              {data.map((item) => (
                  <TableRow key={`${item.port}-${item.pid}`}>
                    <TableCell className="font-medium">{item.port}</TableCell>
                    <TableCell>{item.pid}</TableCell>
                    <TableCell>{item.process}</TableCell>
                    <TableCell>{item.protocol || "-"}</TableCell>
                  </TableRow>
              ))}
            </TableBody>
          </Table>
        </div>
      </div>
  );
}

export function CreateTunnelDialog({
                                     open,
                                     onOpenChange,
                                     onSuccess,
                                     preloadedNodes,
                                     user,
                                   }: CreateTunnelDialogProps) {
  const [tunnelType, setTunnelType] = useState<"standard" | "custom">("standard");
  const [step, setStep] = useState<1 | 2 | 3 | 4>(1);
  const [loading, setLoading] = useState(false);
  const [selectedNode, setSelectedNode] = useState<Node | null>(null);
  const [nodeInfo, setNodeInfo] = useState<NodeInfo | null>(null);
  const [loadingNodeInfo, setLoadingNodeInfo] = useState(false);
  const [pingLatency, setPingLatency] = useState<number | null>(null);
  const [pinging, setPinging] = useState(false);
  const [pingError, setPingError] = useState(false);

  const [portStatus, setPortStatus] = useState<PortStatus | null>(null);
  const [portStatusError, setPortStatusError] = useState<string | null>(null);

  const [showPortOccupiedConfirm, setShowPortOccupiedConfirm] = useState(false);
  const [portOccupiedWarning, setPortOccupiedWarning] = useState<{
    localPort: string;
    processLabel: string;
  } | null>(null);

  // step3 端口占用列表
  const [portList, setPortList] = useState<PortUsage[] | null>(null);
  const [portListLoading, setPortListLoading] = useState(false);
  const [portListError, setPortListError] = useState<string | null>(null);

  const [formData, setFormData] = useState<TunnelFormData>({
    tunnelName: "",
    localIp: "127.0.0.1",
    portType: "TCP",
    localPort: "",
    remotePort: "",
    domain: "",
    encryption: false,
    compression: false,
    extraParams: "",
  });

  const nodes = preloadedNodes || [];

  // 获取端口占用列表（step3刷新按钮用）
  const fetchPortList = useCallback(async () => {
    try {
      setPortListLoading(true);
      setPortListError(null);

      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<PortUsage[]>("get_ports");

      setPortList(result);
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : String(err);
      setPortListError(message);
      setPortList(null);
    } finally {
      setPortListLoading(false);
    }
  }, []);

  // step3 进入时自动获取端口列表
  useEffect(() => {
    if (step === 3) {
      fetchPortList();
    }
  }, [step, fetchPortList]);

  // step4 端口检查（只检查用户输入的 localPort）
  useEffect(() => {
    if (step !== 4) return;

    const port = formData.localPort.trim();
    if (!port) {
      setPortStatus(null);
      setPortStatusError(null);
      return;
    }

    if (!/^\d+$/.test(port)) {
      setPortStatus(null);
      setPortStatusError("请输入有效的本地端口");
      return;
    }

    const portNumber = Number(port);
    if (portNumber < 1 || portNumber > 65535) {
      setPortStatus(null);
      setPortStatusError("端口范围必须在 1-65535 之间");
      return;
    }

    setPortStatusError(null);
    setPortStatus({
      occupied: false,
      pid: null,
      process: null,
      checking: true,
      checkedPort: port,
    });

    let cancelled = false;
    const timer = window.setTimeout(async () => {
      try {
        const result = await checkLocalPort(port);
        if (!cancelled) {
          setPortStatus({
            ...result,
            checking: false,
            checkedPort: port,
          });
        }
      } catch (error) {
        if (!cancelled) {
          const message = error instanceof Error ? error.message : "端口检查失败";
          setPortStatus(null);
          setPortStatusError(message);
        }
      }
    }, 300);

    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [formData.localPort, step]);

  const generateRandomTunnelName = () => {
    const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let result = "";
    for (let i = 0; i < 8; i++) {
      result += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return result;
  };

  const generateRandomPort = (portRange: string) => {
    const match = portRange.match(/(\d+)-(\d+)/);
    if (match) {
      const min = parseInt(match[1], 10);
      const max = parseInt(match[2], 10);
      return Math.floor(Math.random() * (max - min + 1)) + min;
    }
    const singlePort = parseInt(portRange, 10);
    return Number.isNaN(singlePort) ? 10000 : singlePort;
  };

  const loadNodeInfo = async (nodeName: string) => {
    try {
      setLoadingNodeInfo(true);
      const data = await fetchNodeInfo(nodeName);
      setNodeInfo(data);
      setStep(2);
      performPing(data.ip);
    } catch (error) {
      const message = error instanceof Error ? error.message : "获取节点信息失败";
      toast.error(message);
    } finally {
      setLoadingNodeInfo(false);
    }
  };

  const performPing = async (host: string) => {
    try {
      setPinging(true);
      setPingLatency(null);
      setPingError(false);

      const { invoke } = await import("@tauri-apps/api/core");
      const result = await invoke<{
        success: boolean;
        latency?: number;
        error?: string;
      }>("ping_host", { host });

      if (result.success && result.latency !== undefined) {
        setPingLatency(result.latency);
        setPingError(false);
      } else {
        setPingError(true);
      }
    } catch (error) {
      const errorMessage = error instanceof Error ? error.message : String(error);
      if (!errorMessage.includes("Cannot find module")) {
        setPingError(true);
      }
    } finally {
      setPinging(false);
    }
  };

  // step2 -> step3（端口占用情况）
  const goToStep3 = () => {
    setStep(3);
  };

  // step3 -> step4（配置隧道）
  const goToStep4 = () => {
    if (nodeInfo) {
      setFormData((prev) => ({
        ...prev,
        tunnelName: prev.tunnelName || generateRandomTunnelName(),
        remotePort: generateRandomPort(nodeInfo.rport).toString(),
      }));
    }
    setStep(4);
  };

  const handleNodeSelect = (node: Node) => {
    const currentUser = getStoredUser();
    const isFreeUser = currentUser?.usergroup === "免费用户";

    if (isFreeUser && node.nodegroup === "vip") {
      toast.error("此节点为会员节点，您的权限不足");
      return;
    }

    setSelectedNode(node);
    loadNodeInfo(node.name);
  };

  const submitTunnel = async (forceWhenPortOccupied: boolean) => {
    if (!selectedNode) {
      toast.error("请选择节点");
      return;
    }

    if (!formData.tunnelName.trim()) {
      toast.error("请输入隧道名称");
      return;
    }

    if (!formData.localPort) {
      toast.error("请输入本地端口");
      return;
    }

    const localPort = formData.localPort.trim();
    if (!/^\d+$/.test(localPort)) {
      toast.error("请输入有效的本地端口");
      return;
    }

    const localPortNumber = Number(localPort);
    if (localPortNumber < 1 || localPortNumber > 65535) {
      toast.error("端口范围必须在 1-65535 之间");
      return;
    }

    if (portStatusError) {
      toast.error(portStatusError);
      return;
    }

    if (portStatus?.checking && portStatus.checkedPort === localPort) {
      toast.error("端口占用检查尚未完成");
      return;
    }

    if (portStatus?.checkedPort === localPort && !portStatus.checking && portStatus.occupied) {
      const processLabel = portStatus.process
          ? `${portStatus.process} (PID ${portStatus.pid || "未知"})`
          : "当前进程";
      setPortOccupiedWarning({ localPort, processLabel });
      if (!forceWhenPortOccupied) {
        setShowPortOccupiedConfirm(true);
        return;
      }
    }

    const isHttpProtocol = formData.portType === "HTTP" || formData.portType === "HTTPS";

    if (isHttpProtocol) {
      if (!formData.domain.trim()) {
        toast.error("请输入域名");
        return;
      }
    } else if (!formData.remotePort) {
      toast.error("请输入远程端口");
      return;
    }

    try {
      setLoading(true);

      const baseTunnelParams = {
        tunnelname: formData.tunnelName,
        node: selectedNode.name,
        localip: formData.localIp,
        porttype: formData.portType,
        localport: localPortNumber,
        encryption: formData.encryption,
        compression: formData.compression,
        extraparams: formData.extraParams,
      };

      const tunnelParams = isHttpProtocol
          ? { ...baseTunnelParams, banddomain: formData.domain }
          : {
            ...baseTunnelParams,
            remoteport: parseInt(formData.remotePort, 10),
          };

      await createTunnel(tunnelParams);

      toast.success("隧道创建成功");
      onSuccess();
      handleClose();
    } catch (error) {
      const message = error instanceof Error ? error.message : "创建隧道失败";
      toast.error(message);
    } finally {
      setLoading(false);
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    await submitTunnel(false);
  };

  const handlePortOccupiedContinue = async () => {
    setShowPortOccupiedConfirm(false);
    await submitTunnel(true);
  };

  const handleClose = () => {
    setTunnelType("standard");
    setStep(1);
    setSelectedNode(null);
    setNodeInfo(null);
    setPingLatency(null);
    setPinging(false);
    setPingError(false);

    setPortStatus(null);
    setPortStatusError(null);

    setPortList(null);
    setPortListError(null);
    setPortListLoading(false);

    setShowPortOccupiedConfirm(false);
    setPortOccupiedWarning(null);

    setFormData({
      tunnelName: "",
      localIp: "127.0.0.1",
      portType: "TCP",
      localPort: "",
      remotePort: "",
      domain: "",
      encryption: false,
      compression: false,
      extraParams: "",
    });

    onOpenChange(false);
  };

  const handleBack = () => {
    if (step === 4) {
      setStep(3);
      return;
    }

    if (step === 3) {
      setStep(2);
      return;
    }

    if (step === 2) {
      setStep(1);
      setNodeInfo(null);
      setPingLatency(null);
      setPinging(false);
      setPingError(false);
    }
  };

  const handleFormChange = useCallback((updates: Partial<TunnelFormData>) => {
    setFormData((prev) => ({ ...prev, ...updates }));
  }, []);

  if (tunnelType === "custom") {
    return <CustomTunnelDialog open={open} onOpenChange={handleClose} onSuccess={onSuccess} />;
  }

  // step3 数据分类：java/python 单独表格
  const javaPythonPorts =
      portList?.filter((p) => {
        const name = p.process?.toLowerCase() || "";
        return name.includes("java") || name.includes("python");
      }) || [];

  const otherPorts =
      portList?.filter((p) => {
        const name = p.process?.toLowerCase() || "";
        return !(name.includes("java") || name.includes("python"));
      }) || [];

  return (
      <>
        <Dialog open={open} onOpenChange={handleClose}>
          <DialogContent
              className={cn(
                  "flex max-h-[90vh] flex-col",
                  step === 1
                      ? "max-w-6xl"
                      : step === 2
                          ? "max-w-4xl"
                          : step === 3
                              ? "max-w-4xl"
                              : "max-w-xl",
              )}
          >
            <DialogHeader className="shrink-0 gap-1.5">
              <DialogTitle className="animate-in fade-in text-xl duration-300" key={`title-${step}`}>
                {step === 1 && "新建隧道"}
                {step === 2 && "节点详情"}
                {step === 3 && "端口占用情况"}
                {step === 4 && "配置隧道"}
              </DialogTitle>

              {step === 2 && selectedNode && (
                  <DialogDescription className="animate-in fade-in text-sm duration-300" key="desc-step2">
                    {selectedNode.name} - 查看节点详细信息
                  </DialogDescription>
              )}

              {step === 3 && selectedNode && (
                  <DialogDescription className="animate-in fade-in text-sm duration-300" key="desc-step3">
                    节点：{selectedNode.name} - 查看本机端口占用情况
                  </DialogDescription>
              )}

              {step === 4 && selectedNode && (
                  <DialogDescription className="animate-in fade-in text-sm duration-300" key="desc-step4">
                    节点：{selectedNode.name} - 填写隧道配置信息
                  </DialogDescription>
              )}
            </DialogHeader>

            {step === 1 ? (
                <div
                    key="step1"
                    className="animate-in slide-in-from-bottom-2 fade-in flex min-h-0 flex-1 flex-col py-4 duration-300"
                >
                  {user && (
                      <Tabs
                          value={tunnelType}
                          onValueChange={(value) => setTunnelType(value as "standard" | "custom")}
                          className="mb-4"
                      >
                        <TabsList className="w-full">
                          <TabsTrigger value="standard" className="flex-1">
                            标准隧道
                          </TabsTrigger>
                          <TabsTrigger value="custom" className="flex-1">
                            自定义隧道
                          </TabsTrigger>
                        </TabsList>
                      </Tabs>
                  )}

                  <NodeSelector nodes={nodes} loading={loadingNodeInfo} onNodeSelect={handleNodeSelect} />
                </div>
            ) : step === 2 ? (
                <div
                    key="step2"
                    className="animate-in slide-in-from-bottom-2 fade-in flex min-h-0 flex-1 flex-col pt-3 duration-300"
                >
                  <NodeDetails
                      nodeInfo={nodeInfo}
                      pingLatency={pingLatency}
                      pinging={pinging}
                      pingError={pingError}
                  />

                  <DialogFooter className="gap-2 border-t pt-3">
                    <Button type="button" variant="outline" onClick={handleBack}>
                      返回
                    </Button>
                    <Button type="button" onClick={goToStep3}>
                      下一步
                    </Button>
                  </DialogFooter>
                </div>
            ) : step === 3 ? (
                <div
                    key="step3"
                    className="flex flex-1 flex-col overflow-hidden min-h-0 pt-3"
                >
                  {/* 可滚动的内容区域 */}
                  <div className="flex-1 overflow-auto pr-1">
                    {portListError && (
                        <div className="mb-3 rounded-xl border bg-red-50 p-2 text-sm text-red-600">
                          获取端口占用失败：{portListError}
                        </div>
                    )}

                    {!portListLoading && portList && portList.length === 0 && (
                        <div className="mb-3 rounded-xl border bg-muted/30 p-3 text-sm text-muted-foreground">
                          暂无端口占用数据
                        </div>
                    )}

                    {portListLoading && (
                        <div className="mb-3 rounded-xl border bg-muted/30 p-3 text-sm text-muted-foreground">
                          正在加载端口占用情况...
                        </div>
                    )}

                    {portList && portList.length > 0 && (
                        <>
                          {javaPythonPorts.length > 0 && (
                              <PortListTable
                                  title="⚠ Java / Python 占用端口"
                                  data={javaPythonPorts}
                                  loading={portListLoading}
                                  onRefresh={fetchPortList}
                              />
                          )}
                          <PortListTable
                              title="当前端口占用情况"
                              data={otherPorts}
                              loading={portListLoading}
                              onRefresh={fetchPortList}
                          />
                        </>
                    )}
                  </div>

                  <DialogFooter className="gap-2 border-t pt-3">
                    <Button type="button" variant="outline" onClick={handleBack}>
                      返回
                    </Button>
                    <Button type="button" onClick={goToStep4}>
                      下一步
                    </Button>
                  </DialogFooter>
                </div>
            ) : (
                <form
                    key="step4"
                    onSubmit={handleSubmit}
                    className="animate-in slide-in-from-bottom-2 fade-in flex min-h-0 flex-1 flex-col pt-3 duration-300"
                >
                  <TunnelForm
                      formData={formData}
                      onChange={handleFormChange}
                      nodeInfo={nodeInfo}
                      disabled={loading}
                      portStatus={portStatus}
                      portStatusError={portStatusError}
                  />

                  <DialogFooter className="shrink-0 gap-2 border-t pt-3">
                    <Button
                        type="button"
                        variant="outline"
                        onClick={handleBack}
                        disabled={loading}
                        className="min-w-[100px]"
                    >
                      返回
                    </Button>

                    <Button type="submit" disabled={loading} className="min-w-[100px]">
                      {loading ? (
                          <span className="flex items-center gap-2">
                      <svg className="h-4 w-4 animate-spin" viewBox="0 0 24 24">
                        <circle
                            className="opacity-25"
                            cx="12"
                            cy="12"
                            r="10"
                            stroke="currentColor"
                            strokeWidth="4"
                            fill="none"
                        />
                        <path
                            className="opacity-75"
                            fill="currentColor"
                            d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4zm2 5.291A7.962 7.962 0 014 12H0c0 3.042 1.135 5.824 3 7.938l3-2.647z"
                        />
                      </svg>
                      创建中...
                    </span>
                      ) : (
                          "创建隧道"
                      )}
                    </Button>
                  </DialogFooter>
                </form>
            )}
          </DialogContent>
        </Dialog>

        <Dialog open={showPortOccupiedConfirm} onOpenChange={setShowPortOccupiedConfirm}>
          <DialogContent className="max-w-md">
            <DialogHeader>
              <DialogTitle>端口占用确认</DialogTitle>
              <DialogDescription>
                {portOccupiedWarning
                    ? `检测到本地端口 ${portOccupiedWarning.localPort} 已被 ${portOccupiedWarning.processLabel} 占用。确定继续创建隧道吗？`
                    : "检测到本地端口已被占用。确定继续创建隧道吗？"}
              </DialogDescription>
            </DialogHeader>
            <DialogFooter>
              <Button
                  type="button"
                  variant="outline"
                  onClick={() => setShowPortOccupiedConfirm(false)}
                  disabled={loading}
              >
                取消
              </Button>
              <Button type="button" onClick={handlePortOccupiedContinue} disabled={loading}>
                继续创建
              </Button>
            </DialogFooter>
          </DialogContent>
        </Dialog>
      </>
  );
}
