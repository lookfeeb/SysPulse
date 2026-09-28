import { create } from "zustand";
import type { AppConfig } from "@/bindings";
import * as configApi from "@/ipc";
import { listen } from "@tauri-apps/api/event";
import { createEventBinder } from "@/utils/bindOnce";

interface ConfigState {
  config: AppConfig | null;
  loading: boolean;
  error: string | null;
  firstLoadFailure: string | null;

  load: () => Promise<void>;
  patch: (p: Partial<AppConfig>) => Promise<void>;
  reset: () => Promise<void>;
  set: (cfg: AppConfig) => void;
}

export const useConfigStore = create<ConfigState>((set) => ({
  config: null,
  loading: false,
  error: null,
  firstLoadFailure: null,

  load: async () => {
    set({ loading: true, error: null });
    try {
      const cfg = await configApi.getConfig();
      set({ config: cfg, loading: false, error: null });
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      const code = e instanceof configApi.IpcError ? e.code : "UNKNOWN";
      // Retain the first failure after a successful retry, without storing config data.
      set((state) => ({
        loading: false,
        error: message,
        firstLoadFailure: state.firstLoadFailure ??
          `${new Date().toISOString()} get_config [${code}] ${message}`.slice(0, 4096),
      }));
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

export const bindConfigEvents = createEventBinder(() =>
  listen<AppConfig>("config:changed", (e) => {
    useConfigStore.getState().set(e.payload);
  }),
);
