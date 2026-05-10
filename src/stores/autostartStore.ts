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

export const useAutostartStore = create<AutostartState>((set, get) => ({
  enabled: null,
  loading: false,
  error: null,

  refresh: async () => {
    set({ loading: true, error: null });
    try {
      const enabled = await commands.autostartIsEnabled();
      set({ enabled, loading: false });
      return enabled;
    } catch (e: unknown) {
      set({ loading: false, error: errorMessage(e) });
      throw e;
    }
  },

  setEnabled: async (enabled) => {
    set({ loading: true, error: null });
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
    }
  },
}));
