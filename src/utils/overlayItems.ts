import type { OverlayItem } from "@/bindings";

export const overlayItemOptions: { label: string; value: OverlayItem }[] = [
  { label: "CPU 占用", value: "cpu" },
  { label: "CPU 温度", value: "cpu-temp" },
  { label: "CPU 频率", value: "cpu-freq" },
  { label: "内存占用", value: "mem" },
  { label: "GPU 占用", value: "gpu-usage" },
  { label: "GPU 温度", value: "gpu-temp" },
  { label: "硬盘读写", value: "disk-read" },
  { label: "磁盘温度", value: "disk-temp" },
  { label: "风扇转速", value: "fan-rpm" },
  { label: "主板温度", value: "mb-temp" },
  { label: "网速", value: "net-down" },
];

const validItems = new Set(overlayItemOptions.map((option) => option.value));

export function overlayPairFor(item: OverlayItem): [OverlayItem, OverlayItem] | null {
  if (item === "net-down" || item === "net-up") return ["net-down", "net-up"];
  if (item === "disk-read" || item === "disk-write") return ["disk-read", "disk-write"];
  return null;
}

export function normalizeOverlayItem(item: OverlayItem): OverlayItem {
  return item === "gpu" ? "gpu-usage" : item;
}

export function toDisplayItems(value: unknown): OverlayItem[] {
  if (!Array.isArray(value)) return [];

  const selected: OverlayItem[] = [];
  const push = (item: OverlayItem) => {
    const normalized = normalizeOverlayItem(item);
    const pair = overlayPairFor(normalized);
    const displayItem = pair ? pair[0] : normalized;
    if (validItems.has(displayItem) && !selected.includes(displayItem)) {
      selected.push(displayItem);
    }
  };

  for (const item of value as OverlayItem[]) push(item);
  return selected;
}

export function toConfigItems(value: (string | number)[]): OverlayItem[] {
  const items: OverlayItem[] = [];
  const push = (item: OverlayItem) => {
    if (!items.includes(item)) items.push(item);
  };

  for (const raw of value as OverlayItem[]) {
    const item = normalizeOverlayItem(raw);
    const pair = overlayPairFor(item);
    if (pair) {
      push(pair[0]);
      push(pair[1]);
    } else if (validItems.has(item)) {
      push(item);
    }
  }

  return items;
}

export function expandOrderedItems(value: unknown): OverlayItem[] {
  return toConfigItems(toDisplayItems(value));
}
