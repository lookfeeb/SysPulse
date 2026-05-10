import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { commands } from "@/bindings";
import type {
  AppConfig,
  AppInfo,
  DailyTraffic,
  FanControlStatus,
  FanCurvePoint,
  HelperStatus,
  HelperStatusEvent,
  HistoryQuery,
  HwSnapshot,
  JsonValue,
  Snapshot,
} from "@/bindings";

export class IpcError extends Error {
  code: string;

  constructor(code: string, message: string) {
    super(message);
    this.code = code;
    this.name = "IpcError";
  }
}

async function call<T>(promise: Promise<T>): Promise<T> {
  try {
    return await promise;
  } catch (e: unknown) {
    if (e && typeof e === "object" && "code" in e && "message" in e) {
      const err = e as { code: string; message: string };
      throw new IpcError(err.code, err.message);
    }
    throw e instanceof Error ? e : new IpcError("UNKNOWN", String(e));
  }
}

export const getConfig = () => call(commands.getConfig());

export const setConfig = (patch: Partial<AppConfig>) =>
  call(commands.setConfig({ patch: patch as unknown as JsonValue }));

export const resetConfig = () => call(commands.resetConfig());

export const getRealtimeStats = () => call(commands.getRealtimeStats());

export function subscribeStats(
  handler: (s: Snapshot) => void,
): Promise<UnlistenFn> {
  return listen<Snapshot>("stats:update", (e) => handler(e.payload));
}

export const queryTrafficHistory = (query: HistoryQuery) =>
  call(commands.queryTrafficHistory(query));

export const exportTrafficCsv = (query: HistoryQuery, path: string) =>
  call(commands.exportTrafficCsv({ query, path }));

export const getAppInfo = () => call(commands.getAppInfo());

export const openPath = async (path: string) => {
  await call(commands.openPath({ path }));
};

export const quitApp = async () => {
  await call(commands.quitApp());
};

export const showConfigWindow = async () => {
  await call(commands.showConfigWindow());
};

export const hideConfigWindow = async () => {
  await call(commands.hideConfigWindow());
};

export const dockOverlayToTaskbar = async () => {
  await call(commands.dockOverlayToTaskbar());
};

export const getHwSnapshot = () => call(commands.getHwSnapshot());

export const getHelperStatus = () => call(commands.getHelperStatus());

export const isAdmin = () => call(commands.isAdmin());

export const getFanControlState = () => call(commands.getFanControlState());

export const setFanManual = (fanId: string, pwm: number) =>
  call(commands.setFanManual({ fanId, pwm }));

export const setFanCurve = (fanId: string, curve: FanCurvePoint[]) =>
  call(commands.setFanCurve({ fanId, curve }));

export const resetFanControl = (fanId: string) =>
  call(commands.resetFanControl({ fanId }));

export const resetAllFanControls = () => call(commands.resetAllFanControls());

export function subscribeHwUpdate(
  handler: (s: HwSnapshot) => void,
): Promise<UnlistenFn> {
  return listen<HwSnapshot>("hw:update", (e) => handler(e.payload));
}

export function subscribeHelperStatus(
  handler: (s: HelperStatusEvent) => void,
): Promise<UnlistenFn> {
  return listen<HelperStatusEvent>("hw:helper-status", (e) => handler(e.payload));
}

export function subscribeFanControl(
  handler: (s: FanControlStatus) => void,
): Promise<UnlistenFn> {
  return listen<FanControlStatus>("fan-control:changed", (e) => handler(e.payload));
}

export type {
  AppConfig,
  AppInfo,
  DailyTraffic,
  FanControlStatus,
  FanCurvePoint,
  HelperStatus,
  HelperStatusEvent,
  HistoryQuery,
  HwSnapshot,
  JsonValue,
  Snapshot,
};
