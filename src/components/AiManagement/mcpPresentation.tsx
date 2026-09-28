import type {
  AiMcpOAuthStatus,
  AiMcpServerItem
} from "@/bindings";
import {
  CheckCircleOutlined,
  DisconnectOutlined,
  KeyOutlined,
  ReloadOutlined
} from "@ant-design/icons";

export type McpClient = "codex" | "kiro" | "claude-cli";

export type ServerFilter = "all" | "enabled" | "remote";

export const CLIENTS: ReadonlyArray<{
  key: McpClient;
  label: string;
  monogram: string;
  description: string;
}> = [
    { key: "codex", label: "Codex", monogram: "CX", description: "OpenAI 编程客户端" },
    { key: "kiro", label: "Kiro", monogram: "KR", description: "IDE 智能开发环境" },
    { key: "claude-cli", label: "Claude CLI", monogram: "CL", description: "Anthropic 命令行" },
  ];

export const FILTERS: ReadonlyArray<{ key: ServerFilter; label: string }> = [
  { key: "all", label: "全部" },
  { key: "enabled", label: "已启用" },
  { key: "remote", label: "远程" },
];

export const EMPTY_SERVERS: Record<McpClient, AiMcpServerItem[]> = {
  codex: [],
  kiro: [],
  "claude-cli": [],
};

export function isRemoteType(type: string): boolean {
  return type === "url" || type === "http" || type === "sse";
}

export function serverKey(server: Pick<AiMcpServerItem, "client" | "name">): string {
  return server.client + ":" + server.name;
}

export function busyKey(action: string, server: AiMcpServerItem, target?: string): string {
  return action + ":" + serverKey(server) + (target ? ":" + target : "");
}

export function clientLabel(client: string): string {
  return CLIENTS.find((item) => item.key === client)?.label ?? client;
}

export function sortMcpServers(items: AiMcpServerItem[]): AiMcpServerItem[] {
  return [...items].sort((left, right) => {
    const leftRemote = isRemoteType(left.type);
    const rightRemote = isRemoteType(right.type);
    if (leftRemote !== rightRemote) return leftRemote ? -1 : 1;
    return left.name.localeCompare(right.name);
  });
}

export function oauthMeta(status?: AiMcpOAuthStatus): {
  label: string;
  tone: string;
  icon: React.ReactNode;
} {
  if (!status) {
    return { label: "检测中", tone: "mute", icon: <ReloadOutlined spin /> };
  }
  if (status.oauthSupported === false) {
    return { label: "无需 OAuth", tone: "mute", icon: <CheckCircleOutlined /> };
  }
  if (status.oauthSupported === null) {
    return { label: "检测失败", tone: "warn", icon: <DisconnectOutlined /> };
  }
  if (!status.authorized) {
    return { label: "未授权", tone: "info", icon: <KeyOutlined /> };
  }
  if (status.needsReauth || status.expired) {
    return { label: "需重新授权", tone: "err", icon: <DisconnectOutlined /> };
  }
  if (status.refreshFailed) {
    return { label: "刷新失败", tone: "warn", icon: <DisconnectOutlined /> };
  }
  if (status.expiringSoon) {
    return { label: "即将过期", tone: "warn", icon: <KeyOutlined /> };
  }
  return { label: "已授权", tone: "ok", icon: <CheckCircleOutlined /> };
}

export function oauthTooltip(status?: AiMcpOAuthStatus): string {
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
