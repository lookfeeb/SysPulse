import { create } from "zustand";
import type { Snapshot } from "@/bindings";
import { subscribeStats, getRealtimeStats } from "@/ipc";
import { createEventBinder } from "@/utils/bindOnce";

const HISTORY_LIMIT = 60;

interface LiveState {
  current: Snapshot | null;
  history: Snapshot[];
  push: (s: Snapshot) => void;
  prime: () => Promise<void>;
}

export const useLiveStore = create<LiveState>((set, get) => ({
  current: null,
  history: [],
  push: (s) => {
    const next = [...get().history, s];
    if (next.length > HISTORY_LIMIT) next.shift();
    set({ current: s, history: next });
  },
  prime: async () => {
    try {
      const last = await getRealtimeStats();
      if (last) get().push(last);
    } catch {
      /* ignore */
    }
  },
}));

export const bindLiveEvents = createEventBinder(() =>
  subscribeStats((s) => useLiveStore.getState().push(s)),
);
