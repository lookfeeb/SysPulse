import { create } from "zustand";
import type { Snapshot } from "@/bindings";
import { subscribeStats, getRealtimeStats } from "@/ipc";

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

let unsub: (() => void) | null = null;
export async function bindLiveEvents() {
  if (unsub) return;
  const fn = await subscribeStats((s) => useLiveStore.getState().push(s));
  unsub = fn;
}
