import type { AiSessionSummary, AiSessionTree } from "@/bindings";
import { sameSession, sourceMeta, timestampDate, workspaceName, workspacePath, type WorkspaceView } from "./sessionPresentation";

export function buildWorkspaceViews(tree: AiSessionTree): WorkspaceView[] {
  return tree.workspaces.map((hash) => {
    const sessions = [...(tree.sessionsByWorkspace[hash] ?? [])].sort((a, b) =>
      (timestampDate(b.modifiedAt)?.getTime() ?? 0) - (timestampDate(a.modifiedAt)?.getTime() ?? 0));
    const path = workspacePath(hash, sessions);
    return { hash, path, name: workspaceName(path), sessions };
  });
}

export function filterSessions(sessions: AiSessionSummary[], query: string, source: string) {
  const terms = query.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  return sessions.filter((summary) => {
    if (source !== "all" && summary.source !== source) return false;
    if (terms.length === 0) return true;
    const text = [summary.title, summary.sessionId, summary.workspaceDirectory, summary.sessionType,
      summary.source, sourceMeta(summary.source).label].join(" ").toLocaleLowerCase();
    return terms.every((term) => text.includes(term));
  });
}

/** An empty active project remains active; never borrow another project's session. */
export function resolveTreeSelection(workspaces: WorkspaceView[], activeHash: string | null, selected: AiSessionSummary | null) {
  const workspace = workspaces.find((w) => w.hash === activeHash) ?? workspaces[0] ?? null;
  return {
    workspace,
    summary: workspace?.sessions.find((s) => sameSession(selected, s)) ?? workspace?.sessions[0] ?? null,
  };
}

export function nextSessionAfterRemoval(sessions: AiSessionSummary[], removed: AiSessionSummary) {
  const index = sessions.findIndex((s) => sameSession(removed, s));
  return index < 0 ? null : sessions[index + 1] ?? sessions[index - 1] ?? null;
}
