import { describe, expect, it } from "vitest";
import {
  canRevealSession,
  isReadOnlySession,
} from "./sessionCapabilities";

const session = (source: string, sessionId: string) => ({ source, sessionId });

describe("AI 会话操作能力", () => {
  it("不再允许操作旧的 Codex 会话记录", () => {
    for (const id of ["@index/thread", "@archive/thread", "2026/rollout.jsonl"]) {
      expect(isReadOnlySession(session("codex", id))).toBe(true);
      expect(canRevealSession(session("codex", id))).toBe(false);
    }
  });


  it("保留真正只读的备份和暂不支持删除的虚拟索引", () => {
    expect(isReadOnlySession(session("antigravity-backup", "conversation.pb"))).toBe(true);
    expect(isReadOnlySession(session("antigravity", "@summary/thread"))).toBe(true);
  });

  it("允许清理 Claude 历史索引和普通正文", () => {
    expect(isReadOnlySession(session("claude", "@history/thread"))).toBe(false);
    expect(isReadOnlySession(session("claude", "project/thread.jsonl"))).toBe(false);
  });
});
