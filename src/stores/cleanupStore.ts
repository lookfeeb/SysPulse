import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { commands, type CleanArgs, type CleanResult, type ScanResult } from "@/bindings";
import { createEventBinder } from "@/utils/bindOnce";
import { errorMessage } from "@/utils/errorMessage";
import { fmtBytes } from "@/utils/format";

type CleanupProgress = {
  percent: number; processedItems: number; totalItems: number;
  currentCategory: string; currentPath: string | null;
  freedBytes: number; deletedFiles: number; done: boolean;
};
type CleanupScanProgress = {
  stage: string; phase: string; scannedPaths: number; skippedPaths: number;
  ignoredPaths: number; scoutTasks: number; engineerTasks: number;
  currentPath: string | null; done: boolean; cancelled: boolean;
};
type Notice = { id: number; type: "success" | "warning" | "error"; text: string };
type CleanupState = {
  taskId: number;
  scan: ScanResult | null;
  phase: "idle" | "scanning" | "cleaning" | "refreshing";
  consumed: boolean;
  cancelRequested: boolean;
  scanProgress: CleanupScanProgress | null;
  cleanProgress: CleanupProgress | null;
  lastCleanResult: CleanResult | null;
  notice: Notice | null;
  invalidate: () => void;
  startScan: (projectRoots: string[]) => Promise<void>;
  cancelScan: () => Promise<void>;
  clean: (args: CleanArgs, projectRoots: string[]) => Promise<void>;
};

let noticeId = 0;
let taskId = 0;
const notice = (type: Notice["type"], text: string): Notice => ({ id: ++noticeId, type, text });
const initialProgress = (): CleanupScanProgress => ({
  stage: "queen", phase: "准备扫描", scannedPaths: 0, skippedPaths: 0,
  ignoredPaths: 0, scoutTasks: 0, engineerTasks: 0, currentPath: null,
  done: false, cancelled: false,
});

// 任务属于应用，不属于路由；页面卸载只取消 React 订阅，不取消 IPC 或事件监听。
export const useCleanupStore = create<CleanupState>((set, get) => ({
  taskId: 0, scan: null, phase: "idle", consumed: false, cancelRequested: false,
  scanProgress: null, cleanProgress: null, lastCleanResult: null, notice: null,
  invalidate() {
    if (get().phase !== "idle") return;
    set({ scan: null, consumed: false, scanProgress: null });
  },
  async startScan(projectRoots) {
    if (get().phase !== "idle") return;
    set({ taskId: ++taskId, phase: "scanning", scan: null, consumed: false, cancelRequested: false,
      scanProgress: initialProgress(), cleanProgress: null, notice: null });
    try {
      await bindCleanupEvents();
      const scan = await commands.scanCleanup({ projectRoots: [...projectRoots] });
      set({ scan, notice: scan.cancelled
        ? notice("warning", "扫描已停止，当前结果仅供审核")
        : !scan.complete
          ? notice("warning", "扫描达到限制，当前结果仅供审核，请缩小扫描范围后重试")
          : notice("success", `扫描完成，发现 ${fmtBytes(scan.totalSizeBytes)}`) });
    } catch (error) {
      set({ notice: notice("error", `扫描失败：${errorMessage(error)}`) });
    } finally {
      set({ phase: "idle", cancelRequested: false });
    }
  },
  async cancelScan() {
    if (!["scanning", "refreshing"].includes(get().phase) || get().cancelRequested) return;
    const cancelledTask = get().taskId;
    set({ cancelRequested: true });
    try {
      const accepted = await commands.cancelCleanupScan();
      if (!accepted && get().taskId === cancelledTask) set({ cancelRequested: false });
    } catch (error) {
      if (get().taskId === cancelledTask) {
        set({ cancelRequested: false, notice: notice("error", `停止扫描失败：${errorMessage(error)}`) });
      }
    }
  },
  async clean(args, projectRoots) {
    const current = get();
    if (current.phase !== "idle" || current.consumed || !current.scan?.complete
      || current.scan.cancelled || current.scan.scanId !== args.scanId
      || current.scan.expiresAtMs <= Date.now()) return;
    // 同步占用快照，快速双击、切页或重新挂载均不能重复提交删除。
    set({ taskId: ++taskId, phase: "cleaning", consumed: true, lastCleanResult: null, notice: null,
      cleanProgress: { percent: 0, processedItems: 0, totalItems: 0,
        currentCategory: "正在校验扫描快照", currentPath: null, freedBytes: 0,
        deletedFiles: 0, done: false } });
    try {
      await bindCleanupEvents();
      const result = await commands.cleanCategories(args);
      set({ lastCleanResult: result,
        cleanProgress: { ...get().cleanProgress!, freedBytes: result.freedBytes,
          deletedFiles: result.deletedFiles, percent: 100, done: true },
        notice: notice(result.errors.length ? "warning" : "success",
          result.errors.length
            ? `清理结束，释放 ${fmtBytes(result.freedBytes)}；部分内容已跳过，详情见清理页`
            : `已释放 ${fmtBytes(result.freedBytes)}，删除 ${result.deletedFiles.toLocaleString()} 个文件`) });
      // 清理结果单独保存；刷新失败/取消不能覆盖已完成的清理报告。
      set({ phase: "refreshing", scan: null, scanProgress: initialProgress(), cancelRequested: false });
      try {
        const scan = await commands.scanCleanup({ projectRoots: [...projectRoots] });
        set({ scan, consumed: false });
      } catch (error) {
        set({ notice: notice("warning", `清理报告已保留；刷新扫描失败：${errorMessage(error)}`) });
      }
    } catch (error) {
      set({ cleanProgress: null, notice: notice("error", `清理失败：${errorMessage(error)}`) });
    } finally {
      set({ phase: "idle", cancelRequested: false });
    }
  },
}));

const bindCleanupEvents = createEventBinder(async () => {
  const unlistenScan = await listen<CleanupScanProgress>("cleanup:scan-progress", ({ payload }) => {
    const state = useCleanupStore.getState();
    if (state.phase === "scanning" || state.phase === "refreshing") {
      useCleanupStore.setState({ scanProgress: payload });
    }
  });
  try {
    const unlistenClean = await listen<CleanupProgress>("cleanup:progress", ({ payload }) => {
      if (useCleanupStore.getState().phase === "cleaning") {
        useCleanupStore.setState({ cleanProgress: payload });
      }
    });
    return [unlistenScan, unlistenClean];
  } catch (error) {
    unlistenScan();
    throw error;
  }
});
