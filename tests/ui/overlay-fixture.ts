import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";

let finishMenu: ((value: [number, string]) => void) | undefined;
export const overlayFixture = {
  showCalls: 0,
  popupCalls: 0,
  finishMenu: () => finishMenu?.([1, "fixture-menu"]),
};

mockWindows("overlay");
mockIPC((command) => {
  switch (command) {
    case "plugin:menu|new": return new Promise<[number, string]>(resolve => { finishMenu = resolve; });
    case "plugin:menu|popup": overlayFixture.popupCalls++; return null;
    case "show_config_window": overlayFixture.showCalls++; return null;
    case "get_overlay_config": return {items:["net-down", "net-up", "cpu", "mem"]};
    case "resize_overlay": case "register_overlay_tooltip_regions": return null;
    default: throw new Error(`隔离验收未实现命令: ${command}`);
  }
}, {shouldMockEvents:true});

// Use the production markup and script, holding menu creation to exercise
// double-clicks before asynchronous overlay initialization has completed.
const markup = new DOMParser().parseFromString(await (await fetch("/overlay/index.html")).text(), "text/html");
document.body.append(markup.getElementById("root")!);
await import("../../overlay/overlay");
