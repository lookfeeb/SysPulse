import { create } from "zustand";
import type { DailyTraffic, HistoryQuery } from "@/bindings";
import { queryTrafficHistory } from "@/ipc";
import { errorMessage } from "@/utils/errorMessage";

interface HistoryState {
  query: HistoryQuery | null;
  rows: DailyTraffic[];
  loading: boolean;
  refreshing: boolean;
  error: string | null;
  updatedAt: number | null;
  load: (query: HistoryQuery, silent?: boolean) => Promise<boolean>;
  refresh: () => Promise<boolean>;
}

let revision = 0;

export const useHistoryStore = create<HistoryState>((set, get) => ({
  query: null, rows: [], loading: false, refreshing: false, error: null, updatedAt: null,
  load: async (query, silent = false) => {
    if (silent && (get().loading || get().refreshing)) return false;
    const startedRevision = ++revision;
    set({ loading: !silent, refreshing: silent, error: silent ? get().error : null });
    try {
      const rows = await queryTrafficHistory(query);
      if (startedRevision !== revision) return false;
      set({ query, rows: [...rows].sort((a, b) => b.date.localeCompare(a.date)), updatedAt: Date.now(), error: null });
      return true;
    } catch (e: unknown) {
      if (startedRevision === revision) set({ error: errorMessage(e) });
      return false;
    } finally {
      if (startedRevision === revision) set({ loading: false, refreshing: false });
    }
  },
  refresh: () => {
    const query = get().query;
    return query ? get().load(query, true) : Promise.resolve(false);
  },
}));

export function trafficTotals(rows: DailyTraffic[]) {
  const totals = rows.reduce((sum, row) => ({
    recv: sum.recv + row.bytesRecv,
    sent: sum.sent + row.bytesSent,
  }), { recv: 0, sent: 0 });
  return { ...totals, total: totals.recv + totals.sent };
}
