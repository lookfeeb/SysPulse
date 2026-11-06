import { beforeEach, describe, expect, it, vi } from "vitest";
import { commands } from "@/bindings";
import { useAutostartStore } from "./autostartStore";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

describe("autostart state", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useAutostartStore.setState(useAutostartStore.getInitialState());
  });

  it("keeps the initial state unknown and deduplicates concurrent reads", async () => {
    const pending = deferred<boolean>();
    const read = vi.spyOn(commands, "autostartIsEnabled").mockReturnValue(pending.promise);
    const first = useAutostartStore.getState().refresh();
    const second = useAutostartStore.getState().refresh();
    expect(first).toBe(second);
    expect(useAutostartStore.getState()).toMatchObject({ enabled: null, loading: true });
    pending.resolve(true);
    await first;
    expect(read).toHaveBeenCalledTimes(1);
    expect(useAutostartStore.getState()).toMatchObject({ enabled: true, loading: false });
  });

  it.each([true, false])("does not animate a confirmed %s switch during navigation reads", async (enabled) => {
    useAutostartStore.setState({ enabled });
    const pending = deferred<boolean>();
    vi.spyOn(commands, "autostartIsEnabled").mockReturnValue(pending.promise);
    const task = useAutostartStore.getState().refresh();
    expect(useAutostartStore.getState()).toMatchObject({ enabled, loading: false });
    pending.resolve(enabled);
    await task;
    expect(useAutostartStore.getState()).toMatchObject({ enabled, loading: false });
  });

  it("ignores a stale enabled result after the user disables startup", async () => {
    useAutostartStore.setState({ enabled: true });
    const oldRead = deferred<boolean>();
    vi.spyOn(commands, "autostartIsEnabled").mockReturnValueOnce(oldRead.promise).mockResolvedValue(false);
    const disable = vi.spyOn(commands, "autostartDisable").mockResolvedValue(null);
    const oldTask = useAutostartStore.getState().refresh();
    await Promise.resolve();
    const change = useAutostartStore.getState().setEnabled(false);
    expect(useAutostartStore.getState().refresh()).toBe(change);
    expect(useAutostartStore.getState().setEnabled(false)).toBe(change);
    await change;
    oldRead.resolve(true);
    await oldTask;
    expect(disable).toHaveBeenCalledTimes(1);
    expect(useAutostartStore.getState()).toMatchObject({ enabled: false, loading: false });
  });

  it("preserves confirmed state on read failure and recovers on retry", async () => {
    useAutostartStore.setState({ enabled: false });
    vi.spyOn(commands, "autostartIsEnabled").mockRejectedValueOnce(new Error("read failed")).mockResolvedValue(true);
    await expect(useAutostartStore.getState().refresh()).rejects.toThrow("read failed");
    expect(useAutostartStore.getState()).toMatchObject({ enabled: false, loading: false, error: "read failed" });
    await useAutostartStore.getState().refresh();
    expect(useAutostartStore.getState()).toMatchObject({ enabled: true, loading: false, error: null });
  });
});
