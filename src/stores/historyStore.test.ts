import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DailyTraffic, HistoryQuery } from "@/bindings";
import * as api from "@/ipc";
import { trafficTotals, useHistoryStore } from "./historyStore";

const query: HistoryQuery = { from: "2026-10-01", to: "2026-11-06", granularity: "day", iface: null };
const rows: DailyTraffic[] = Array.from({ length: 15 }, (_, i) => ({
  date: `2026-10-${String(i + 1).padStart(2, "0")}`, iface: null, bytesRecv: 100, bytesSent: 50,
}));

describe("live history", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    useHistoryStore.setState(useHistoryStore.getInitialState());
  });

  it("replaces the previous date filter with all history and keeps it during refresh", async () => {
    const read = vi.spyOn(api, "queryTrafficHistory").mockResolvedValue(rows);
    await useHistoryStore.getState().load(query);
    const all = { ...query, from: null, to: null };
    await useHistoryStore.getState().load(all);
    await useHistoryStore.getState().refresh();
    expect(useHistoryStore.getState().query).toEqual(all);
    expect(read).toHaveBeenLastCalledWith(all);
  });

  it("totals all records across pages and replaces live results without accumulating twice", async () => {
    vi.spyOn(api, "queryTrafficHistory").mockResolvedValueOnce(rows).mockResolvedValue([...rows, {
      date: "2026-11-06", iface: null, bytesRecv: 1000, bytesSent: 500,
    }]);
    await useHistoryStore.getState().load(query);
    expect(trafficTotals(useHistoryStore.getState().rows)).toEqual({ recv: 1500, sent: 750, total: 2250 });
    await useHistoryStore.getState().refresh();
    await useHistoryStore.getState().refresh();
    expect(useHistoryStore.getState().rows[0].date).toBe("2026-11-06");
    expect(trafficTotals(useHistoryStore.getState().rows)).toEqual({ recv: 2500, sent: 1250, total: 3750 });
  });

  it("does not let a slow refresh overwrite a newer monthly query", async () => {
    useHistoryStore.setState({ query, rows });
    let resolve!: (value: DailyTraffic[]) => void;
    const read = vi.spyOn(api, "queryTrafficHistory")
      .mockReturnValueOnce(new Promise((done) => { resolve = done; }))
      .mockResolvedValueOnce([{ ...rows[0], date: "2026-10", bytesRecv: 1500, bytesSent: 750 }]);
    const refresh = useHistoryStore.getState().refresh();
    expect(useHistoryStore.getState()).toMatchObject({ loading: false, refreshing: true, rows });
    await useHistoryStore.getState().refresh();
    expect(read).toHaveBeenCalledTimes(1);
    const monthly = { ...query, granularity: "month" as const };
    await useHistoryStore.getState().load(monthly);
    resolve(rows);
    await refresh;
    expect(useHistoryStore.getState()).toMatchObject({ query: monthly, loading: false, refreshing: false });
    expect(useHistoryStore.getState().rows.map((row) => row.date)).toEqual(["2026-10"]);
  });

  it("retains the last successful query and totals after failures", async () => {
    vi.spyOn(api, "queryTrafficHistory").mockResolvedValueOnce(rows).mockRejectedValueOnce(new Error("offline")).mockResolvedValue([]);
    await useHistoryStore.getState().load(query);
    const updatedAt = useHistoryStore.getState().updatedAt;
    await useHistoryStore.getState().load({ ...query, from: "2026-11-01" });
    expect(useHistoryStore.getState()).toMatchObject({ query, error: "offline", updatedAt, loading: false });
    expect(trafficTotals(useHistoryStore.getState().rows).total).toBe(2250);
    await useHistoryStore.getState().refresh();
    expect(useHistoryStore.getState()).toMatchObject({ query, rows: [], error: null });
    expect(trafficTotals([]).total).toBe(0);
  });
});
