import type { AiHistoryItem, AiSessionSummary } from "@/bindings";

const SOURCE_META: Record<string, { label: string; color: string }> = {
  ide: { label: "Kiro IDE", color: "blue" },
  cli: { label: "Kiro CLI", color: "purple" },
  codex: { label: "Codex", color: "green" },
  claude: { label: "Claude", color: "orange" },
  antigravity: { label: "Gemini", color: "cyan" },
  "antigravity-backup": { label: "AG 备份", color: "geekblue" },
  "antigravity-ide": { label: "AG IDE", color: "processing" },
  gemini: { label: "Gemini CLI", color: "magenta" },
};

// Windows 文件名保留字符；控制字符另行按码点过滤，避免源码里出现裸控制字节。
const RESERVED_FILENAME_CHARS = /[<>:"/\\|?*]/g;

export { errorMessage } from "@/utils/errorMessage";

export function sourceMeta(source: string): { label: string; color: string } {
  return SOURCE_META[source] ?? { label: source || "未知来源", color: "default" };
}

function cleanTitle(value?: string): string {
  const clean = (value ?? "")
    .replace(/<\/?[a-zA-Z][^>]*>?/g, " ")
    .replace(/&[a-zA-Z#0-9]+;/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!clean) return "未命名会话";
  return clean.length > 100 ? clean.slice(0, 100) + "…" : clean;
}

/** Hide a copied thread-link prefix only when a readable title follows it. */
export function sessionDisplayTitle(value: string): string {
  const withoutLink = value.trim().replace(/^codex:\/\/threads\/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\s*/i, "");
  return cleanTitle(withoutLink || value);
}

export function timestampDate(timestamp: number | null): Date | null {
  if (!timestamp) return null;
  const milliseconds = timestamp > 10_000_000_000 ? timestamp : timestamp * 1000;
  const date = new Date(milliseconds);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function formatTime(timestamp: number | null): string {
  const date = timestampDate(timestamp);
  if (!date) return "时间未知";
  return date.toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

export function formatClock(timestamp: number | null): string {
  const date = timestampDate(timestamp);
  if (!date) return "--:--";
  const today = new Date();
  if (date.toDateString() !== today.toDateString()) {
    return date.toLocaleDateString("zh-CN", {
      ...(date.getFullYear() !== today.getFullYear() ? { year: "numeric" as const } : {}), month: "2-digit", day: "2-digit",
    });
  }
  return date.toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

export function formatFileSize(bytes: number): string {
  if (bytes < 1024) return bytes + " B";
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + " KB";
  return (bytes / (1024 * 1024)).toFixed(1) + " MB";
}

function decodeWorkspacePath(hash: string): string {
  const knownPrefix = Object.keys(SOURCE_META).find((source) => hash.startsWith(source + ":"));
  if (knownPrefix) return hash.slice(knownPrefix.length + 1);
  try {
    return atob(hash.replace(/_+$/, ""));
  } catch {
    return hash;
  }
}

export function workspacePath(hash: string, sessions: AiSessionSummary[]): string {
  return sessions.find((session) => session.workspaceDirectory)?.workspaceDirectory
    || decodeWorkspacePath(hash);
}

export function workspaceName(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) || path || "未知工作区";
}

export function historyText(item: AiHistoryItem): string {
  return item.message.content.map((content) => content.text).filter(Boolean).join("\n\n");
}

export function safeFileName(value: string): string {
  const clean = Array.from(cleanTitle(value))
    .map((char) => (char.charCodeAt(0) < 32 ? "_" : char))
    .join("")
    .replace(RESERVED_FILENAME_CHARS, "_")
    .trim();
  return (clean || "ai-session").slice(0, 80);
}

export function sameSession(left: AiSessionSummary | null, right: AiSessionSummary): boolean {
  return !!left
    && left.workspaceHash === right.workspaceHash
    && left.sessionId === right.sessionId;
}

export function roleLabel(role: string): string {
  if (role === "user") return "用户";
  if (role === "assistant") return "助手";
  if (role === "system") return "系统";
  return role || "系统";
}

export function sourceMonogram(source: string): string {
  const label = sourceMeta(source).label;
  const words = label.match(/[A-Za-z]+/g) ?? [];
  if (words.length >= 2) {
    return ((words[0]?.[0] ?? "") + (words[1]?.[0] ?? "")).toUpperCase();
  }
  if (words.length === 1) return words[0].slice(0, 2).toUpperCase();
  return label.slice(0, 2).toUpperCase();
}

export function dateGroup(timestamp: number | null): "today" | "yesterday" | "earlier" {
  const date = timestampDate(timestamp);
  if (!date) return "earlier";
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const value = new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
  if (value === today) return "today";
  if (value === today - 86_400_000) return "yesterday";
  return "earlier";
}

export const SESSION_GROUP_META = {
  today: "今天",
  yesterday: "昨天",
  earlier: "更早",
} as const;

export interface WorkspaceView {
  hash: string;
  path: string;
  name: string;
  sessions: AiSessionSummary[];
}

export interface SessionGroupView {
  key: keyof typeof SESSION_GROUP_META;
  label: string;
  sessions: AiSessionSummary[];
}
