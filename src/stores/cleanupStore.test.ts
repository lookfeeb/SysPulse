import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ScanResult } from "@/bindings";

const api = vi.hoisted(() => ({ scanCleanup: vi.fn(), cleanCategories: vi.fn(), cancelCleanupScan: vi.fn() }));
const events = vi.hoisted(() => new Map<string, (event: { payload: unknown }) => void>());
vi.mock("@/bindings", () => ({ commands: api }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async (name, callback) => {
  events.set(name, callback);
  return () => events.delete(name);
}) }));
import { useCleanupStore } from "./cleanupStore";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const scan = (): ScanResult => ({ scanId: "scan-1", categories: [], totalSizeBytes: 1024,
  totalFileCount: 2, scannedAtMs: Date.now(), expiresAtMs: Date.now() + 60_000,
  durationMs: 5, scannedPaths: 2, skippedPaths: 0, ignoredPaths: 0, hotspotCount: 0,
  scoutWorkers: 1, engineerWorkers: 1, scoutTasks: 1, engineerTasks: 1, projectRoots: [],
  warnings: [], complete: true, cancelled: false });
const args = { scanId: "scan-1", categoryIds: ["node-cache"], excludedPaths: [], confirmCaution: true, confirmAdvanced: true };

describe("cleanup application task lifetime", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useCleanupStore.setState(useCleanupStore.getInitialState());
  });

  it("continues scanning without page subscribers and exposes progress/results on return", async () => {
    const request = deferred<ScanResult>();
    api.scanCleanup.mockReturnValue(request.promise);
    const leavePage = useCleanupStore.subscribe(() => {});
    const running = useCleanupStore.getState().startScan(["C:/projects"]);
    await vi.waitFor(() => expect(api.scanCleanup).toHaveBeenCalledOnce());
    leavePage();
    events.get("cleanup:scan-progress")!({ payload: { phase: "deep", scannedPaths: 42 } });
    expect(useCleanupStore.getState().phase).toBe("scanning");
    expect(useCleanupStore.getState().scanProgress?.scannedPaths).toBe(42);
    await useCleanupStore.getState().startScan([]);
    expect(api.scanCleanup).toHaveBeenCalledOnce();
    const result = scan();
    request.resolve(result);
    await running;
    expect(useCleanupStore.getState()).toMatchObject({ phase: "idle", scan: result });
    expect(api.cancelCleanupScan).not.toHaveBeenCalled();
  });

  it("cleans once across page changes and retains the report when refreshing fails", async () => {
    const request = deferred<{ freedBytes: number; deletedFiles: number; errors: string[] }>();
    api.cleanCategories.mockReturnValue(request.promise);
    api.scanCleanup.mockRejectedValue(new Error("refresh failed"));
    useCleanupStore.setState({ scan: scan() });
    const running = useCleanupStore.getState().clean(args, []);
    await useCleanupStore.getState().clean(args, []);
    await vi.waitFor(() => expect(api.cleanCategories).toHaveBeenCalledOnce());
    const result = { freedBytes: 512, deletedFiles: 1, errors: ["locked file"] };
    request.resolve(result);
    await running;
    expect(useCleanupStore.getState()).toMatchObject({ phase: "idle", lastCleanResult: result, consumed: true });
    expect(useCleanupStore.getState().notice?.text).toContain("清理报告已保留");
    expect(api.cleanCategories).toHaveBeenCalledOnce();
  });

  it("keeps post-clean refresh visible as a cancellable scan", async () => {
    const refresh = deferred<ScanResult>();
    api.cleanCategories.mockResolvedValue({ freedBytes: 512, deletedFiles: 1, errors: [] });
    api.scanCleanup.mockReturnValue(refresh.promise);
    api.cancelCleanupScan.mockResolvedValue(true);
    useCleanupStore.setState({ scan: scan() });
    const running = useCleanupStore.getState().clean(args, []);
    await vi.waitFor(() => expect(useCleanupStore.getState().phase).toBe("refreshing"));
    await useCleanupStore.getState().cancelScan();
    expect(api.cancelCleanupScan).toHaveBeenCalledOnce();
    refresh.resolve({ ...scan(), cancelled: true, complete: false });
    await running;
    expect(useCleanupStore.getState().lastCleanResult?.deletedFiles).toBe(1);
    await useCleanupStore.getState().clean(args, []);
    expect(api.cleanCategories).toHaveBeenCalledOnce();
  });

  it("recovers cancellation controls on IPC failure instead of hanging", async () => {
    const request = deferred<ScanResult>();
    api.scanCleanup.mockReturnValue(request.promise);
    api.cancelCleanupScan.mockRejectedValue(new Error("cancel failed"));
    const running = useCleanupStore.getState().startScan([]);
    await vi.waitFor(() => expect(api.scanCleanup).toHaveBeenCalledOnce());
    await useCleanupStore.getState().cancelScan();
    expect(useCleanupStore.getState()).toMatchObject({ phase: "scanning", cancelRequested: false });
    request.resolve(scan());
    await running;
  });

  it("ignores a delayed cancellation failure from an earlier scan", async () => {
    const first = deferred<ScanResult>();
    const second = deferred<ScanResult>();
    const cancel = deferred<boolean>();
    api.scanCleanup.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
    api.cancelCleanupScan.mockReturnValue(cancel.promise);
    const firstRun = useCleanupStore.getState().startScan([]);
    await vi.waitFor(() => expect(api.scanCleanup).toHaveBeenCalledOnce());
    const cancelling = useCleanupStore.getState().cancelScan();
    first.resolve(scan());
    await firstRun;
    const secondRun = useCleanupStore.getState().startScan([]);
    cancel.reject(new Error("old cancellation failed"));
    await cancelling;
    expect(useCleanupStore.getState()).toMatchObject({ phase: "scanning", notice: null });
    second.resolve(scan());
    await secondRun;
  });

  it("clears pending progress on cleanup IPC failure and forbids snapshot reuse", async () => {
    api.cleanCategories.mockRejectedValue(new Error("cleanup failed"));
    useCleanupStore.setState({ scan: scan() });
    await useCleanupStore.getState().clean(args, []);
    expect(useCleanupStore.getState()).toMatchObject({ phase: "idle", consumed: true, cleanProgress: null });
    await useCleanupStore.getState().clean(args, []);
    expect(api.cleanCategories).toHaveBeenCalledOnce();
  });
});
