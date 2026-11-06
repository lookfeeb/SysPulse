import { create } from "zustand";
import { commands } from "@/bindings";

interface AutostartState {
  enabled: boolean | null;
  loading: boolean;
  error: string | null;
  refresh: () => Promise<boolean>;
  setEnabled: (enabled: boolean) => Promise<boolean>;
}

function errorMessage(e: unknown) {
  return e instanceof Error ? e.message : String(e);
}

let refreshTask: Promise<boolean> | null = null;
let changeTask: Promise<boolean> | null = null;
let revision = 0;

export const useAutostartStore = create<AutostartState>((set, get) => ({
  enabled: null,
  loading: false,
  error: null,

  refresh: () => {
    if (changeTask) return changeTask;
    if (refreshTask) return refreshTask;
    const startedRevision = revision;
    // Keep the confirmed switch steady while checking in the background.
    set({ loading: get().enabled === null, error: null });
    const task = Promise.resolve().then(async () => {
      try {
        const enabled = await commands.autostartIsEnabled();
        if (startedRevision === revision) set({ enabled, loading: false });
        return enabled;
      } catch (e: unknown) {
        if (startedRevision === revision) set({ loading: false, error: errorMessage(e) });
        throw e;
      } finally {
        if (refreshTask === task) refreshTask = null;
      }
    });
    refreshTask = task;
    return task;
  },

  setEnabled: (enabled) => {
    if (changeTask) return changeTask;
    revision++;
    refreshTask = null;
    set({ loading: true, error: null });
    const task = Promise.resolve().then(async () => {
      try {
        if (enabled) await commands.autostartEnable();
        else await commands.autostartDisable();

        const actual = await commands.autostartIsEnabled();
        set({ enabled: actual, loading: false });
        return actual;
      } catch (e: unknown) {
        const fallback = await commands.autostartIsEnabled().catch(() => get().enabled);
        set({ enabled: fallback, loading: false, error: errorMessage(e) });
        throw e;
      } finally {
        changeTask = null;
      }
    });
    changeTask = task;
    return task;
  },
}));
