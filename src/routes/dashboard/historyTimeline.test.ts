import { describe, expect, it } from "vitest";
import { alignHistory, historyTimeline } from "./historyTimeline";

describe("hardware history time alignment", () => {
  it("uses actual elapsed time and bounds the chart window", () => {
    const timeline = historyTimeline([{ timestampMs: 0 }, { timestampMs: 120_000 }]);
    expect(timeline).toHaveLength(60);
    expect(timeline[0]).toBe(61_000);
    expect(timeline[59]).toBe(120_000);
  });
  it("does not stretch a slow or stalled source across the full chart", () => {
    const samples = [{ timestampMs: 2000, value: 10 }, { timestampMs: 4000, value: 20 }];
    expect(alignHistory(samples, [1000, 2000, 3000, 4000, 10000], 5000).map((s) => s?.value ?? null))
      .toEqual([null, 10, 10, 20, null]);
  });
  it("handles empty and single-point startup history", () => {
    expect(historyTimeline([])).toEqual([]);
    expect(historyTimeline([{ timestampMs: 42 }])).toEqual([42]);
  });
});
