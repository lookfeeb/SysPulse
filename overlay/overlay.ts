import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Menu } from "@tauri-apps/api/menu";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { fmtFreq, fmtPct, fmtRpm, fmtSpeed, fmtTemp, maxValid, sumValid } from "@/utils/format";
import { expandOrderedItems, overlayPairFor } from "@/utils/overlayItems";
import type { OverlayItem } from "@/bindings";

interface InterfaceStats {
  bytesSentPerSec: number;
  bytesRecvPerSec: number;
}

interface Snapshot {
  cpu: { usagePercent: number };
  memory: { usedPercent: number };
  network: { total: InterfaceStats };
}

interface DiskHw {
  tempC: number | null;
  readBytesPerSec: number | null;
  writeBytesPerSec: number | null;
}

interface HwSnapshot {
  cpu?: { packageTempC: number | null; frequencyMhz: number | null } | null;
  gpus?: { tempC: number | null; usagePercent: number | null }[];
  disks?: DiskHw[];
  motherboard?: { temperaturesC: { value: number }[] } | null;
  fans?: { rpm: number | null }[];
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
let fitFrame: number | null = null;

function autoFitWindow(root: HTMLElement, options: { allowShrink?: boolean } = {}) {
  if (fitFrame != null) {
    cancelAnimationFrame(fitFrame);
  }
  fitFrame = requestAnimationFrame(async () => {
    fitFrame = null;
    const previousWidth = root.style.width;
    const previousMaxWidth = root.style.maxWidth;
    const previousFlexWrap = root.style.flexWrap;
    const previousHeight = root.style.height;
    const previousOverflow = root.style.overflow;
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
}

function setText(el: HTMLElement | null, value: string): boolean {
  if (!el || el.textContent === value) return false;
  el.textContent = value;
  return true;
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

  const resizeObserver = new ResizeObserver(() => {
    autoFitWindow(root, { allowShrink: true });
  });
  resizeObserver.observe(root);

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
    const changed = [
      setText(valDown, fmtSpeed(s.network.total.bytesRecvPerSec)),
      setText(valUp, fmtSpeed(s.network.total.bytesSentPerSec)),
      setText(valCpu, `${Math.round(s.cpu.usagePercent)}%`),
      setText(valMem, `${Math.round(s.memory.usedPercent)}%`),
    ].some(Boolean);
    if (changed) autoFitWindow(root, { allowShrink: true });
  });

  await listen<HwSnapshot>("hw:update", (e) => {
    const h = e.payload;
    const changed = [
      setText(valCpuTemp, fmtTemp(h.cpu?.packageTempC)),
      setText(valCpuFreq, fmtFreq(h.cpu?.frequencyMhz)),
    ];
    const gpuUsage = maxValid((h.gpus ?? []).map((g) => g.usagePercent));
    changed.push(
      setText(valGpuTemp, fmtTemp(maxValid((h.gpus ?? []).map((g) => g.tempC)))),
      setText(valGpuUsage, fmtPct(gpuUsage)),
      setText(valDiskR, fmtSpeed(sumValid((h.disks ?? []).map((d) => d.readBytesPerSec)))),
      setText(valDiskW, fmtSpeed(sumValid((h.disks ?? []).map((d) => d.writeBytesPerSec)))),
      setText(valDiskTemp, fmtTemp(maxValid((h.disks ?? []).map((d) => d.tempC)))),
      setText(valFanRpm, fmtRpm(maxValid((h.fans ?? []).map((f) => f.rpm)))),
    );
    if (valMbTemp) {
      const mbTemps = (h.motherboard?.temperaturesC ?? []).map((t) => t.value);
      changed.push(setText(valMbTemp, fmtTemp(maxValid(mbTemps))));
    }
    if (changed.some(Boolean)) autoFitWindow(root, { allowShrink: true });
  });

  await listen<OverlayConfig>("overlay:config-changed", (e) => {
    config = e.payload;
    applyConfig(config, root);
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
