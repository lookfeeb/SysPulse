import type { Dayjs } from "dayjs";
import type { HistoryQuery } from "@/bindings";

export type HistoryRange = [Dayjs | null, Dayjs | null];

export function historyQueryFromRange(range: HistoryRange, granularity: "day" | "month"): HistoryQuery {
  const [from, to] = range;
  return {
    from: from ? (granularity === "month" ? from.startOf("month") : from).format("YYYY-MM-DD") : null,
    to: to ? (granularity === "month" ? to.endOf("month") : to).format("YYYY-MM-DD") : null,
    granularity,
    iface: null,
  };
}

export function historyRangeLabel(query: HistoryQuery): string {
  if (query.from && query.to) return `${query.from} 至 ${query.to}`;
  if (query.from) return `${query.from} 起`;
  if (query.to) return `截至 ${query.to}`;
  return "全部历史记录";
}
