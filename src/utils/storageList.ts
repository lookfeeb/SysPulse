export function readStoredStringList(key: string): string[] | null {
  try {
    const raw = window.localStorage.getItem(key);
    if (raw === null) return null;
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed)
      ? parsed.filter((value): value is string => typeof value === "string")
      : [];
  } catch {
    return null;
  }
}

export function writeStoredStringList(key: string, values: Iterable<string>) {
  try {
    window.localStorage.setItem(key, JSON.stringify([...values]));
  } catch {
    // Ignore storage failures in restricted WebView contexts.
  }
}
