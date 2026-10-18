type RegisterListeners = () => Promise<unknown>;

/** Application-lifetime listeners; registration must undo partial work before rejecting. */
export function createEventBinder(register: RegisterListeners) {
  let bound = false;
  let binding: Promise<void> | null = null;

  return async function bindOnce() {
    if (bound) return;
    if (binding) return binding;

    binding = (async () => {
      await register();
      bound = true;
    })();

    try {
      await binding;
    } finally {
      binding = null;
    }
  };
}
