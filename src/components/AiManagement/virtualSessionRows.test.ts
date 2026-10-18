import { describe, expect, it } from "vitest";
import type { AiSessionSummary } from "@/bindings";
import { buildSessionRows, visibleRowRange } from "./virtualSessionRows";

describe("会话虚拟列表", () => {
  it("一万条会话只渲染可见范围，并能访问末尾", () => {
    const sessions = Array.from({ length: 10_000 }, (_, i) => ({ sessionId: String(i), workspaceHash: "codex:test" } as AiSessionSummary));
    const { rows, height } = buildSessionRows([{ key: "today", label: "今天", sessions }]);
    expect(rows).toHaveLength(10_001);
    expect(height).toBe(rows.at(-1)!.top + rows.at(-1)!.height);
    expect(rows.every((row, index) => index === 0 || row.top === rows[index - 1].top + rows[index - 1].height)).toBe(true);
    const first = visibleRowRange(rows, 0, 600);
    expect(first.end - first.start).toBeLessThan(25);
    const last = visibleRowRange(rows, height - 600, 600);
    expect(last.end).toBe(rows.length);
    expect(last.end - last.start).toBeLessThan(25);
  });
  it("空列表和分组边界保持正确", () => {
    expect(visibleRowRange([], 0, 500)).toEqual({ start: 0, end: 0 });
    const { rows } = buildSessionRows([{ key: "today", label: "今天", sessions: [] }, { key: "earlier", label: "更早", sessions: [] }]);
    expect(rows.map((row) => row.top)).toEqual([0, 30]);
  });
});
