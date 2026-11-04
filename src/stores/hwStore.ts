import { create } from "zustand";
import type {
  FanControlStatus,
  HwSnapshot,
  HelperStatus,
  HelperStatusEvent,
} from "@/bindings";
import {
  getFanControlState,
  getHwSnapshot,
  getHelperStatus,
  subscribeFanControl,
  subscribeHwUpdate,
  subscribeHelperStatus,
} from "@/ipc";
import { createEventBinder } from "@/utils/bindOnce";
import { SENSOR_DISPLAY_MS, SENSOR_FRESH_MS, sensorAge, visibleHardware } from "@/utils/hardwareFreshness";

const HISTORY_LIMIT = 60;
export interface HwHistoryPoint {
  timestampMs: number;
  cpuTempC: number | null;
  gpuTempC: number | null;
}
let primeInFlight: Promise<void> | null = null;
let helperRevision = 0;
let fanRevision = 0;
let snapshotRevision = 0;

interface HwState {
  current: HwSnapshot | null;
  history: HwHistoryPoint[];
  stale: boolean;
  storageStale: boolean;
  lastReceivedAt: number | null;
  helperStatus: HelperStatus;
  helperReason?: string | null;
  fanControl: FanControlStatus;
  prime: () => Promise<void>;
  setSnapshot: (s: HwSnapshot) => void;
  setHelper: (e: HelperStatusEvent) => void;
  setFanControl: (s: FanControlStatus) => void;
  checkFreshness: () => void;
}

export const useHwStore = create<HwState>((set) => ({
  current: null,
  history: [],
  stale: true,
  storageStale: true,
  lastReceivedAt: null,
  helperStatus: "starting",
  helperReason: undefined,
  fanControl: { fuseHold: false, fuseReason: null, maxTempC: null, entries: [] },
  prime: () => {
    if (primeInFlight) return primeInFlight;
    const revisions = [snapshotRevision, helperRevision, fanRevision];
    primeInFlight = Promise.allSettled([
      getHwSnapshot().then((last) => {
        if (last && snapshotRevision === revisions[0]) useHwStore.getState().setSnapshot(last);
      }),
      getHelperStatus().then((status) => {
        if (helperRevision === revisions[1]) useHwStore.getState().setHelper({ status, reason: null });
      }),
      getFanControlState().then((status) => {
        if (fanRevision === revisions[2]) useHwStore.getState().setFanControl(status);
      }),
    ]).then(() => {}).finally(() => { primeInFlight = null; });
    return primeInFlight;
  },
  setSnapshot: (s) =>
    set((state) => {
      snapshotRevision++;
      const repeated = s.timestampMs === state.current?.timestampMs;
      const temps = (s.gpus ?? []).map((gpu) => gpu.tempC).filter((v): v is number => v != null && Number.isFinite(v));
      const history = repeated ? state.history : [...state.history.slice(-(HISTORY_LIMIT - 1)), {
        timestampMs: s.timestampMs, cpuTempC: s.cpu?.packageTempC ?? null,
        gpuTempC: temps.length ? Math.max(...temps) : null,
      }];
      return { current: visibleHardware(s), history,
        stale: sensorAge(s.sampleAgeMs) > SENSOR_FRESH_MS,
        storageStale: sensorAge(s.storageAgeMs) > SENSOR_FRESH_MS,
        lastReceivedAt: performance.now() };
    }),
  setHelper: (e) => {
    helperRevision++;
    if (e.status !== "running") snapshotRevision++;
    set({ helperStatus: e.status, helperReason: e.reason,
      ...(e.status !== "running" ? { current: null, stale: true, storageStale: true, lastReceivedAt: null } : {}),
    });
  },
  setFanControl: (s) => { fanRevision++; set({ fanControl: s }); },
  checkFreshness: () => set((state) => {
    if (state.lastReceivedAt == null) return state;
    const elapsed = performance.now() - state.lastReceivedAt;
    const stale = sensorAge(state.current?.sampleAgeMs, elapsed) > SENSOR_FRESH_MS;
    const storageStale = sensorAge(state.current?.storageAgeMs, elapsed) > SENSOR_FRESH_MS;
    if (elapsed > SENSOR_DISPLAY_MS) {
      snapshotRevision++;
      return { current: null, stale: true, storageStale: true, lastReceivedAt: null };
    }
    return { stale, storageStale, current: state.current && visibleHardware(state.current, elapsed) };
  }),
}));

export const bindHwEvents = createEventBinder(async () => {
  const unlisteners: Array<() => void> = [];
  try {
    unlisteners.push(await subscribeHwUpdate((s) => useHwStore.getState().setSnapshot(s)));
    unlisteners.push(await subscribeHelperStatus((e) => useHwStore.getState().setHelper(e)));
    unlisteners.push(await subscribeFanControl((s) => useHwStore.getState().setFanControl(s)));
    const timer = setInterval(() => useHwStore.getState().checkFreshness(), 1000);
    unlisteners.push(() => clearInterval(timer));
    return unlisteners;
  } catch (error) {
    for (const unlisten of unlisteners) {
      try {
        unlisten();
      } catch {
        // Ignore cleanup errors before retrying the bind.
      }
    }
    throw error;
  }
});
