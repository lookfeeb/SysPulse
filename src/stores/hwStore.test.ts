import { beforeEach, describe, expect, it, vi } from "vitest";
import type { HwSnapshot } from "@/bindings";

vi.mock("@/ipc", () => ({
  getHwSnapshot: vi.fn(), getHelperStatus: vi.fn(), getFanControlState: vi.fn(),
  subscribeHwUpdate: vi.fn(), subscribeHelperStatus: vi.fn(), subscribeFanControl: vi.fn(),
}));
import { getHwSnapshot, getHelperStatus, getFanControlState } from "@/ipc";
import { useHwStore } from "./hwStore";

const snapshot = (timestampMs: number): HwSnapshot => ({
  timestampMs, cpu: { name: "CPU", packageTempC: 60, perCoreUsage: Array(128).fill(20),
    perCoreTempsC: Array(128).fill(60), totalUsage: 20, frequencyMhz: 3000, powerW: 40 },
  gpus: [], memory: null, disks: [], motherboard: null, fans: [],
});
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((r) => { resolve = r; });
  return { promise, resolve };
};

beforeEach(() => {
  vi.restoreAllMocks();
  vi.clearAllMocks();
  useHwStore.setState({ current: null, history: [], helperStatus: "starting", stale: true, lastReceivedAt: null });
});

describe("hardware snapshots", () => {
  it("retains only chart metrics for the last 60 samples", () => {
    for (let i = 1; i <= 70; i++) useHwStore.getState().setSnapshot(snapshot(i));
    const state = useHwStore.getState();
    expect(state.history).toHaveLength(60);
    expect(state.history[0]).toEqual({ timestampMs: 11, cpuTempC: 60, gpuTempC: null });
    expect(state.current?.cpu?.perCoreUsage).toHaveLength(128);
    expect(JSON.stringify(state.history).length).toBeLessThan(JSON.stringify(Array(60).fill(snapshot(1))).length / 10);
  });

  it("deduplicates prime and update snapshots without extending their freshness", () => {
    useHwStore.getState().setSnapshot(snapshot(1));
    const state = useHwStore.getState();
    useHwStore.getState().setSnapshot(snapshot(1));
    expect(useHwStore.getState()).toBe(state);
  });

  it("expires readings using monotonic receipt time and recovers", () => {
    const clock = vi.spyOn(performance, "now").mockReturnValue(100);
    useHwStore.getState().setSnapshot(snapshot(1));
    clock.mockReturnValue(5101);
    useHwStore.getState().checkFreshness();
    expect(useHwStore.getState().current).toBeNull();
    expect(useHwStore.getState().stale).toBe(true);
    useHwStore.getState().setSnapshot(snapshot(2));
    expect(useHwStore.getState().stale).toBe(false);
  });

  it("invalidates data immediately on helper restart", () => {
    useHwStore.getState().setSnapshot(snapshot(1));
    useHwStore.getState().setHelper({ status: "restarting", reason: "timeout" });
    expect(useHwStore.getState().current).toBeNull();
    expect(useHwStore.getState().stale).toBe(true);
  });

  it("coalesces prime requests and preserves newer events in every channel", async () => {
    const oldSnapshot = deferred<HwSnapshot | null>();
    const oldStatus = deferred<"starting">();
    const oldFan = deferred<ReturnType<typeof useHwStore.getState>["fanControl"]>();
    vi.mocked(getHwSnapshot).mockReturnValue(oldSnapshot.promise);
    vi.mocked(getHelperStatus).mockReturnValue(oldStatus.promise);
    vi.mocked(getFanControlState).mockReturnValue(oldFan.promise);
    const first = useHwStore.getState().prime();
    const second = useHwStore.getState().prime();
    useHwStore.getState().setHelper({ status: "running", reason: null });
    useHwStore.getState().setSnapshot(snapshot(20));
    useHwStore.getState().setFanControl({ fuseHold: true, fuseReason: "stale", maxTempC: null, entries: [] });
    oldSnapshot.resolve(snapshot(10)); oldStatus.resolve("starting");
    oldFan.resolve({ fuseHold: false, fuseReason: null, maxTempC: null, entries: [] });
    await Promise.all([first, second]);
    expect(getHwSnapshot).toHaveBeenCalledTimes(1);
    expect(useHwStore.getState().current?.timestampMs).toBe(20);
    expect(useHwStore.getState().helperStatus).toBe("running");
    expect(useHwStore.getState().fanControl.fuseHold).toBe(true);
  });
});
