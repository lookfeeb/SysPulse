import { create } from "zustand";
import type { AppConfig } from "@/bindings";
import * as configApi from "@/ipc";
import { listen } from "@tauri-apps/api/event";

interface ConfigState {
  config: AppConfig | null;
  loading: boolean;
  error: string | null;

  load: () => Promise<void>;
  patch: (p: Partial<AppConfig>) => Promise<void>;
  reset: () => Promise<void>;
  set: (cfg: AppConfig) => void;
}

export const useConfigStore = create<ConfigState>((set) => ({
  config: null,
  loading: false,
  error: null,

  load: async () => {
    set({ loading: true, error: null });
    try {
      const cfg = await configApi.getConfig();
      set({ config: cfg, loading: false });
    } catch (e: unknown) {
      set({
        loading: false,
        error: e instanceof Error ? e.message : String(e),
      });
    }
  },

  patch: async (p) => {
    try {
      const cfg = await configApi.setConfig(p);
      set({ config: cfg });
    } catch (e: unknown) {
      set({ error: e instanceof Error ? e.message : String(e) });
      throw e;
    }
  },

  reset: async () => {
    const cfg = await configApi.resetConfig();
    set({ config: cfg });
  },

  set: (cfg: AppConfig) => set({ config: cfg }),
}));

// Subscribe to backend config changes (e.g., changes from another window or
// from external file edits routed through the backend).
let unlistener: (() => void) | null = null;
export async function bindConfigEvents() {
  if (unlistener) return;
  const fn = await listen<AppConfig>("config:changed", (e) => {
    useConfigStore.getState().set(e.payload);
  });
  unlistener = fn;
}
