/** Release listeners even when registration completes after a component unmounts. */
export function disposeAsyncListeners(
  registrations: Promise<() => void>[],
  onError: (error: unknown) => void,
): () => void {
  let disposed = false;
  const cleanups = new Set<() => void>();
  for (const registration of registrations) {
    void registration.then((cleanup) => {
      if (disposed) cleanup();
      else cleanups.add(cleanup);
    }).catch(onError);
  }
  return () => {
    disposed = true;
    for (const cleanup of cleanups) {
      try { cleanup(); } catch (error) { onError(error); }
    }
    cleanups.clear();
  };
}
