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

let bound = false;
export async function bindHwEvents() {
  if (bound) return;
  bound = true;
  await subscribeHwUpdate((s) => useHwStore.getState().setSnapshot(s));
  await subscribeHelperStatus((e) => useHwStore.getState().setHelper(e));
  await subscribeFanControl((s) => useHwStore.getState().setFanControl(s));
}
