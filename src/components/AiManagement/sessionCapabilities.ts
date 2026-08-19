import type { AiSessionSummary } from "@/bindings";

type SessionIdentity = Pick<AiSessionSummary, "sessionId" | "source">;

export function isCodexIndexSession(summary: SessionIdentity): boolean {
  return summary.source === "codex" && summary.sessionId.startsWith("@index/");
}

export function isCodexArchiveSession(summary: SessionIdentity): boolean {
  return summary.source === "codex" && summary.sessionId.startsWith("@archive/");
}

export function isReadOnlySession(summary: SessionIdentity): boolean {
  if (summary.source === "antigravity-backup") return true;
  if (!summary.sessionId.startsWith("@")) return false;
  if (summary.source === "codex") {
    return !isCodexIndexSession(summary) && !isCodexArchiveSession(summary);
  }
  if (summary.source === "claude") {
    return !summary.sessionId.startsWith("@history/");
  }
  return true;
}

export function canRevealSession(summary: SessionIdentity): boolean {
  return !summary.sessionId.startsWith("@") || isCodexArchiveSession(summary);
}
