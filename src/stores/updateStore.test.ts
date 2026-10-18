import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { commands, type UpdateDownloadEvent } from "@/bindings";
import { useUpdateStore } from "./updateStore";

vi.mock("@tauri-apps/plugin-updater", () => ({ check: vi.fn() }));
vi.mock("@/bindings", () => ({ commands: { downloadInstallUpdate: vi.fn() } }));
vi.mock("@tauri-apps/api/core", () => ({ Channel: class { onmessage = (_event: unknown) => {}; } }));

const deferred = <T>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};
const makeUpdate = () => ({ rid: 42, version: "0.2.0", body: "notes", close: vi.fn().mockResolvedValue(undefined) } as unknown as Update);

beforeEach(() => { vi.clearAllMocks(); useUpdateStore.setState(useUpdateStore.getInitialState()); });
afterEach(() => { vi.useRealTimers(); });

describe("update workflow", () => {
  it("deduplicates checks and preserves the previous handle when a recheck fails", async () => {
    const job = deferred<Update | null>();
    const previous = makeUpdate();
    useUpdateStore.setState({ pending: previous });
    vi.mocked(check).mockReturnValue(job.promise);
    const first = useUpdateStore.getState().checkForUpdate();
    const second = useUpdateStore.getState().checkForUpdate();
    expect(check).toHaveBeenCalledTimes(1);
    job.reject(new Error("offline"));
    await Promise.all([first, second]);
    expect(useUpdateStore.getState().pending).toBe(previous);
    expect(previous.close).not.toHaveBeenCalled();
    expect(useUpdateStore.getState().status).toMatchObject({ kind: "error", operation: "check" });
  });

  it("closes a replaced native resource once", async () => {
    const previous = makeUpdate();
    const next = makeUpdate();
    useUpdateStore.setState({ pending: previous });
    vi.mocked(check).mockResolvedValue(next);
    await useUpdateStore.getState().checkForUpdate();
    expect(previous.close).toHaveBeenCalledTimes(1);
    expect(next.close).not.toHaveBeenCalled();
  });

  it("deduplicates installs, guards checks, and waits for actual installation", async () => {
    const update = makeUpdate();
    const job = deferred<null>();
    let emit!: (event: UpdateDownloadEvent) => void;
    useUpdateStore.setState({ pending: update });
    vi.mocked(commands.downloadInstallUpdate).mockImplementation((_rid, channel) => {
      emit = channel.onmessage; return job.promise;
    });
    const first = useUpdateStore.getState().downloadAndInstall();
    const second = useUpdateStore.getState().downloadAndInstall();
    await Promise.resolve();
    await useUpdateStore.getState().checkForUpdate();
    expect(commands.downloadInstallUpdate).toHaveBeenCalledTimes(1);
    expect(check).not.toHaveBeenCalled();
    emit({ event: "Started", data: { contentLength: null } });
    emit({ event: "Progress", data: { downloaded: 1024, contentLength: null } });
    expect(useUpdateStore.getState().status).toMatchObject({ kind: "downloading", progress: null, downloaded: 1024 });
    emit({ event: "Verifying" });
    expect(useUpdateStore.getState().status.kind).toBe("verifying");
    emit({ event: "Installing" });
    expect(useUpdateStore.getState().status.kind).toBe("installing");
    job.resolve(null);
    await Promise.all([first, second]);
    expect(useUpdateStore.getState().status.kind).toBe("ready");
    expect(update.close).toHaveBeenCalledTimes(1);
  });

  it("keeps the handle for a failed download retry and resets fallback progress", async () => {
    const update = makeUpdate();
    useUpdateStore.setState({ pending: update });
    vi.mocked(commands.downloadInstallUpdate).mockImplementationOnce(async (_rid, channel) => {
      channel.onmessage({ event: "Progress", data: { downloaded: 4096, contentLength: 8192 } });
      channel.onmessage({ event: "Started", data: { contentLength: null } });
      expect(useUpdateStore.getState().status).toMatchObject({ kind: "downloading", downloaded: 0, progress: null });
      throw new Error("network failure");
    });
    await useUpdateStore.getState().downloadAndInstall();
    expect(useUpdateStore.getState().status).toMatchObject({ kind: "error", operation: "install" });
    expect(update.close).not.toHaveBeenCalled();
    vi.mocked(commands.downloadInstallUpdate).mockResolvedValueOnce(null);
    await useUpdateStore.getState().downloadAndInstall();
    expect(commands.downloadInstallUpdate).toHaveBeenCalledTimes(2);
    expect(check).not.toHaveBeenCalled();
  });

  it("waits for an in-flight check before installing its resource", async () => {
    const update = makeUpdate();
    const checking = deferred<Update | null>();
    vi.mocked(check).mockReturnValue(checking.promise);
    vi.mocked(commands.downloadInstallUpdate).mockResolvedValue(null);
    const checkJob = useUpdateStore.getState().checkForUpdate();
    const installJob = useUpdateStore.getState().downloadAndInstall();
    checking.resolve(update);
    await Promise.all([checkJob, installJob]);
    expect(commands.downloadInstallUpdate).toHaveBeenCalledWith(42, expect.anything());
    expect(check).toHaveBeenCalledTimes(1);
  });

  it("does not replace a visible update when the delayed startup check runs", async () => {
    vi.useFakeTimers();
    useUpdateStore.getState().startAutoCheck();
    useUpdateStore.setState({ status: { kind: "available", version: "0.2.0", notes: "", checkedAt: 0 }, pending: makeUpdate() });
    await vi.advanceTimersByTimeAsync(60_000);
    expect(check).not.toHaveBeenCalled();
  });
});
