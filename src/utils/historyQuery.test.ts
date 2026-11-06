import { describe, expect, it } from "vitest";
import dayjs from "dayjs";
import { historyQueryFromRange, historyRangeLabel } from "./historyQuery";

describe("history date filters", () => {
  it.each(["day", "month"] as const)("keeps an empty %s range unbounded", (granularity) => {
    const query = historyQueryFromRange([null, null], granularity);
    expect(query).toEqual({ from: null, to: null, granularity, iface: null });
    expect(historyRangeLabel(query)).toBe("全部历史记录");
  });
  it("expands only selected monthly boundaries, including leap years", () => {
    expect(historyQueryFromRange([dayjs("2024-02-15"), dayjs("2024-02-15")], "month"))
      .toMatchObject({ from: "2024-02-01", to: "2024-02-29" });
    expect(historyQueryFromRange([null, dayjs("2024-02-15")], "month"))
      .toMatchObject({ from: null, to: "2024-02-29" });
    expect(historyQueryFromRange([dayjs("2024-02-15"), null], "month"))
      .toMatchObject({ from: "2024-02-01", to: null });
  });
  it("preserves inclusive daily bounds and describes partial ranges", () => {
    const date = dayjs("2026-11-06");
    expect(historyQueryFromRange([date, date], "day")).toMatchObject({ from: "2026-11-06", to: "2026-11-06" });
    expect(historyRangeLabel(historyQueryFromRange([date, null], "day"))).toBe("2026-11-06 起");
    expect(historyRangeLabel(historyQueryFromRange([null, date], "day"))).toBe("截至 2026-11-06");
  });
});
