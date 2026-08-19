import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Alert,
  App as AntdApp,
  Button,
  Dropdown,
  Empty,
  Input,
  Popconfirm,
  Space,
  Spin,
  Switch,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import {
  ApiOutlined,
  CheckCircleOutlined,
  CloudServerOutlined,
  CopyOutlined,
  DeleteOutlined,
  DisconnectOutlined,
  GlobalOutlined,
  KeyOutlined,
  LinkOutlined,
  ReloadOutlined,
  SafetyCertificateOutlined,
  ScanOutlined,
  SearchOutlined,
} from "@ant-design/icons";
import { listen } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import type {
  AiMcpOAuthStatus,
  AiMcpOverview,
  AiMcpServerItem,
} from "@/bindings";
import { ExclusiveActionLock } from "./mcpActionLock";

type McpClient = "codex" | "kiro" | "claude-cli";
type ServerFilter = "all" | "enabled" | "remote";

const CLIENTS: ReadonlyArray<{
  key: McpClient;
  label: string;
  monogram: string;
  description: string;
}> = [
  { key: "codex", label: "Codex", monogram: "CX", description: "OpenAI 编程客户端" },
  { key: "kiro", label: "Kiro", monogram: "KR", description: "IDE 智能开发环境" },
  { key: "claude-cli", label: "Claude CLI", monogram: "CL", description: "Anthropic 命令行" },
];

const FILTERS: ReadonlyArray<{ key: ServerFilter; label: string }> = [
  { key: "all", label: "全部" },
  { key: "enabled", label: "已启用" },
  { key: "remote", label: "远程服务" },
];

const EMPTY_SERVERS: Record<McpClient, AiMcpServerItem[]> = {
  codex: [],
  kiro: [],
  "claude-cli": [],
};

function errorMessage(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) {
    return String((error as { message: unknown }).message);
  }
  return error instanceof Error ? error.message : String(error);
}

function isRemoteType(type: string): boolean {
  return type === "url" || type === "http" || type === "sse";
}

function serverKey(server: Pick<AiMcpServerItem, "client" | "name">): string {
  return server.client + ":" + server.name;
}

function busyKey(action: string, server: AiMcpServerItem, target?: string): string {
  return action + ":" + serverKey(server) + (target ? ":" + target : "");
}

function clientLabel(client: string): string {
  return CLIENTS.find((item) => item.key === client)?.label ?? client;
}

function sortMcpServers(items: AiMcpServerItem[]): AiMcpServerItem[] {
  return [...items].sort((left, right) => {
    const leftRemote = isRemoteType(left.type);
    const rightRemote = isRemoteType(right.type);
    if (leftRemote !== rightRemote) return leftRemote ? -1 : 1;
    return left.name.localeCompare(right.name);
  });
}

function oauthMeta(status?: AiMcpOAuthStatus): {
  label: string;
  color: string;
  icon: React.ReactNode;
} {
  if (!status) {
    return { label: "检测中", color: "default", icon: <ReloadOutlined spin /> };
  }
  if (status.oauthSupported === false) {
    return { label: "无需 OAuth", color: "success", icon: <CheckCircleOutlined /> };
  }
  if (status.oauthSupported === null) {
    return { label: "检测失败", color: "warning", icon: <DisconnectOutlined /> };
  }
  if (!status.authorized) {
    return { label: "未授权", color: "default", icon: <KeyOutlined /> };
  }
  if (status.needsReauth || status.expired) {
    return { label: "需重新授权", color: "error", icon: <DisconnectOutlined /> };
  }
  if (status.refreshFailed) {
    return { label: "刷新失败", color: "orange", icon: <DisconnectOutlined /> };
  }
  if (status.expiringSoon) {
    return { label: "即将过期", color: "gold", icon: <KeyOutlined /> };
  }
  return { label: "已授权", color: "success", icon: <CheckCircleOutlined /> };
}

function oauthTooltip(status?: AiMcpOAuthStatus): string {
  if (!status) return "正在读取 OAuth 状态";
  if (status.message) return status.message;
  if (status.oauthSupported === false) return "该远程 MCP 可直接连接，无需 OAuth";
  if (status.oauthSupported === null) return "暂时无法确认该服务是否支持 OAuth";
  if (!status.authorized) return "尚未保存 OAuth 凭据";
  if (!status.expiresAt) return "已授权，但服务未返回 Token 到期时间；系统会按保守周期自动续期";
  const milliseconds = status.expiresAt > 10_000_000_000
    ? status.expiresAt
    : status.expiresAt * 1000;
  return "Token 到期时间：" + new Date(milliseconds).toLocaleString("zh-CN", { hour12: false });
}

export default function McpManager() {
  const { message } = AntdApp.useApp();
  const [activeClient, setActiveClient] = useState<McpClient>("codex");
  const [overview, setOverview] = useState<AiMcpOverview | null>(null);
  const [serversByClient, setServersByClient] =
    useState<Record<McpClient, AiMcpServerItem[]>>(EMPTY_SERVERS);
  const [oauthByServer, setOauthByServer] = useState<Record<string, AiMcpOAuthStatus>>({});
  const [busy, setBusy] = useState<Record<string, boolean>>({});
  const [exclusiveAction, setExclusiveAction] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [serverQuery, setServerQuery] = useState("");
  const [serverFilter, setServerFilter] = useState<ServerFilter>("all");
  const serversRef = useRef(serversByClient);
  const loadVersionRef = useRef(0);
  const oauthLoadVersionRef = useRef(0);
  const actionLockRef = useRef(new ExclusiveActionLock());

  useEffect(() => {
    serversRef.current = serversByClient;
  }, [serversByClient]);

  const setActionBusy = useCallback((key: string, value: boolean) => {
    setBusy((current) => {
      if (value) return { ...current, [key]: true };
      const next = { ...current };
      delete next[key];
      return next;
    });
  }, []);

  const beginExclusiveAction = useCallback((key: string): boolean => {
    if (!actionLockRef.current.acquire(key)) return false;
    setExclusiveAction(key);
    return true;
  }, []);

  const finishExclusiveAction = useCallback((key: string) => {
    if (actionLockRef.current.release(key)) setExclusiveAction(null);
  }, []);

  const loadOAuth = useCallback(async (servers: AiMcpServerItem[]) => {
    const requestVersion = ++oauthLoadVersionRef.current;
    const serverSnapshotVersion = loadVersionRef.current;
    const remoteServers = servers.filter((server) => isRemoteType(server.type));
    const settled = await Promise.allSettled(
      remoteServers.map(async (server) => ({
        key: serverKey(server),
        status: await commands.aiMcpOauthStatus({
          client: server.client,
          name: server.name,
        }),
      })),
    );
    const loaded: Record<string, AiMcpOAuthStatus> = {};
    for (const result of settled) {
      if (result.status === "fulfilled") {
        loaded[result.value.key] = result.value.status;
      }
    }
    if (requestVersion !== oauthLoadVersionRef.current) return;
    const activeKeys = new Set(remoteServers.map(serverKey));
    setOauthByServer((current) => {
      const next: Record<string, AiMcpOAuthStatus> = {};
      for (const [key, status] of Object.entries(current)) {
        if (activeKeys.has(key)) next[key] = status;
      }
      return { ...next, ...loaded };
    });

    void serverSnapshotVersion;
  }, []);

  const loadAll = useCallback(async (): Promise<boolean> => {
    const requestVersion = ++loadVersionRef.current;
    setLoading(true);
    setLoadError(null);
    try {
      const [nextOverview, codex, kiro, claude] = await Promise.all([
        commands.aiGetMcpOverview(),
        commands.aiGetMcpServers({ client: "codex" }),
        commands.aiGetMcpServers({ client: "kiro" }),
        commands.aiGetMcpServers({ client: "claude-cli" }),
      ]);
      const nextServers: Record<McpClient, AiMcpServerItem[]> = {
        codex,
        kiro,
        "claude-cli": claude,
      };
      for (const client of CLIENTS) nextServers[client.key] = sortMcpServers(nextServers[client.key]);
      if (requestVersion !== loadVersionRef.current) return false;
      setOverview(nextOverview);
      setServersByClient(nextServers);
      serversRef.current = nextServers;
      void loadOAuth(Object.values(nextServers).flat());
      return true;
    } catch (error: unknown) {
      if (requestVersion === loadVersionRef.current) setLoadError(errorMessage(error));
      return false;
    } finally {
      if (requestVersion === loadVersionRef.current) setLoading(false);
    }
  }, [loadOAuth]);

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen("mcp-tokens-updated", () => {
      void loadOAuth(Object.values(serversRef.current).flat());
    }).then((dispose) => {
      if (disposed) {
        dispose();
        return;
      }
      unlisten = dispose;
    }).catch((error: unknown) => {
      if (!disposed) setLoadError("监听 MCP Token 更新失败：" + errorMessage(error));
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [loadOAuth]);

  const servers = serversByClient[activeClient];
  const activeStats = useMemo(() => {
    const enabled = servers.filter((server) => !server.disabled).length;
    return {
      total: servers.length,
      enabled,
      percent: servers.length === 0 ? 0 : Math.round((enabled / servers.length) * 100),
    };
  }, [servers]);

  const filteredServers = useMemo(() => {
    const query = serverQuery.trim().toLocaleLowerCase("zh-CN");
    return servers.filter((server) => {
      if (serverFilter === "enabled" && server.disabled) return false;
      if (serverFilter === "remote" && !isRemoteType(server.type)) return false;
      if (!query) return true;
      return server.name.toLocaleLowerCase("zh-CN").includes(query)
        || server.detail.toLocaleLowerCase("zh-CN").includes(query)
        || server.type.toLocaleLowerCase("zh-CN").includes(query);
    });
  }, [serverFilter, serverQuery, servers]);

  const toggleServer = async (server: AiMcpServerItem, enabled: boolean) => {
    const key = busyKey("toggle", server);
    if (!beginExclusiveAction(key)) return;
    setActionBusy(key, true);
    try {
      await commands.aiToggleMcpServer({
        client: server.client,
        name: server.name,
        disabled: !enabled,
      });
      const sourceClient = server.client as McpClient;
      setServersByClient((current) => ({
        ...current,
        [sourceClient]: current[sourceClient].map((item) =>
          item.name === server.name ? { ...item, disabled: !enabled } : item
        ),
      }));
      const nextOverview = await commands.aiGetMcpOverview();
      setOverview(nextOverview);
      void message.success(enabled ? "MCP 服务器已启用" : "MCP 服务器已停用");
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setActionBusy(key, false);
      finishExclusiveAction(key);
    }
  };

  const discoverServers = async () => {
    const key = "scan:" + activeClient;
    if (!beginExclusiveAction(key)) return;
    setScanning(true);
    try {
      const imported = await commands.aiDiscoverMcpServers({ client: activeClient });
      const refreshed = await loadAll();
      if (!refreshed) {
        void message.warning("扫描已完成，但刷新配置失败，请重试刷新");
        return;
      }
      if (imported.length > 0) {
        void message.success("扫描完成，已导入 " + imported.length + " 项 MCP 配置");
      } else {
        void message.info("扫描完成，没有发现需要导入的新配置");
      }
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setScanning(false);
      finishExclusiveAction(key);
    }
  };

  const refreshState = async () => {
    const key = "refresh";
    if (!beginExclusiveAction(key)) return;
    try {
      if (await loadAll()) {
        void message.success({ content: "MCP 配置与授权状态已刷新", duration: 1.5 });
      }
    } finally {
      finishExclusiveAction(key);
    }
  };

  const authorize = async (server: AiMcpServerItem) => {
    const key = busyKey("authorize", server);
    if (!beginExclusiveAction(key)) return;
    setActionBusy(key, true);
    try {
      await commands.aiMcpOauthAuthorize({ client: server.client, name: server.name });
      const status = await commands.aiMcpOauthStatus({
        client: server.client,
        name: server.name,
      });
      setOauthByServer((current) => ({ ...current, [serverKey(server)]: status }));
      if (await loadAll()) {
        void message.success(server.name + " 授权成功");
      } else {
        void message.warning(server.name + " 已授权，但界面刷新失败");
      }
    } catch (error: unknown) {
      const text = errorMessage(error);
      if (text.includes("取消")) {
        void message.info("OAuth 授权已取消");
      } else if (text.includes("未声明 OAuth")) {
        try {
          const status = await commands.aiMcpOauthStatus({
            client: server.client,
            name: server.name,
          });
          setOauthByServer((current) => ({ ...current, [serverKey(server)]: status }));
        } catch {
          // 保留当前状态，下一次刷新时重新检测。
        }
        void message.info("该服务无需 OAuth，可直接连接");
      } else {
        void message.error(text);
      }
    } finally {
      setActionBusy(key, false);
      finishExclusiveAction(key);
    }
  };

  const checkOAuth = async (server: AiMcpServerItem) => {
    const key = busyKey("oauth-check", server);
    if (!beginExclusiveAction(key)) return;
    setActionBusy(key, true);
    try {
      const status = await commands.aiMcpOauthStatus({
        client: server.client,
        name: server.name,
      });
      setOauthByServer((current) => ({ ...current, [serverKey(server)]: status }));
      if (status.oauthSupported === false) {
        void message.info("该服务无需 OAuth，可直接连接");
      } else if (status.oauthSupported === true) {
        void message.success("已确认该服务支持 OAuth");
      } else {
        void message.warning(status.message || "暂时无法确认 OAuth 能力");
      }
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setActionBusy(key, false);
      finishExclusiveAction(key);
    }
  };

  const cancelAuthorize = async (server: AiMcpServerItem) => {
    const key = busyKey("cancel", server);
    setActionBusy(key, true);
    try {
      await commands.aiMcpOauthCancel({ client: server.client, name: server.name });
      void message.info("正在取消 OAuth 授权");
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setActionBusy(key, false);
    }
  };

  const refreshToken = async (server: AiMcpServerItem) => {
    const key = busyKey("token", server);
    if (!beginExclusiveAction(key)) return;
    setActionBusy(key, true);
    try {
      const status = await commands.aiMcpOauthRefresh({
        client: server.client,
        name: server.name,
      });
      setOauthByServer((current) => ({ ...current, [serverKey(server)]: status }));
      await loadOAuth(Object.values(serversRef.current).flat());
      void message.success(server.name + " Token 已刷新");
    } catch (error: unknown) {
      void message.error(errorMessage(error));
      try {
        const status = await commands.aiMcpOauthStatus({
          client: server.client,
          name: server.name,
        });
        setOauthByServer((current) => ({ ...current, [serverKey(server)]: status }));
      } catch {
        // 保留原状态，下一次全量刷新时重试。
      }
    } finally {
      setActionBusy(key, false);
      finishExclusiveAction(key);
    }
  };

  const revoke = async (server: AiMcpServerItem) => {
    const key = busyKey("revoke", server);
    if (!beginExclusiveAction(key)) return;
    setActionBusy(key, true);
    try {
      const result = await commands.aiMcpOauthRevoke({ client: server.client, name: server.name });
      const refreshed = await loadAll();
      const text = result || (server.name + " 的授权已撤销");
      if (!refreshed) {
        void message.warning(text + "；界面刷新失败");
        return;
      }
      if (text.includes("未完成") || text.includes("未提供") || text.includes("未恢复")) {
        void message.warning(text);
      } else {
        void message.success(text);
      }
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setActionBusy(key, false);
      finishExclusiveAction(key);
    }
  };

  const copyServer = async (server: AiMcpServerItem, target: McpClient) => {
    const key = busyKey("copy", server, target);
    if (!beginExclusiveAction(key)) return;
    setActionBusy(key, true);
    try {
      await commands.aiCopyMcpServer({
        fromClient: server.client,
        toClient: target,
        name: server.name,
        overwrite: true,
      });
      if (await loadAll()) {
        void message.success("已将 " + server.name + " 复制到 " + clientLabel(target));
      } else {
        void message.warning("复制已完成，但界面刷新失败");
      }
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setActionBusy(key, false);
      finishExclusiveAction(key);
    }
  };

  const deleteServer = async (server: AiMcpServerItem) => {
    const key = busyKey("delete", server);
    if (!beginExclusiveAction(key)) return;
    setActionBusy(key, true);
    try {
      await commands.aiDeleteMcpServer({ client: server.client, name: server.name });
      if (await loadAll()) {
        void message.success("已删除 MCP 服务器 " + server.name);
      } else {
        void message.warning("服务器已删除，但界面刷新失败");
      }
    } catch (error: unknown) {
      void message.error(errorMessage(error));
    } finally {
      setActionBusy(key, false);
      finishExclusiveAction(key);
    }
  };

  const activeClientMeta = CLIENTS.find((client) => client.key === activeClient) ?? CLIENTS[0];
  const conflictingActionActive = exclusiveAction !== null || loading || scanning;

  return (
    <div className="ai-mcp-console">
      <aside className="ai-mcp-rail">
        <div className="ai-mcp-rail-heading">
          <span className="ai-section-kicker">CLIENT MATRIX</span>
          <h2>客户端</h2>
          <p>选择配置来源并查看当前能力覆盖。</p>
        </div>

        <div className="ai-mcp-health">
          <div className="ai-mcp-health-value">
            <strong>{activeStats.percent}</strong><span>%</span>
          </div>
          <div>
            <b>当前启用率</b>
            <span>{activeStats.enabled} / {activeStats.total} 个服务器已启用</span>
          </div>
          <div className="ai-mcp-health-bar" aria-hidden="true">
            <i style={{ width: activeStats.percent + "%" }} />
          </div>
        </div>

        <div className="ai-client-list">
          {CLIENTS.map((client) => {
            const stats = overview?.clients.find((item) => item.client === client.key);
            const active = client.key === activeClient;
            return (
              <button
                key={client.key}
                type="button"
                disabled={conflictingActionActive}
                className={active ? "is-active" : ""}
                onClick={() => {
                  setActiveClient(client.key);
                  setServerQuery("");
                }}
              >
                <span className="ai-client-monogram">{client.monogram}</span>
                <span className="ai-client-copy">
                  <strong>{client.label}</strong>
                  <small>{client.description}</small>
                </span>
                <span className="ai-client-ratio">
                  <b>{stats?.enabledServers ?? 0}</b>/{stats?.totalServers ?? 0}
                </span>
              </button>
            );
          })}
        </div>

        <div className="ai-mcp-inventory">
          <div>
            <CloudServerOutlined />
            <span>全部服务器</span>
            <b>{overview?.totalServers ?? 0}</b>
          </div>
          <div>
            <CheckCircleOutlined />
            <span>已启用</span>
            <b>{overview?.enabledServers ?? 0}</b>
          </div>
        </div>
      </aside>

      <section className="ai-mcp-workspace">
        {loadError && (
          <Alert
            type="error"
            showIcon
            title="MCP 配置加载失败"
            description={loadError}
            action={(
              <Button
                size="small"
                disabled={conflictingActionActive}
                onClick={() => void loadAll()}
              >
                重试
              </Button>
            )}
          />
        )}

        <header className="ai-panel-heading">
          <div>
            <span className="ai-section-kicker">SERVER WORKSPACE</span>
            <h2>{activeClientMeta.label} MCP 服务器</h2>
            <p>{activeStats.total} 项配置，其中 {activeStats.enabled} 项已启用；实际工具数需连接后探测。</p>
          </div>
          <Space wrap size={8}>
            <Button
              icon={<ScanOutlined />}
              loading={scanning}
              disabled={conflictingActionActive}
              onClick={() => void discoverServers()}
            >
              扫描导入
            </Button>
            <Button
              icon={<ReloadOutlined />}
              loading={loading && !scanning}
              disabled={conflictingActionActive}
              onClick={() => void refreshState()}
            >
              刷新状态
            </Button>
          </Space>
        </header>

        <div className="ai-mcp-toolbar">
          <Input
            allowClear
            value={serverQuery}
            prefix={<SearchOutlined />}
            placeholder="搜索服务器、类型或连接信息"
            onChange={(event) => setServerQuery(event.target.value)}
          />
          <div className="ai-filter-switch" role="group" aria-label="服务器筛选">
            {FILTERS.map((filter) => (
              <button
                key={filter.key}
                type="button"
                className={serverFilter === filter.key ? "is-active" : ""}
                onClick={() => setServerFilter(filter.key)}
              >
                {filter.label}
              </button>
            ))}
          </div>
          <span className="ai-result-count">{filteredServers.length} 项结果</span>
        </div>

        <Spin spinning={loading}>
          {filteredServers.length === 0 ? (
            <div className="ai-mcp-empty">
              <Empty
                image={Empty.PRESENTED_IMAGE_SIMPLE}
                description={serverQuery || serverFilter !== "all"
                  ? "没有匹配的 MCP 服务器"
                  : "当前客户端没有 MCP 配置"}
              />
            </div>
          ) : (
            <div className="ai-mcp-server-list">
              {filteredServers.map((server) => {
                const remote = isRemoteType(server.type);
                const status = remote ? oauthByServer[serverKey(server)] : undefined;
                const meta = remote ? oauthMeta(status) : null;
                const authorizing = !!busy[busyKey("authorize", server)];
                const cancelling = !!busy[busyKey("cancel", server)];
                const refreshing = !!busy[busyKey("token", server)];
                const revoking = !!busy[busyKey("revoke", server)];
                const checkingOAuth = !!busy[busyKey("oauth-check", server)];
                const needsAuthorization = remote
                  && status?.oauthSupported === true
                  && (!status?.authorized || status.needsReauth || status.expired);
                const targets = CLIENTS.filter((client) => client.key !== server.client);
                const copying = targets.some(
                  (target) => !!busy[busyKey("copy", server, target.key)],
                );

                return (
                  <article
                    key={server.client + ":" + server.name}
                    className={"ai-mcp-server" + (server.disabled ? " is-disabled" : "")}
                  >
                    <header className="ai-mcp-server-header">
                      <div className="ai-mcp-server-identity">
                        <span className={"ai-mcp-type-icon " + (remote ? "remote" : "command")}>
                          {remote ? <GlobalOutlined /> : <ApiOutlined />}
                        </span>
                        <div>
                          <div className="ai-mcp-server-name">
                            <Tooltip title={server.name}>
                              <strong>{server.name}</strong>
                            </Tooltip>
                            <Tag color={remote ? "blue" : "gold"} bordered={false}>
                              {server.type.toUpperCase()}
                            </Tag>
                          </div>
                          <span>{remote ? "远程 MCP 服务" : "本地进程服务"}</span>
                        </div>
                      </div>

                      <div className="ai-mcp-runtime">
                        <span className={server.disabled ? "is-offline" : ""}>
                          <i />
                          {server.disabled ? "已停用" : "已配置"}
                        </span>
                        <Tooltip title={server.disabled ? "启用服务器" : "停用服务器"}>
                          <Switch
                            size="small"
                            checked={!server.disabled}
                            loading={!!busy[busyKey("toggle", server)]}
                            disabled={conflictingActionActive}
                            onChange={(checked) => void toggleServer(server, checked)}
                          />
                        </Tooltip>
                      </div>
                    </header>

                    <div className="ai-mcp-server-body">
                      <section className="ai-mcp-connection">
                        <div className="ai-mcp-field-label">
                          <LinkOutlined />
                          <span>连接端点</span>
                        </div>
                        <Tooltip title={server.detail || "未提供"}>
                          <Typography.Text code ellipsis>
                            {server.detail || "未提供"}
                          </Typography.Text>
                        </Tooltip>
                      </section>

                      <section className="ai-mcp-security">
                        <div className="ai-mcp-field-label">
                          <SafetyCertificateOutlined />
                          <span>访问授权</span>
                        </div>
                        <div className="ai-mcp-auth">
                          {remote && meta ? (
                            <>
                              <Tooltip title={oauthTooltip(status)}>
                                <Tag icon={meta.icon} color={meta.color}>{meta.label}</Tag>
                              </Tooltip>
                              {!status || status.oauthSupported === false ? null
                                : status.oauthSupported === null ? (
                                <Button
                                  size="small"
                                  loading={checkingOAuth}
                                  disabled={conflictingActionActive}
                                  icon={<ReloadOutlined />}
                                  onClick={() => void checkOAuth(server)}
                                >
                                  重试检测
                                </Button>
                              ) : authorizing ? (
                                <Button
                                  size="small"
                                  danger
                                  loading={cancelling}
                                  disabled={cancelling}
                                  onClick={() => void cancelAuthorize(server)}
                                >
                                  取消授权
                                </Button>
                              ) : needsAuthorization ? (
                                <Button
                                  size="small"
                                  type="primary"
                                  icon={<KeyOutlined />}
                                  disabled={conflictingActionActive}
                                  onClick={() => void authorize(server)}
                                >
                                  {status?.authorized ? "重新授权" : "授权"}
                                </Button>
                              ) : (
                                <>
                                  <Tooltip title="立即刷新 OAuth Token">
                                    <Button
                                      size="small"
                                      type="text"
                                      icon={<ReloadOutlined spin={refreshing} />}
                                      loading={refreshing}
                                      disabled={conflictingActionActive}
                                      onClick={() => void refreshToken(server)}
                                    >
                                      刷新
                                    </Button>
                                  </Tooltip>
                                  <Popconfirm
                                    title="撤销 OAuth 授权"
                                    description={
                                      "解除 " + clientLabel(server.client)
                                      + " 中此服务器的绑定；若没有其他客户端共用，将同时尝试撤销服务端 Token。"
                                    }
                                    okText="撤销"
                                    cancelText="取消"
                                    onConfirm={() => revoke(server)}
                                  >
                                    <Button
                                      size="small"
                                      type="text"
                                      danger
                                      loading={revoking}
                                      disabled={conflictingActionActive}
                                      icon={<DisconnectOutlined />}
                                    >
                                      撤销
                                    </Button>
                                  </Popconfirm>
                                </>
                              )}
                            </>
                          ) : (
                            <span className="ai-local-auth">
                              <CheckCircleOutlined />
                              本地进程无需 OAuth
                            </span>
                          )}
                        </div>
                      </section>
                    </div>

                    <footer className="ai-mcp-server-footer">
                      <span className="ai-mcp-config-source">
                        <ApiOutlined />
                        {clientLabel(server.client)} 配置
                      </span>
                      <div className="ai-mcp-server-actions">
                        <Dropdown
                          trigger={["click"]}
                          disabled={conflictingActionActive}
                          menu={{
                            items: targets.map((target) => ({
                              key: target.key,
                              label: "复制到 " + target.label,
                              disabled: conflictingActionActive,
                              onClick: () => void copyServer(server, target.key),
                            })),
                          }}
                        >
                          <Button
                            size="small"
                            loading={copying}
                            disabled={conflictingActionActive}
                            icon={<CopyOutlined />}
                          >
                            复制到
                          </Button>
                        </Dropdown>
                        <Popconfirm
                          title="删除 MCP 服务器"
                          description={"确定从 " + clientLabel(server.client) + " 删除 " + server.name + "？"}
                          okText="删除"
                          cancelText="取消"
                          okButtonProps={{ danger: true }}
                          onConfirm={() => deleteServer(server)}
                        >
                          <Button
                            type="text"
                            size="small"
                            danger
                            loading={!!busy[busyKey("delete", server)]}
                            disabled={conflictingActionActive}
                            icon={<DeleteOutlined />}
                          >
                            删除
                          </Button>
                        </Popconfirm>
                      </div>
                    </footer>
                  </article>
                );
              })}
            </div>
          )}
        </Spin>
      </section>
    </div>
  );
}
