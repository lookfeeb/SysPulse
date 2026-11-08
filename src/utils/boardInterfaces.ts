import type { BoardInterface } from "@/bindings";

export function isUsbPort(port: BoardInterface): boolean {
  return port.source === "usb" || /^USB\b/i.test(port.kind) || /^USB\b/i.test(port.connector ?? "");
}
export function usbShape(port: BoardInterface): "c" | "a" | "unknown" {
  const shape = (port.connector ?? "").toLowerCase().replace(/[\s_-]/g, "");
  if (shape === "usbtypec" || shape === "usbc") return "c";
  if (shape === "usbtypea" || shape === "usba") return "a";
  return "unknown"; // A missing Type-C bit does not prove a Type-A connector.
}
export function usbVersion(port: BoardInterface): string {
  // Older module versions put the explicit protocol in supportedSpeed.
  // A bare rate or the connector shape never determines a USB version.
  for (const value of [port.kind, port.supportedSpeed ?? ""]) {
    const version = value.match(/^USB\s+(\d+\.(?:\d+|x))\b/i)?.[1];
    if (version) return "USB " + version.toLowerCase();
  }
  return "版本未报告";
}
export function supportedRate(port: BoardInterface): string | null {
  // Old helpers inferred 480 Mb/s or >= 5 Gb/s from a logical USB channel.
  // Neither value establishes the physical socket's maximum capability.
  if (port.source === "usb" && !port.usb?.maximumSpeedSource) return null;
  return port.supportedSpeed?.replace(/^USB\s+[^（(]+[（(](.*)[）)]$/, "$1") || null;
}
export function capabilityLabel(port: BoardInterface): string {
  return port.source === "usb" ? "系统 USB 通道上限" : port.source === "pcie" ? "设备链路上限" : port.source === "firmware-slot" ? "固件报告能力" : "接口额定上限";
}
export function portShapeLabel(port: BoardInterface): string {
  if (!isUsbPort(port)) return port.kind || "接口";
  return usbShape(port) === "c" ? "USB-C" : usbShape(port) === "a" ? "USB-A" : "USB · 形态未报告";
}
export function connectionLabel(status: string): string {
  return ({ connected: "已连接", disconnected: "未连接", "in-use": "已占用", available: "空闲",
    unavailable: "不可用", error: "连接异常" } as Record<string, string>)[status] ?? "状态未知";
}
export function orderPorts(ports: BoardInterface[]): BoardInterface[] {
  const rank = (p: BoardInterface) => p.source === "usb" ? (usbShape(p) === "c" ? 0 : 1) :
    p.source === "pcie" ? 2 : p.source === "network" ? 3 : p.source === "firmware-slot" ? 4 : 5;
  // Stable IDs, rather than changing connection status, keep rows stationary on refresh.
  return [...ports].sort((a, b) => rank(a) - rank(b) || a.name.localeCompare(b.name, "zh-CN", { numeric: true }) || a.id.localeCompare(b.id));
}
export function filterPorts(ports: BoardInterface[], source: string, connectedOnly = false): BoardInterface[] {
  return orderPorts(ports.filter(p => (source === "all" || p.source === source) &&
    (!connectedOnly || p.status === "connected")));
}
