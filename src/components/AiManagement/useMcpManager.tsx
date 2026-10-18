import { errorMessage } from "@/utils/errorMessage";
import type {
  AiMcpOAuthStatus,
  AiMcpServerItem,
} from "@/bindings";
import { commands } from "@/bindings";
import {
  CopyOutlined,
  DeleteOutlined,
  DisconnectOutlined,
  ReloadOutlined
} from "@ant-design/icons";
import type { MenuProps } from "antd";
import {
  App as AntdApp
} from "antd";
import { useCallback, useMemo } from "react";
import { useMcpStore } from "@/stores/mcpStore";
import { busyKey, clientLabel, CLIENTS, isRemoteType, McpClient, ServerFilter, serverKey } from './mcpPresentation';
export function useMcpManager() {
  const { message, modal } = AntdApp.useApp();
  const { activeClient, overview, serversByClient, oauthByServer, busy, exclusiveAction, loading, scanning,
    loadError, serverQuery, serverFilter, updateServers: setServersByClient,
    beginAction: beginExclusiveAction, finishAction: finishExclusiveAction, refresh: loadAll, syncClient } = useMcpStore();
  const setActiveClient = (activeClient: McpClient) => useMcpStore.setState({ activeClient });
  const setServerQuery = (serverQuery: string) => useMcpStore.setState({ serverQuery });
  const setServerFilter = (serverFilter: ServerFilter) => useMcpStore.setState({ serverFilter });
  const setScanning = (scanning: boolean) => useMcpStore.setState({ scanning });
  const setOauthByServer = (update: (current: Record<string, AiMcpOAuthStatus>) => Record<string, AiMcpOAuthStatus>) =>
    useMcpStore.setState((state) => ({ oauthByServer: update(state.oauthByServer) }));
  const setActionBusy = useCallback((key: string, value: boolean) => {
    useMcpStore.setState((state) => {
      const busy = { ...state.busy };
      if (value) busy[key] = true;
      else delete busy[key];
      return { busy };
    });
  }, []);

  const reconcileClient = async (client: McpClient, completed: string): Promise<boolean> => {
    try {
      await syncClient(client);
      return true;
    } catch (error) {
      void message.warning(`${completed}，列表同步失败：${errorMessage(error)}。请手动刷新。`);
      return false;
    }
  };

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
      if (!await reconcileClient(activeClient, "扫描已完成")) return;
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
      if (!await reconcileClient(server.client as McpClient, server.name + " 已授权")) return;
      void message.success(server.name + " 授权成功");
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
      if (!await reconcileClient(server.client as McpClient, "撤销操作已完成")) return;
      const text = result || (server.name + " 的授权已撤销");
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
      if (!await reconcileClient(target, "复制已完成")) return;
      void message.success("已将 " + server.name + " 复制到 " + clientLabel(target));
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
      const client = server.client as McpClient;
      setServersByClient((current) => ({ ...current, [client]: current[client].filter((item) => item.name !== server.name) }));
      void message.success("已删除 MCP 服务器 " + server.name);
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
