import { describe, expect, it, vi } from "vitest";
import { disposeAsyncListeners } from "./disposeAsyncListeners";

describe("asynchronous event cleanup", () => {
  it("releases listeners registered before and after unmount exactly once", async () => {
    let finish!: (cleanup: () => void) => void;
    const late = new Promise<() => void>((resolve) => { finish = resolve; });
    const earlyCleanup = vi.fn();
    const lateCleanup = vi.fn();
    const dispose = disposeAsyncListeners([Promise.resolve(earlyCleanup), late], vi.fn());
    await Promise.resolve();
    dispose();
    finish(lateCleanup);
    await Promise.resolve();
    dispose();
    expect(earlyCleanup).toHaveBeenCalledTimes(1);
    expect(lateCleanup).toHaveBeenCalledTimes(1);
  });
  it("reports a failed registration while retaining cleanup for successful ones", async () => {
    const cleanup = vi.fn();
    const onError = vi.fn();
    const error = new Error("IPC failed");
    const dispose = disposeAsyncListeners([Promise.reject(error), Promise.resolve(cleanup)], onError);
    await Promise.resolve();
    await Promise.resolve();
    dispose();
    expect(onError).toHaveBeenCalledWith(error);
    expect(cleanup).toHaveBeenCalledTimes(1);
  });
});
