import { describe, expect, it } from "vitest";
import type { AiSessionSummary } from "@/bindings";
import { buildWorkspaceViews, nextSessionAfterRemoval, resolveTreeSelection } from "./sessionNavigation";
import { sessionDisplayTitle } from "./sessionPresentation";

const session = (sessionId: string, workspaceHash = "a", title = "会话", source = "codex"): AiSessionSummary => ({
  sessionId, workspaceHash, title, source, sessionType: source, workspaceDirectory: `C:/Project/${workspaceHash}`,
  messageCount: 1, fileSize: 10, createdAt: null, modifiedAt: null,
});

describe("session sidebar navigation", () => {
  it("displays the title after a copied thread URL and retains a standalone link", () => {
    const link = "codex://threads/01a14a07-57f0-7ff0-adcd-58f5f2da1011";
    expect(sessionDisplayTitle(link + "检查请求")).toBe("检查请求");
    expect(sessionDisplayTitle(link)).toBe(link);
  });
  it("orders mixed second and millisecond timestamps consistently", () => {
    const a = {...session("a"), modifiedAt:1_700_000_100};
    const b = {...session("b"), modifiedAt:1_700_000_000_000};
    expect(buildWorkspaceViews({workspaces:["a"],sessionsByWorkspace:{a:[b,a]}})[0].sessions.map(s=>s.sessionId)).toEqual(["a","b"]);
  });
  it("keeps an empty project selected after a refresh", () => {
    const workspaces = buildWorkspaceViews({ workspaces: ["a", "b"], sessionsByWorkspace: { a: [], b: [session("b1", "b")] } });
    expect(resolveTreeSelection(workspaces, "a", null)).toEqual({ workspace: workspaces[0], summary: null });
  });
  it("keeps selection within the active project, falling back only if that project disappeared", () => {
    const a = session("a1"), b = session("b1", "b");
    const workspaces = buildWorkspaceViews({ workspaces: ["a", "b"], sessionsByWorkspace: { a: [a], b: [b] } });
    expect(resolveTreeSelection(workspaces, "b", a).summary).toEqual(b);
    expect(resolveTreeSelection(workspaces, "missing", b).summary).toEqual(a);
  });
  it("chooses the adjacent remaining session instead of jumping to the first", () => {
    const sessions = [session("1"), session("2"), session("3")];
    expect(nextSessionAfterRemoval(sessions, sessions[1])).toEqual(sessions[2]);
    expect(nextSessionAfterRemoval(sessions, sessions[2])).toEqual(sessions[1]);
    expect(nextSessionAfterRemoval([sessions[0]], sessions[0])).toBeNull();
  });
});
