import { describe, expect, it } from "vitest";
import {
  canRevealSession,
  isCodexArchiveSession,
  isCodexIndexSession,
  isReadOnlySession,
} from "./sessionCapabilities";

const session = (source: string, sessionId: string) => ({ source, sessionId });

describe("AI 会话操作能力", () => {
  it("允许清理 Codex 残留索引，但不允许定位不存在的正文", () => {
    const value = session("codex", "@index/11111111-1111-4111-8111-111111111111");
    expect(isCodexIndexSession(value)).toBe(true);
    expect(isReadOnlySession(value)).toBe(false);
    expect(canRevealSession(value)).toBe(false);
  });

  it("允许删除和定位 Codex 归档正文", () => {
    const value = session("codex", "@archive/2026%2Frollout.jsonl");
    expect(isCodexArchiveSession(value)).toBe(true);
    expect(isReadOnlySession(value)).toBe(false);
    expect(canRevealSession(value)).toBe(true);
  });

  it("保留真正只读的备份和暂不支持删除的虚拟索引", () => {
    expect(isReadOnlySession(session("antigravity-backup", "conversation.pb"))).toBe(true);
    expect(isReadOnlySession(session("antigravity", "@summary/thread"))).toBe(true);
  });

  it("允许清理 Claude 历史索引和普通正文", () => {
    expect(isReadOnlySession(session("claude", "@history/thread"))).toBe(false);
    expect(isReadOnlySession(session("codex", "2026/rollout.jsonl"))).toBe(false);
  });
});
