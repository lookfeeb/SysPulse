import { describe, expect, it } from "vitest";
import { ExclusiveActionLock } from "./mcpActionLock";

describe("MCP 冲突操作锁", () => {
  it("只允许一个配置或授权操作进入", () => {
    const lock = new ExclusiveActionLock();
    expect(lock.acquire("authorize:codex:notion")).toBe(true);
    expect(lock.acquire("delete:kiro:server")).toBe(false);
    expect(lock.current).toBe("authorize:codex:notion");
  });

  it("错误操作不能释放当前锁，完成后可继续下一项", () => {
    const lock = new ExclusiveActionLock();
    expect(lock.acquire("copy:codex:server:kiro")).toBe(true);
    expect(lock.release("delete:codex:server")).toBe(false);
    expect(lock.acquire("toggle:kiro:server")).toBe(false);
    expect(lock.release("copy:codex:server:kiro")).toBe(true);
    expect(lock.acquire("toggle:kiro:server")).toBe(true);
  });
});
