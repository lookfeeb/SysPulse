import { create } from "zustand";
import { Channel } from "@tauri-apps/api/core";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { commands, type UpdateDownloadEvent } from "@/bindings";
import { errorMessage } from "@/utils/errorMessage";

export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "latest"; checkedAt: number }
  | { kind: "available"; version: string; notes: string; checkedAt: number }
  | { kind: "downloading"; progress: number | null; downloaded: number; total: number | null; bytesPerSecond: number }
  | { kind: "verifying" }
  | { kind: "installing" }
  | { kind: "ready" }
  | { kind: "error"; operation: "check" | "install"; message: string; checkedAt: number };

interface UpdateState {
  status: UpdateStatus;
  pending: Update | null;
  autoCheckStarted: boolean;
  checkForUpdate: (opts?: { silent?: boolean }) => Promise<void>;
  downloadAndInstall: () => Promise<void>;
  startAutoCheck: () => void;
}

let checkTask: Promise<void> | null = null;
let installTask: Promise<void> | null = null;

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: { kind: "idle" },
  pending: null,
  autoCheckStarted: false,

  checkForUpdate: ({ silent = false } = {}) => {
    if (installTask || get().status.kind === "ready") return Promise.resolve();
    if (checkTask) return checkTask;
    // A delayed auto-check must not discard an update the user is already viewing.
    if (silent && get().status.kind !== "idle") return Promise.resolve();
    const previous = get().pending;
    set({ status: { kind: "checking" } });
    checkTask = (async () => {
      try {
        const update = await check({ timeout: 15_000 });
        set({ pending: update, status: update
          ? { kind: "available", version: update.version, notes: update.body ?? "", checkedAt: Date.now() }
          : { kind: "latest", checkedAt: Date.now() } });
        if (previous && previous !== update) await previous.close().catch(() => undefined);
      } catch (error) {
        set({ status: { kind: "error", operation: "check", message: errorMessage(error), checkedAt: Date.now() } });
      }
    })().finally(() => { checkTask = null; });
    return checkTask;
  },

  downloadAndInstall: () => {
    if (installTask) return installTask;
    if (get().status.kind === "ready") return Promise.resolve();
    // Schedule the job so the guard is set before any asynchronous work starts.
    installTask = Promise.resolve().then(async () => {
      if (checkTask) await checkTask;
      let update = get().pending;
      if (!update) {
        // The install owns the lock, so perform this refresh directly.
        set({ status: { kind: "checking" } });
        try {
          update = await check({ timeout: 15_000 });
          set({ pending: update });
          if (!update) { set({ status: { kind: "latest", checkedAt: Date.now() } }); return; }
        } catch (error) {
          set({ status: { kind: "error", operation: "check", message: errorMessage(error), checkedAt: Date.now() } });
          return;
        }
      }
      set({ status: { kind: "downloading", progress: null, downloaded: 0, total: null, bytesPerSecond: 0 } });
      let sampledAt = performance.now();
      let sampledBytes = 0;
      let speed = 0;
      const channel = new Channel<UpdateDownloadEvent>();
      channel.onmessage = (event) => {
        if (event.event === "Started") {
          sampledAt = performance.now(); sampledBytes = 0; speed = 0;
          set({ status: { kind: "downloading", progress: event.data.contentLength ? 0 : null, downloaded: 0,
            total: event.data.contentLength, bytesPerSecond: 0 } });
        } else if (event.event === "Progress") {
          const now = performance.now();
          const elapsed = now - sampledAt;
          if (elapsed >= 500) {
            speed = Math.max(0, (event.data.downloaded - sampledBytes) * 1000 / elapsed);
            sampledAt = now; sampledBytes = event.data.downloaded;
          }
          const total = event.data.contentLength;
          set({ status: { kind: "downloading", downloaded: event.data.downloaded, total, bytesPerSecond: speed,
            progress: total ? Math.min(100, Math.floor(event.data.downloaded / total * 100)) : null } });
        } else if (event.event === "Verifying") {
          set({ status: { kind: "verifying" } });
        } else if (event.event === "Installing") {
          set({ status: { kind: "installing" } });
        }
      };
      try {
        await commands.downloadInstallUpdate(update.rid, channel);
        set({ pending: null, status: { kind: "ready" } });
        await update.close().catch(() => undefined);
      } catch (error) {
        // Retain the native update handle so retry does not re-check or leak resources.
        set({ status: { kind: "error", operation: "install", message: errorMessage(error), checkedAt: Date.now() } });
      } finally {
        channel.onmessage = () => undefined;
      }
    }).finally(() => { installTask = null; });
    return installTask;
  },

  startAutoCheck: () => {
    if (get().autoCheckStarted) return;
    set({ autoCheckStarted: true });
    setTimeout(() => { void get().checkForUpdate({ silent: true }); }, 60_000);
  },
}));
