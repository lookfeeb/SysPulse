import {
  ApiOutlined,
  CheckCircleOutlined,
  CloudServerOutlined,
  GlobalOutlined,
  KeyOutlined,
  LinkOutlined,
  MoreOutlined,
  ReloadOutlined,
  ScanOutlined,
  SearchOutlined
} from "@ant-design/icons";
import {
  Alert,
  Button,
  Dropdown,
  Empty,
  Input,
  Spin,
  Switch,
  Tooltip
} from "antd";
import { busyKey, CLIENTS, FILTERS, isRemoteType, oauthMeta, oauthTooltip, serverKey } from './mcpPresentation';
import { useMcpManager } from './useMcpManager';
export default function McpManager() {
  const { activeClient, setActiveClient, overview, oauthByServer, busy, loading, scanning, loadError, serverQuery, setServerQuery, serverFilter, setServerFilter, loadAll, activeStats, filteredServers, toggleServer, discoverServers, refreshState, authorize, checkOAuth, cancelAuthorize, activeClientMeta, conflictingActionActive, buildMenu } = useMcpManager();
  return (
    <div className="aim-mcp">
      <aside className="aim-card aim-mcp-rail">
        <div className="aim-rail-hd">
          <strong>客户端</strong>
          <span>{CLIENTS.length}</span>
        </div>

        {CLIENTS.map((client) => {
          const stats = overview?.clients.find((item) => item.client === client.key);
          const active = client.key === activeClient;
          return (
            <button
              key={client.key}
              type="button"
              data-client={client.key}
              disabled={conflictingActionActive}
              className={"aim-client" + (active ? " is-active" : "")}
              aria-current={active ? "true" : undefined}
              onClick={() => {
                setActiveClient(client.key);
                setServerQuery("");
              }}
            >
              <span className="aim-client-mono">{client.monogram}</span>
              <span className="aim-client-name">{client.label}</span>
              <span className="aim-client-desc">{client.description}</span>
              <span className="aim-client-ratio">
                <b>{stats?.enabledServers ?? 0}</b>/{stats?.totalServers ?? 0}
              </span>
            </button>
          );
        })}

        <div className="aim-rail-split" />

        <div className="aim-rail-stats">
          <div className="aim-rail-stat">
            <span><CloudServerOutlined />服务器</span>
            <b>{overview?.totalServers ?? 0}</b>
          </div>
          <div className="aim-rail-stat aim-rail-stat--accent">
            <span><CheckCircleOutlined />已启用</span>
            <b>{overview?.enabledServers ?? 0}</b>
          </div>
        </div>
      </aside>

      <section className="aim-card aim-mcp-work">
        <header className="aim-card-hd">
          <div className="aim-card-hd-text">
            <strong>{activeClientMeta.label} 服务器</strong>
            <span>
              {activeStats.total} 项配置 · {activeStats.enabled} 项已启用 · 工具数需连接后探测
            </span>
          </div>
          <Button
            className="aim-act"
            icon={<ScanOutlined />}
            loading={scanning}
            disabled={conflictingActionActive}
            onClick={() => void discoverServers()}
          >
            扫描导入
          </Button>
          <Tooltip title="刷新配置与授权状态">
            <Button
              className="aim-act aim-act--icon"
              aria-label="刷新状态"
              icon={<ReloadOutlined />}
              loading={loading && !scanning}
              disabled={conflictingActionActive}
              onClick={() => void refreshState()}
            />
          </Tooltip>
        </header>

        {loadError && (
          <Alert
            className="aim-mcp-alert"
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

        <div className="aim-mcp-tools">
          <Input
            allowClear
            className="aim-input"
            value={serverQuery}
            prefix={<SearchOutlined />}
            placeholder="搜索名称、类型或连接地址"
            onChange={(event) => setServerQuery(event.target.value)}
          />
          <div className="aim-seg" role="group" aria-label="服务器筛选">
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
          <span className="aim-mcp-count">{filteredServers.length} 项</span>
        </div>

        <div className="aim-srv-scroll">
          <Spin spinning={loading}>
            {filteredServers.length === 0 ? (
              <div className="aim-empty">
                <Empty
                  image={Empty.PRESENTED_IMAGE_SIMPLE}
                  description={serverQuery || serverFilter !== "all"
                    ? "没有匹配的 MCP 服务器"
                    : "当前客户端没有 MCP 配置"}
                />
              </div>
            ) : filteredServers.map((server) => {
              const remote = isRemoteType(server.type);
              const status = remote ? oauthByServer[serverKey(server)] : undefined;
              const meta = remote ? oauthMeta(status) : null;
              const authorizing = !!busy[busyKey("authorize", server)];
              const cancelling = !!busy[busyKey("cancel", server)];
              const revoking = !!busy[busyKey("revoke", server)];
              const checkingOAuth = !!busy[busyKey("oauth-check", server)];
              const needsAuthorization = remote
                && status?.oauthSupported === true
                && (!status?.authorized || status.needsReauth || status.expired);
              const canManageToken = remote
                && status?.oauthSupported === true
                && !!status?.authorized
                && !needsAuthorization;
              const busyInMenu = revoking
                || !!busy[busyKey("token", server)]
                || !!busy[busyKey("delete", server)]
                || CLIENTS.some((target) => !!busy[busyKey("copy", server, target.key)]);

              return (
                <article
                  key={serverKey(server)}
                  className={"aim-srv" + (server.disabled ? " is-disabled" : "")}
                >
                  <span
                    className={"aim-srv-icon " + (remote ? "is-remote" : "is-local")}
                    title={remote ? "远程 MCP 服务" : "本地进程服务，无需 OAuth"}
                  >
                    {remote ? <GlobalOutlined /> : <ApiOutlined />}
                  </span>

                  <div className="aim-srv-body">
                    <div className="aim-srv-name">
                      <Tooltip title={server.name}>
                        <strong>{server.name}</strong>
                      </Tooltip>
                      <span className="aim-chip aim-chip--type">{server.type.toUpperCase()}</span>
                      {meta && (
                        <Tooltip title={oauthTooltip(status)}>
                          <span className={"aim-chip aim-chip--" + meta.tone}>
                            {meta.icon}
                            {meta.label}
                          </span>
                        </Tooltip>
                      )}
                      {server.disabled && <span className="aim-chip aim-chip--mute">已停用</span>}
                    </div>
                    <div className="aim-srv-endpoint">
                      {remote ? <LinkOutlined /> : <ApiOutlined />}
                      <code title={server.detail || "未提供"}>{server.detail || "未提供"}</code>
                    </div>
                  </div>

                  <div className="aim-srv-actions">
                    {remote && status && status.oauthSupported === null && (
                      <Button
                        className="aim-act aim-act--sm"
                        icon={<ReloadOutlined />}
                        loading={checkingOAuth}
                        disabled={conflictingActionActive}
                        onClick={() => void checkOAuth(server)}
                      >
                        重试检测
                      </Button>
                    )}
                    {authorizing ? (
                      <Button
                        className="aim-act aim-act--sm aim-act--danger"
                        danger
                        loading={cancelling}
                        disabled={cancelling}
                        onClick={() => void cancelAuthorize(server)}
                      >
                        取消授权
                      </Button>
                    ) : needsAuthorization ? (
                      <Button
                        className="aim-act aim-act--sm aim-act--primary"
                        icon={<KeyOutlined />}
                        disabled={conflictingActionActive}
                        onClick={() => void authorize(server)}
                      >
                        {status?.authorized ? "重新授权" : "授权"}
                      </Button>
                    ) : null}

                    <Tooltip title={server.disabled ? "启用服务器" : "停用服务器"}>
                      <Switch
                        className="aim-switch"
                        checked={!server.disabled}
                        loading={!!busy[busyKey("toggle", server)]}
                        disabled={conflictingActionActive}
                        onChange={(checked) => void toggleServer(server, checked)}
                      />
                    </Tooltip>

                    <Dropdown
                      trigger={["click"]}
                      classNames={{ root: "aim-menu" }}
                      disabled={conflictingActionActive}
                      menu={{ items: buildMenu(server, canManageToken) }}
                    >
                      <Button
                        className="aim-act aim-act--sm aim-act--icon aim-act--ghost aim-srv-more"
                        aria-label={"更多操作：" + server.name}
                        loading={busyInMenu}
                        icon={<MoreOutlined />}
                      />
                    </Dropdown>
                  </div>
                </article>
              );
            })}
          </Spin>
        </div>
      </section>
    </div>
  );
}
