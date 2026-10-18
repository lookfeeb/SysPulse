import { describe, expect, it, vi } from "vitest";
import { createEventBinder } from "./bindOnce";

describe("application listener registration", () => {
  it("coalesces concurrent callers and keeps the successful registration", async () => {
    let finish!: () => void;
    const register = vi.fn(() => new Promise<void>((resolve) => { finish = resolve; }));
    const bind = createEventBinder(register);
    const first = bind();
    const second = bind();
    expect(register).toHaveBeenCalledTimes(1);
    finish();
    await Promise.all([first, second]);
    await bind();
    expect(register).toHaveBeenCalledTimes(1);
  });

  it("reports failure to waiting callers and permits one shared retry", async () => {
    const error = new Error("registration failed");
    const register = vi.fn().mockRejectedValueOnce(error).mockResolvedValue(undefined);
    const bind = createEventBinder(register);
    const results = await Promise.allSettled([bind(), bind()]);
    expect(results).toEqual([
      { status: "rejected", reason: error },
      { status: "rejected", reason: error },
    ]);
    await Promise.all([bind(), bind()]);
    await bind();
    expect(register).toHaveBeenCalledTimes(2);
  });
});
