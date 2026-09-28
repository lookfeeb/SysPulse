import { errorMessage } from "@/utils/errorMessage";
import type {
  AiMcpOAuthStatus,
  AiMcpOverview,
  AiMcpServerItem,
} from "@/bindings";
import { commands } from "@/bindings";
import {
  CopyOutlined,
  DeleteOutlined,
  DisconnectOutlined,
  ReloadOutlined
} from "@ant-design/icons";
import { listen } from "@tauri-apps/api/event";
import type { MenuProps } from "antd";
import {
  App as AntdApp
} from "antd";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ExclusiveActionLock } from "./mcpActionLock";
import { busyKey, clientLabel, CLIENTS, EMPTY_SERVERS, isRemoteType, McpClient, ServerFilter, serverKey, sortMcpServers } from './mcpPresentation';
export function useMcpManager() {
  const { message, modal } = AntdApp.useApp();
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
  const activeStats = useMemo(() => ({
    total: servers.length,
    enabled: servers.filter((server) => !server.disabled).length,
  }), [servers]);

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

  const confirmRevoke = (server: AiMcpServerItem) => {
    modal.confirm({
      title: "撤销 OAuth 授权",
      content: "解除 " + clientLabel(server.client)
        + " 中此服务器的绑定；若没有其他客户端共用，将同时尝试撤销服务端 Token。",
      okText: "撤销",
      cancelText: "取消",
      okButtonProps: { danger: true },
      onOk: () => revoke(server),
    });
  };

  const confirmDelete = (server: AiMcpServerItem) => {
    modal.confirm({
      title: "删除 MCP 服务器",
      content: "确定从 " + clientLabel(server.client) + " 删除 " + server.name + "？",
      okText: "删除",
      cancelText: "取消",
      okButtonProps: { danger: true },
      onOk: () => deleteServer(server),
    });
  };

  const activeClientMeta = CLIENTS.find((client) => client.key === activeClient) ?? CLIENTS[0];
  const conflictingActionActive = exclusiveAction !== null || loading || scanning;

  const buildMenu = (
    server: AiMcpServerItem,
    canManageToken: boolean,
  ): NonNullable<MenuProps["items"]> => {
    const targets = CLIENTS.filter((client) => client.key !== server.client);
    const items: NonNullable<MenuProps["items"]> = [
      {
        type: "group",
        key: "copy-group",
        label: "复制到",
        children: targets.map((target) => ({
          key: "copy:" + target.key,
          icon: <CopyOutlined />,
          label: target.label,
          disabled: conflictingActionActive,
          onClick: () => void copyServer(server, target.key),
        })),
      },
    ];

    if (canManageToken) {
      items.push({ type: "divider", key: "token-divider" });
      items.push({
        key: "token-refresh",
        icon: <ReloadOutlined />,
        label: "刷新 Token",
        disabled: conflictingActionActive,
        onClick: () => void refreshToken(server),
      });
      items.push({
        key: "token-revoke",
        icon: <DisconnectOutlined />,
        label: "撤销授权",
        danger: true,
        disabled: conflictingActionActive,
        onClick: () => confirmRevoke(server),
      });
    }

    items.push({ type: "divider", key: "delete-divider" });
    items.push({
      key: "delete",
      icon: <DeleteOutlined />,
      label: "删除服务器",
      danger: true,
      disabled: conflictingActionActive,
      onClick: () => confirmDelete(server),
    });
    return items;
  };


  return { activeClient, setActiveClient, overview, oauthByServer, busy, loading, scanning, loadError, serverQuery, setServerQuery, serverFilter, setServerFilter, loadAll, activeStats, filteredServers, toggleServer, discoverServers, refreshState, authorize, checkOAuth, cancelAuthorize, activeClientMeta, conflictingActionActive, buildMenu };
}
