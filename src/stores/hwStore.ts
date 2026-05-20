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

const HISTORY_LIMIT = 60;

interface HwState {
  current: HwSnapshot | null;
  history: HwSnapshot[];
  helperStatus: HelperStatus;
  helperReason?: string | null;
  fanControl: FanControlStatus;
  prime: () => Promise<void>;
  setSnapshot: (s: HwSnapshot) => void;
  setHelper: (e: HelperStatusEvent) => void;
  setFanControl: (s: FanControlStatus) => void;
}

export const useHwStore = create<HwState>((set) => ({
  current: null,
  history: [],
  helperStatus: "starting",
  helperReason: undefined,
  fanControl: { fuseHold: false, fuseReason: null, maxTempC: null, entries: [] },
  prime: async () => {
    try {
      const last = await getHwSnapshot();
      if (last) useHwStore.getState().setSnapshot(last);
    } catch {
      /* ignore */
    }
    try {
      const status = await getHelperStatus();
      set({ helperStatus: status });
    } catch {
      /* ignore */
    }
    try {
      const fanControl = await getFanControlState();
      set({ fanControl });
    } catch {
      /* ignore */
    }
  },
  setSnapshot: (s) =>
    set((state) => {
      const history = [...state.history, s];
      if (history.length > HISTORY_LIMIT) history.shift();
      return { current: s, history };
    }),
  setHelper: (e) => set({ helperStatus: e.status, helperReason: e.reason }),
  setFanControl: (s) => set({ fanControl: s }),
}));

export const bindHwEvents = createEventBinder(async () => {
  const unlisteners: Array<() => void> = [];
  try {
    unlisteners.push(await subscribeHwUpdate((s) => useHwStore.getState().setSnapshot(s)));
    unlisteners.push(await subscribeHelperStatus((e) => useHwStore.getState().setHelper(e)));
    unlisteners.push(await subscribeFanControl((s) => useHwStore.getState().setFanControl(s)));
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
