import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { fmtBytes, fmtFreq, fmtPct, fmtRpm, fmtSpeed, fmtTemp, maxValid, sumValid } from "@/utils/format";
import { expandOrderedItems, overlayPairFor } from "@/utils/overlayItems";
import type { OverlayItem } from "@/bindings";

interface InterfaceStats {
  bytesSentPerSec: number;
  bytesRecvPerSec: number;
}

interface Snapshot {
  cpu: { usagePercent: number; model?: string; physicalCores?: number };
  memory: {
    usedPercent: number;
    usedBytes?: number;
    totalBytes?: number;
    swapUsedBytes?: number;
    swapTotalBytes?: number;
  };
  network: { total: InterfaceStats };
}

interface DiskHw {
  model?: string | null;
  bus?: string | null;
  tempC: number | null;
  readBytesPerSec: number | null;
  writeBytesPerSec: number | null;
  health?: string | null;
}

interface GpuHw {
  name?: string | null;
  vendor?: string | null;
  usagePercent: number | null;
  tempC: number | null;
  memUsedMb?: number | null;
  memTotalMb?: number | null;
  powerW?: number | null;
  fanRpm?: number | null;
}

interface CpuHw {
  name?: string | null;
  packageTempC: number | null;
  frequencyMhz: number | null;
  perCoreTempsC?: (number | null)[];
  totalUsage?: number | null;
  powerW?: number | null;
}

interface FanHwItem {
  name?: string | null;
  rpm: number | null;
  pwmPercent?: number | null;
}

interface NamedValue {
  name: string;
  value: number;
}

interface MotherboardHw {
  vendor?: string | null;
  model?: string | null;
  temperaturesC?: NamedValue[];
}

interface HwSnapshot {
  cpu?: CpuHw | null;
  gpus?: GpuHw[];
  disks?: DiskHw[];
  motherboard?: MotherboardHw | null;
  fans?: FanHwItem[];
}

interface OverlayConfig {
  items: string[];
}

interface AppInfo {
  configDir: string;
  logsDir: string;
}

function applyConfig(cfg: OverlayConfig, root: HTMLElement) {
  lastFitWidth = 0;
  lastFitHeight = 0;

  const orderedKeys = expandOrderedItems(cfg.items);
  const wanted = new Set<string>(orderedKeys);
  for (const key of orderedKeys) {
    const el = root.querySelector<HTMLElement>(`.ov-item[data-key="${key}"]`);
    if (el) root.appendChild(el);
  }

  // Show/hide items based on config.items
  root.querySelectorAll<HTMLElement>(".ov-item").forEach((el) => {
    const key = el.dataset.key || "";
    if (wanted.has(key)) {
      el.classList.remove("hidden");
    } else {
      el.classList.add("hidden");
    }
  });
  layoutItems(root, orderedKeys);
}
function ensureOverlayRow(root: HTMLElement, className: string): HTMLElement {
  let row = root.querySelector<HTMLElement>(`.${className}`);
  if (!row) {
    row = document.createElement("div");
    row.className = `ov-row ${className}`;
  }
  return row;
}

function ensureOverlayGrid(root: HTMLElement): HTMLElement {
  let grid = root.querySelector<HTMLElement>(".ov-grid");
  if (!grid) {
    grid = document.createElement("div");
    grid.className = "ov-grid";
  }
  return grid;
}

type LayoutUnit =
  | { kind: "single"; item: HTMLElement }
  | { kind: "pair"; first: HTMLElement; second: HTMLElement };

function createPairCell(unit: Extract<LayoutUnit, { kind: "pair" }>): HTMLElement {
  const cell = document.createElement("div");
  cell.className = "ov-pair-cell";
  cell.append(unit.first, unit.second);
  return cell;
}

function layoutItems(root: HTMLElement, orderedKeys: string[]) {
  const allItems = Array.from(root.querySelectorAll<HTMLElement>(".ov-item"));
  const topRow = ensureOverlayRow(root, "ov-row-top");
  const bottomRow = ensureOverlayRow(root, "ov-row-bottom");
  const grid = ensureOverlayGrid(root);
  const stash = document.createDocumentFragment();
  for (const item of allItems) stash.appendChild(item);
  root.querySelectorAll(".ov-pair-cell").forEach((cell) => cell.remove());

  topRow.replaceChildren();
  bottomRow.replaceChildren();
  grid.replaceChildren();
  for (const item of allItems) {
    item.style.gridColumn = "";
    item.style.gridRow = "";
  }

  topRow.remove();
  bottomRow.remove();

  const itemByKey = new Map(
    allItems.map((item) => [item.dataset.key || "", item] as const),
  );
  const handled = new Set<string>();
  const units: LayoutUnit[] = [];
  for (const key of orderedKeys) {
    if (handled.has(key)) continue;
    const pair = overlayPairFor(key as OverlayItem);
    if (pair) {
      const displayPair =
        pair[0] === "net-down" && pair[1] === "net-up"
          ? (["net-up", "net-down"] as const)
          : pair;
      const first = itemByKey.get(displayPair[0]);
      const second = itemByKey.get(displayPair[1]);
      if (first && second) {
        units.push({ kind: "pair", first, second });
        handled.add(pair[0]);
        handled.add(pair[1]);
        continue;
      }
    }

    const item = itemByKey.get(key);
    if (item) {
      units.push({ kind: "single", item });
      handled.add(key);
    }
  }

  const occupied = new Set<string>();
  const nextColumn = (row: 1 | 2, needsBothRows: boolean) => {
    let column = 1;
    while (
      occupied.has(`${row}:${column}`) ||
      (needsBothRows && (occupied.has(`1:${column}`) || occupied.has(`2:${column}`)))
    ) {
      column += 1;
    }
    return column;
  };
  const placeNode = (node: HTMLElement, row: 1 | 2, column: number, spanRows = false) => {
    node.style.gridRow = spanRows ? "1 / 3" : String(row);
    node.style.gridColumn = String(column);
    if (spanRows) {
      occupied.add(`1:${column}`);
      occupied.add(`2:${column}`);
    } else {
      occupied.add(`${row}:${column}`);
    }
    grid.appendChild(node);
  };
  const placeItem = (item: HTMLElement, row: 1 | 2, column: number) => {
    item.style.gridRow = String(row);
    item.style.gridColumn = String(column);
    occupied.add(`${row}:${column}`);
    grid.appendChild(item);
  };
  const placeSingle = (unit: Extract<LayoutUnit, { kind: "single" }>, row: 1 | 2) => {
    const column = nextColumn(row, false);
    placeItem(unit.item, row, column);
  };

  const pairUnits = units.filter(
    (unit): unit is Extract<LayoutUnit, { kind: "pair" }> => unit.kind === "pair",
  );
  const singleUnits = units.filter(
    (unit): unit is Extract<LayoutUnit, { kind: "single" }> => unit.kind === "single",
  );

  for (const unit of pairUnits) {
    const column = nextColumn(1, true);
    placeNode(createPairCell(unit), 1, column, true);
  }

  const topCount = Math.ceil(singleUnits.length / 2);
  const topUnits =
    singleUnits.length % 2 === 1 && singleUnits.length > 3
      ? [...singleUnits.slice(0, topCount - 1), singleUnits[singleUnits.length - 1]]
      : singleUnits.slice(0, topCount);
  const bottomUnits =
    singleUnits.length % 2 === 1 && singleUnits.length > 3
      ? singleUnits.slice(topCount - 1, -1)
      : singleUnits.slice(topCount);

  for (const unit of topUnits) placeSingle(unit, 1);
  for (const unit of bottomUnits) placeSingle(unit, 2);

  grid.classList.toggle("hidden", grid.children.length === 0);
  root.prepend(grid);

  for (const item of allItems) {
    if (!grid.contains(item)) root.appendChild(item);
  }
}

let lastFitWidth = 0;
let lastFitHeight = 0;
let fitTimer: number | null = null;

function autoFitWindow(root: HTMLElement, options: { allowShrink?: boolean } = {}) {
  if (fitTimer != null) {
    window.clearTimeout(fitTimer);
  }
  fitTimer = window.setTimeout(() => {
    requestAnimationFrame(async () => {
      const previousWidth = root.style.width;
      const previousMaxWidth = root.style.maxWidth;
      const previousFlexWrap = root.style.flexWrap;
      const previousHeight = root.style.height;
      const previousOverflow = root.style.overflow;
      // Temporarily remove all constraints so content can expand naturally
      root.style.width = "max-content";
      root.style.maxWidth = "none";
      root.style.flexWrap = "nowrap";
      root.style.height = "auto";
      root.style.overflow = "visible";
      const rect = root.getBoundingClientRect();
      let w = Math.max(80, Math.ceil(Math.max(rect.width, root.scrollWidth)) + 4);
      let h = Math.max(24, Math.ceil(Math.max(rect.height, root.scrollHeight)) + 4);
      if (!options.allowShrink && lastFitWidth > 0) {
        w = Math.max(w, lastFitWidth);
        h = Math.max(h, lastFitHeight);
      }
      root.style.width = previousWidth;
      root.style.maxWidth = previousMaxWidth;
      root.style.flexWrap = previousFlexWrap;
      root.style.height = previousHeight;
      root.style.overflow = previousOverflow;
      if (Math.abs(w - lastFitWidth) < 2 && Math.abs(h - lastFitHeight) < 2) return;
      lastFitWidth = w;
      lastFitHeight = h;
      try {
        await invoke("resize_overlay", { args: { width: w, height: h } });
      } catch {
        /* ignore */
      }
    });
  }, 120);
}

let lastSnapshot: Snapshot | null = null;
let lastHw: HwSnapshot | null = null;

type TooltipContent = { title: string; lines: string[] };
type TooltipBuilder = (s: Snapshot | null, h: HwSnapshot | null) => TooltipContent;

const tooltipBuilders: Record<string, TooltipBuilder> = {
  cpu: (s, h) => {
    const lines: string[] = [];
    const usage = h?.cpu?.totalUsage ?? s?.cpu.usagePercent;
    if (usage != null) lines.push(`当前占用：${fmtPct(usage)}`);
    if (h?.cpu?.name) lines.push(`型号：${h.cpu.name}`);
    else if (s?.cpu.model) lines.push(`型号：${s.cpu.model}`);
    if (s?.cpu.physicalCores) lines.push(`物理核心：${s.cpu.physicalCores}`);
    if (h?.cpu?.powerW != null) lines.push(`功耗：${h.cpu.powerW.toFixed(1)} W`);
    return { title: "CPU 占用", lines };
  },
  "cpu-temp": (_, h) => {
    const lines: string[] = [];
    lines.push(`封装温度：${fmtTemp(h?.cpu?.packageTempC)}`);
    const perCore = h?.cpu?.perCoreTempsC ?? [];
    const cores = perCore.filter((t): t is number => t != null);
    if (cores.length) {
      lines.push(`最高核心：${fmtTemp(Math.max(...cores))}`);
      lines.push(`最低核心：${fmtTemp(Math.min(...cores))}`);
    }
    return { title: "CPU 温度", lines };
  },
  "cpu-freq": (_, h) => ({
    title: "CPU 频率",
    lines: [`当前：${fmtFreq(h?.cpu?.frequencyMhz)}`],
  }),
  mem: (s) => {
    const lines: string[] = [];
    if (s?.memory) {
      lines.push(`占用：${fmtPct(s.memory.usedPercent)}`);
      if (s.memory.usedBytes != null && s.memory.totalBytes != null) {
        lines.push(`已用：${fmtBytes(s.memory.usedBytes)} / ${fmtBytes(s.memory.totalBytes)}`);
      }
      if (s.memory.swapTotalBytes != null && s.memory.swapTotalBytes > 0) {
        lines.push(
          `交换：${fmtBytes(s.memory.swapUsedBytes ?? 0)} / ${fmtBytes(s.memory.swapTotalBytes)}`,
        );
      }
    }
    return { title: "内存", lines };
  },
  "gpu-usage": (_, h) => {
    const gpus = h?.gpus ?? [];
    const lines: string[] = [];
    if (!gpus.length) lines.push("未检测到 GPU");
    for (const g of gpus) {
      const name = g.name || g.vendor || "GPU";
      lines.push(`${name}：${fmtPct(g.usagePercent)}`);
    }
    return { title: "GPU 占用", lines };
  },
  "gpu-temp": (_, h) => {
    const gpus = h?.gpus ?? [];
    const lines: string[] = [];
    if (!gpus.length) lines.push("未检测到 GPU");
    for (const g of gpus) {
      const name = g.name || g.vendor || "GPU";
      lines.push(`${name}：${fmtTemp(g.tempC)}`);
    }
    return { title: "GPU 温度", lines };
  },
  "disk-read": (_, h) => {
    const disks = h?.disks ?? [];
    const lines: string[] = [];
    const totalR = sumValid(disks.map((d) => d.readBytesPerSec));
    const totalW = sumValid(disks.map((d) => d.writeBytesPerSec));
    lines.push(`合计读取：${fmtSpeed(totalR)}`);
    lines.push(`合计写入：${fmtSpeed(totalW)}`);
    for (const d of disks) {
      const model = d.model || "磁盘";
      lines.push(`${model}：↓${fmtSpeed(d.readBytesPerSec)} ↑${fmtSpeed(d.writeBytesPerSec)}`);
    }
    return { title: "硬盘读写", lines };
  },
  "disk-temp": (_, h) => {
    const disks = h?.disks ?? [];
    const lines: string[] = [];
    if (!disks.length) lines.push("未检测到磁盘");
    for (const d of disks) {
      const model = d.model || "磁盘";
      lines.push(`${model}：${fmtTemp(d.tempC)}`);
    }
    return { title: "磁盘温度", lines };
  },
  "fan-rpm": (_, h) => {
    const fans = (h?.fans ?? []).filter((f) => f.rpm != null && f.rpm > 0);
    const lines: string[] = [];
    if (!fans.length) lines.push("未检测到风扇");
    for (const f of fans) {
      const name = f.name || "风扇";
      const pwm = f.pwmPercent != null ? `（${Math.round(f.pwmPercent)}%）` : "";
      lines.push(`${name}：${fmtRpm(f.rpm)}${pwm}`);
    }
    return { title: "风扇转速", lines };
  },
  "mb-temp": (_, h) => {
    const mb = h?.motherboard;
    const temps = mb?.temperaturesC ?? [];
    const lines: string[] = [];
    if (mb?.model) lines.push(`主板：${(mb.vendor || "") + " " + mb.model}`.trim());
    if (!temps.length) lines.push("未检测到温度");
    for (const t of temps) lines.push(`${t.name}：${fmtTemp(t.value)}`);
    return { title: "主板温度", lines };
  },
  "net-down": (s) => buildNetTooltip(s),
  "net-up": (s) => buildNetTooltip(s),
};

function buildNetTooltip(s: Snapshot | null): TooltipContent {
  const total = s?.network.total;
  return {
    title: "网速",
    lines: [
      `下行：${fmtSpeed(total?.bytesRecvPerSec)}`,
      `上行：${fmtSpeed(total?.bytesSentPerSec)}`,
    ],
  };
}

let hoveredKey: string | null = null;

function buildContentFor(key: string): TooltipContent | null {
  const builder = tooltipBuilders[key];
  if (!builder) return null;
  return builder(lastSnapshot, lastHw);
}

async function showTooltipFor(key: string, el: HTMLElement) {
  const content = buildContentFor(key);
  if (!content) return;
  const rect = el.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const anchor = await mapClientToScreen(rect.left + rect.width / 2, rect.top, dpr);
  try {
    await invoke("show_overlay_tooltip", {
      args: {
        title: content.title,
        lines: content.lines,
        anchorX: Math.round(anchor.x),
        anchorY: Math.round(anchor.y),
      },
    });
  } catch (e) {
    console.warn("show_overlay_tooltip failed", e);
  }
}

async function hideTooltip() {
  try {
    await invoke("hide_overlay_tooltip");
  } catch {
    /* ignore */
  }
}

async function mapClientToScreen(clientX: number, clientY: number, dpr: number) {
  // Convert in-window CSS coordinates to physical screen pixels.
  try {
    const pos = await getCurrentWindow().outerPosition();
    return {
      x: pos.x + clientX * dpr,
      y: pos.y + clientY * dpr,
    };
  } catch {
    return { x: clientX * dpr, y: clientY * dpr };
  }
}

function refreshActiveTooltip() {
  if (!hoveredKey) return;
  const el = document.querySelector<HTMLElement>(`.ov-item[data-key="${hoveredKey}"]`);
  if (!el) return;
  const content = buildContentFor(hoveredKey);
  if (!content) return;
  // Only re-emit the payload; position is kept.
  const rect = el.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  void mapClientToScreen(rect.left + rect.width / 2, rect.top, dpr).then((anchor) => {
    void invoke("show_overlay_tooltip", {
      args: {
        title: content.title,
        lines: content.lines,
        anchorX: Math.round(anchor.x),
        anchorY: Math.round(anchor.y),
      },
    });
  });
}

function bindItemHover(root: HTMLElement) {
  root.querySelectorAll<HTMLElement>(".ov-item").forEach((el) => {
    if (el.dataset.hoverBound === "1") return;
    el.dataset.hoverBound = "1";
    el.addEventListener("mouseenter", () => {
      hoveredKey = el.dataset.key || null;
      if (hoveredKey) void showTooltipFor(hoveredKey, el);
    });
    el.addEventListener("mouseleave", () => {
      if (hoveredKey === el.dataset.key) hoveredKey = null;
      void hideTooltip();
    });
  });
}

async function runMenuAction(action: string) {
  try {
    if (action === "settings") await invoke("show_config_window");
    if (action === "redock") await invoke("dock_overlay_to_taskbar");
    if (action === "quit") await invoke("quit_app");
    if (action === "logs" || action === "config") {
      const info = await invoke<AppInfo>("get_app_info");
      await invoke("open_path", {
        args: { path: action === "logs" ? info.logsDir : info.configDir },
      });
    }
  } catch (e) {
    console.warn("context menu action failed", e);
  }
}

async function createContextMenu(root: HTMLElement) {
  const menu = await Menu.new({
    items: [
      { id: "settings", text: "设置", action: (id) => void runMenuAction(id) },
      { id: "redock", text: "重新贴合任务栏", action: (id) => void runMenuAction(id) },
      { item: "Separator" },
      { id: "logs", text: "打开日志目录", action: (id) => void runMenuAction(id) },
      { id: "config", text: "打开配置目录", action: (id) => void runMenuAction(id) },
      { item: "Separator" },
      { id: "quit", text: "退出", action: (id) => void runMenuAction(id) },
    ],
  });

  document.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    void menu.popup(undefined, getCurrentWindow());
  });
  root.addEventListener("mouseenter", () => root.classList.add("hovering"));
  root.addEventListener("mouseleave", () => root.classList.remove("hovering"));
}

async function main() {
  const root = document.getElementById("root") as HTMLElement;
  await createContextMenu(root);

  let config: OverlayConfig | null = null;
  try {
    config = await invoke<OverlayConfig>("get_overlay_config");
  } catch (e) {
    console.error("get_overlay_config failed", e);
  }
  if (config) {
    applyConfig(config, root);
    autoFitWindow(root, { allowShrink: true });
  }
  bindItemHover(root);

  // Prime tooltips from the last-known snapshots so hover has data on first render
  try {
    const [snap, hw] = await Promise.all([
      invoke<Snapshot | null>("get_realtime_stats"),
      invoke<HwSnapshot | null>("get_hw_snapshot"),
    ]);
    if (snap) lastSnapshot = snap;
    if (hw) lastHw = hw;
  } catch (e) {
    console.warn("prime overlay tooltips failed", e);
  }

  // Update on stats events
  const valDown = root.querySelector<HTMLElement>('[data-key="net-down"] .val')!;
  const valUp = root.querySelector<HTMLElement>('[data-key="net-up"] .val')!;
  const valCpu = root.querySelector<HTMLElement>('[data-key="cpu"] .val')!;
  const valCpuFreq = root.querySelector<HTMLElement>('[data-key="cpu-freq"] .val');
  const valMem = root.querySelector<HTMLElement>('[data-key="mem"] .val')!;
  const valDiskR = root.querySelector<HTMLElement>('[data-key="disk-read"] .val');
  const valDiskW = root.querySelector<HTMLElement>('[data-key="disk-write"] .val');
  const valCpuTemp = root.querySelector<HTMLElement>('[data-key="cpu-temp"] .val');
  const valGpuTemp = root.querySelector<HTMLElement>('[data-key="gpu-temp"] .val');
  const valGpuUsage = root.querySelector<HTMLElement>('[data-key="gpu-usage"] .val');
  const valDiskTemp = root.querySelector<HTMLElement>('[data-key="disk-temp"] .val');
  const valFanRpm = root.querySelector<HTMLElement>('[data-key="fan-rpm"] .val');
  const valMbTemp = root.querySelector<HTMLElement>('[data-key="mb-temp"] .val');

  await listen<Snapshot>("stats:update", (e) => {
    const s = e.payload;
    valDown.textContent = fmtSpeed(s.network.total.bytesRecvPerSec);
    valUp.textContent = fmtSpeed(s.network.total.bytesSentPerSec);
    valCpu.textContent = `${Math.round(s.cpu.usagePercent)}%`;
    valMem.textContent = `${Math.round(s.memory.usedPercent)}%`;
    lastSnapshot = s;
    refreshActiveTooltip();
    // Refit if width changed (e.g., 999 KB/s → 1.0 MB/s)
    autoFitWindow(root);
  });

  await listen<HwSnapshot>("hw:update", (e) => {
    const h = e.payload;
    if (valCpuTemp) valCpuTemp.textContent = fmtTemp(h.cpu?.packageTempC);
    if (valCpuFreq) valCpuFreq.textContent = fmtFreq(h.cpu?.frequencyMhz);
    const gpuUsage = maxValid((h.gpus ?? []).map((g) => g.usagePercent));
    if (valGpuTemp)
      valGpuTemp.textContent = fmtTemp(maxValid((h.gpus ?? []).map((g) => g.tempC)));
    if (valGpuUsage) valGpuUsage.textContent = fmtPct(gpuUsage);
    if (valDiskR)
      valDiskR.textContent = fmtSpeed(sumValid((h.disks ?? []).map((d) => d.readBytesPerSec)));
    if (valDiskW)
      valDiskW.textContent = fmtSpeed(sumValid((h.disks ?? []).map((d) => d.writeBytesPerSec)));
    if (valDiskTemp)
      valDiskTemp.textContent = fmtTemp(maxValid((h.disks ?? []).map((d) => d.tempC)));
    if (valFanRpm)
      valFanRpm.textContent = fmtRpm(maxValid((h.fans ?? []).map((f) => f.rpm)));
    if (valMbTemp) {
      const mbTemps = (h.motherboard?.temperaturesC ?? []).map((t) => t.value);
      valMbTemp.textContent = fmtTemp(maxValid(mbTemps));
    }
    lastHw = h;
    refreshActiveTooltip();
    autoFitWindow(root);
  });

  await listen<OverlayConfig>("overlay:config-changed", (e) => {
    config = e.payload;
    applyConfig(config, root);
    bindItemHover(root);
    autoFitWindow(root, { allowShrink: true });
  });

  // Double-click → open config window.
  document.body.addEventListener("dblclick", async () => {
    try {
      await invoke("show_config_window");
    } catch (e) {
      console.warn("show_config_window failed", e);
    }
  });
}

void main();
