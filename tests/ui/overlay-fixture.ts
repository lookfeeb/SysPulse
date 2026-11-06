import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

let finishMenu: ((value: [number, string]) => void) | undefined;
export const overlayFixture = {
  showCalls: 0,
  popupCalls: 0,
  sizes: [] as { width: number; height: number }[],
  regionCalls: [] as {regions:{key:string;left:number;top:number;right:number;bottom:number}[]}[],
  items: ["net-down", "cpu", "cpu-temp", "mem", "fan-rpm"],
  configure: (items: string[]) => emit("overlay:config-changed", {items}),
  publish: (up: number, down: number, cpu = 100, rpm = 1403) => Promise.all([
    emit("stats:update", {cpu:{usagePercent:cpu},memory:{usedPercent:81},network:{total:{bytesSentPerSec:up,bytesRecvPerSec:down}}}),
    emit("hw:update", {cpu:{packageTempC:60,frequencyMhz:4200},fans:[{rpm}],disks:[{readBytesPerSec:down,writeBytesPerSec:up,tempC:42}]}),
  ]),
  finishMenu: () => finishMenu?.([1, "fixture-menu"]),
};

mockWindows("overlay");
mockIPC((command, payload) => {
  switch (command) {
    case "plugin:menu|new": return new Promise<[number, string]>(resolve => { finishMenu = resolve; });
    case "plugin:menu|popup": overlayFixture.popupCalls++; return null;
    case "show_config_window": overlayFixture.showCalls++; return null;
    case "get_overlay_config": return {items:overlayFixture.items};
    case "resize_overlay": overlayFixture.sizes.push((payload as {args:{width:number;height:number}}).args); return null;
    case "register_overlay_tooltip_regions": overlayFixture.regionCalls.push((payload as {args:typeof overlayFixture.regionCalls[number]}).args); return null;
    default: throw new Error(`隔离验收未实现命令: ${command}`);
  }
}, {shouldMockEvents:true});

// Use the production markup and script, holding menu creation to exercise
// double-clicks before asynchronous overlay initialization has completed.
const markup = new DOMParser().parseFromString(await (await fetch("/overlay/index.html")).text(), "text/html");
document.body.append(markup.getElementById("root")!);
await import("../../overlay/overlay");
