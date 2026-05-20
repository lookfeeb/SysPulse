import type { UnlistenFn } from "@tauri-apps/api/event";

type RegisterListeners = () => Promise<UnlistenFn | UnlistenFn[]>;

export function createEventBinder(register: RegisterListeners) {
  let bound = false;
  let binding: Promise<void> | null = null;
  let unlisteners: UnlistenFn[] = [];

  return async function bindOnce() {
    if (bound) return;
    if (binding) return binding;

    binding = (async () => {
      const result = await register();
      unlisteners = Array.isArray(result) ? result : [result];
      bound = true;
    })();

    try {
      await binding;
    } catch (error) {
      for (const unlisten of unlisteners) {
        try {
          unlisten();
        } catch {
          // Ignore listener cleanup errors so the next bind attempt can retry.
        }
      }
      unlisteners = [];
      bound = false;
      throw error;
    } finally {
      binding = null;
    }
  };
}
