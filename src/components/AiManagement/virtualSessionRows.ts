import type { AiSessionSummary } from "@/bindings";
import type { SessionGroupView } from "./sessionPresentation";

export type SessionRow =
  | { key: string; top: number; height: number; group: SessionGroupView; session?: never }
  | { key: string; top: number; height: number; session: AiSessionSummary; group?: never };

export function buildSessionRows(groups: SessionGroupView[]) {
  const rows: SessionRow[] = [];
  let height = 0;
  for (const group of groups) {
    rows.push({ key: "group:" + group.key, top: height, height: 30, group });
    height += 30;
    for (const session of group.sessions) {
      rows.push({ key: session.workspaceHash + ":" + session.sessionId, top: height, height: 60, session });
      height += 60;
    }
  }
  return { rows, height };
}

export function visibleRowRange(rows: SessionRow[], scrollTop: number, viewportHeight: number) {
  const firstAfter = (offset: number) => {
    let low = 0;
    let high = rows.length;
    while (low < high) {
      const middle = (low + high) >>> 1;
      if (rows[middle].top + rows[middle].height <= offset) low = middle + 1;
      else high = middle;
    }
    return low;
  };
  return {
    start: Math.max(0, firstAfter(Math.max(0, scrollTop)) - 4),
    end: Math.min(rows.length, firstAfter(Math.max(0, scrollTop) + viewportHeight) + 5),
  };
}
