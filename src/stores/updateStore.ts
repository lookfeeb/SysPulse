import { create } from "zustand";
import { check, type Update, type DownloadEvent } from "@tauri-apps/plugin-updater";

// Timeouts chosen to tolerate slow GitHub asset CDN links (particularly from CN),
// while still failing fast enough that the UI doesn't hang.
const CHECK_TIMEOUT_MS = 15_000;
const DOWNLOAD_TIMEOUT_MS = 600_000;

export type UpdateStatus =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "latest"; checkedAt: number }
  | { kind: "available"; version: string; notes: string; checkedAt: number }
  | { kind: "downloading"; progress: number }
  | { kind: "ready" }
  | { kind: "error"; message: string; checkedAt: number };

interface UpdateState {
  status: UpdateStatus;
  /** Cached `Update` resource from the last successful `check()` call. */
  pending: Update | null;
  /** Guard so the startup auto-check only fires once per app session. */
  autoCheckStarted: boolean;

  checkForUpdate: (opts?: { silent?: boolean }) => Promise<void>;
  downloadAndInstall: () => Promise<void>;
  startAutoCheck: () => void;
}

async function runCheck(): Promise<Update | null> {
  return check({ timeout: CHECK_TIMEOUT_MS });
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: { kind: "idle" },
  pending: null,
  autoCheckStarted: false,

  checkForUpdate: async ({ silent = false } = {}) => {
    const prev = get().pending;
    if (!silent) set({ status: { kind: "checking" } });
    try {
      // Release the previously cached Update handle (native Resource) before
      // replacing it — avoids leaking the backing rid in the Tauri runtime.
      if (prev) void prev.close().catch(() => undefined);

      const update = await runCheck();
      if (update) {
        set({
          pending: update,
          status: {
            kind: "available",
            version: update.version,
            notes: update.body ?? "",
            checkedAt: Date.now(),
          },
        });
      } else {
        set({
          pending: null,
          status: { kind: "latest", checkedAt: Date.now() },
        });
      }
    } catch (e: unknown) {
      set({
        pending: null,
        status: {
          kind: "error",
          message: e instanceof Error ? e.message : String(e),
          checkedAt: Date.now(),
        },
      });
    }
  },

  downloadAndInstall: async () => {
    const { pending } = get();
    if (!pending) {
      // Refresh then retry once — the cached handle may have been invalidated.
      await get().checkForUpdate();
      const fresh = get().pending;
      if (!fresh) return;
      await installWithProgress(fresh, set);
      return;
    }
    await installWithProgress(pending, set);
  },

  startAutoCheck: () => {
    if (get().autoCheckStarted) return;
    set({ autoCheckStarted: true });
    // Delay the silent check by 60s so the network stack is ready on cold boot
    // (e.g. auto-start at logon before Wi-Fi/Ethernet is fully connected).
    setTimeout(() => {
      void get().checkForUpdate({ silent: true });
    }, 60_000);
  },
}));

async function installWithProgress(
  update: Update,
  set: (partial: Partial<UpdateState>) => void,
) {
  set({ status: { kind: "downloading", progress: 0 } });
  let downloaded = 0;
  let contentLength = 0;
  try {
    await update.downloadAndInstall(
      (ev: DownloadEvent) => {
        if (ev.event === "Started") {
          contentLength = ev.data.contentLength ?? 0;
          downloaded = 0;
          set({ status: { kind: "downloading", progress: 0 } });
        } else if (ev.event === "Progress") {
          downloaded += ev.data.chunkLength;
          const pct =
            contentLength > 0
              ? Math.min(100, Math.round((downloaded / contentLength) * 100))
              : 0;
          set({ status: { kind: "downloading", progress: pct } });
        } else if (ev.event === "Finished") {
          set({ status: { kind: "ready" } });
        }
      },
      { timeout: DOWNLOAD_TIMEOUT_MS },
    );
    set({ status: { kind: "ready" } });
  } catch (e: unknown) {
    set({
      status: {
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
        checkedAt: Date.now(),
      },
    });
  }
}
