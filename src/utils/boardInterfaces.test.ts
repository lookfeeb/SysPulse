import { describe, expect, it } from "vitest";
import type { BoardInterface } from "@/bindings";
import { capabilityLabel, filterPorts, orderPorts, portShapeLabel, supportedRate, usbShape, usbVersion } from "./boardInterfaces";
const port = (patch: Partial<BoardInterface> = {}): BoardInterface => ({
  id: "usb:1", name: "USB 集线器 1 / 端口 1", kind: "USB", connector: null,
  supportedSpeed: null, currentSpeed: null, status: "unknown", device: null, source: "usb", ...patch,
});
describe("board interface presentation", () => {
  it("keeps connector shape independent from protocol and bandwidth", () => {
    const typeC = port({ connector: "USB Type-C", kind: "USB 2.0", supportedSpeed: "480 Mb/s" });
    expect(portShapeLabel(typeC)).toBe("USB-C");
    expect(usbVersion(typeC)).toBe("USB 2.0");
    expect(usbShape(port({ kind: "USB 3.x" }))).toBe("unknown");
    expect(usbShape(port({ connector: "USB Type-A" }))).toBe("a");
    expect(usbVersion(port({ connector: "USB Type-C", supportedSpeed: "5 Gb/s" }))).toBe("版本未报告");
  });
  it("supports older module snapshots without inventing a USB 3 minor version", () => {
    const old = port({ supportedSpeed: "USB 3.x（至少 5 Gb/s）" });
    expect(usbVersion(old)).toBe("USB 3.x");
    expect(supportedRate(old)).toBeNull();
    expect(supportedRate(port({ supportedSpeed: "480 Mb/s", kind: "USB 2.0" }))).toBeNull();
    expect(supportedRate(port({ supportedSpeed: "480 Mb/s", usb: { logicalPortCount: 1,
      userConnectable: true, maximumSpeedSource: "USB 协议 + 完整伴随端口映射", deviceCapability: null,
      deviceCapabilitySource: null, topologyNote: "无伴随端口" } }))).toBe("480 Mb/s");
    expect(capabilityLabel(port())).toBe("系统 USB 通道上限");
    expect(capabilityLabel(port({ source: "pcie" }))).toBe("设备链路上限");
    expect(usbVersion(port({ source: "firmware-connector" }))).toBe("版本未报告");
  });
  it("puts live Type-C records before firmware while preserving input and refresh order", () => {
    const board = port({ id: "fw:1", source: "firmware-connector" });
    const normal = port({ id: "usb:2" });
    const c = port({ id: "usb:3", connector: "USB Type-C" });
    const items = [board, normal, c];
    expect(orderPorts(items).map(p => p.id)).toEqual(["usb:3", "usb:2", "fw:1"]);
    expect(items[0]).toBe(board);
    expect(orderPorts([{ ...c, status: "connected" }, normal]).map(p => p.id)).toEqual(["usb:3", "usb:2"]);
  });
  it("keeps every shape, protocol and connection state within the selected category", () => {
    const c = port({ connector: "USB Type-C", kind: "USB 2.0", status: "connected", device: "Keyboard" });
    const other = port({ id: "usb:2", kind: "USB 3.x" });
    const board = port({ id: "firmware:1", source: "firmware-connector" });
    expect(filterPorts([c, other, board], "usb")).toEqual([c, other]);
    const disconnected = { ...c, status: "disconnected" };
    expect(filterPorts([disconnected, other], "usb")).toEqual([disconnected, other]);
    expect(filterPorts([c, other, board], "all")).toHaveLength(3);
    expect(filterPorts([c, other, board], "pcie")).toEqual([]);
    expect(filterPorts([c, other, { ...board, status: "in-use" }], "all", true)).toEqual([c]);
    expect(filterPorts([disconnected, other], "usb", true)).toEqual([]);
  });
});
