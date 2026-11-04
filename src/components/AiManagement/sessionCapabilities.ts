import type { AiSessionSummary } from "@/bindings";

type SessionIdentity = Pick<AiSessionSummary, "sessionId" | "source">;

export function isReadOnlySession(summary: SessionIdentity): boolean {
  if (!["ide", "cli", "claude", "antigravity", "antigravity-ide", "gemini"].includes(summary.source)) return true;
  if (!summary.sessionId.startsWith("@")) return false;
  if (summary.source === "claude") {
    return !summary.sessionId.startsWith("@history/");
  }
  return true;
}

export function canRevealSession(summary: SessionIdentity): boolean {
  return summary.source !== "codex" && !summary.sessionId.startsWith("@");
}
