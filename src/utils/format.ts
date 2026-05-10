const KB = 1024;
const MB = 1024 * 1024;
const GB = 1024 * 1024 * 1024;

type MaybeNumber = number | null | undefined;

function isValidNumber(value: MaybeNumber): value is number {
  return value != null && Number.isFinite(value);
}

export function fmtSpeed(b: MaybeNumber, empty = "--"): string {
  if (!isValidNumber(b) || b < 0) return empty;
  if (b >= GB) return `${(b / GB).toFixed(2)} GB/s`;
  if (b >= MB) return `${(b / MB).toFixed(2)} MB/s`;
  if (b >= KB) return `${(b / KB).toFixed(1)} KB/s`;
  return `${b | 0} B/s`;
}

export function fmtBytes(b: MaybeNumber, empty = "--"): string {
  if (!isValidNumber(b) || b < 0) return empty;
  if (b >= GB) return `${(b / GB).toFixed(2)} GB`;
  if (b >= MB) return `${(b / MB).toFixed(2)} MB`;
  if (b >= KB) return `${(b / KB).toFixed(1)} KB`;
  return `${b} B`;
}

export function fmtBytesCompact(b: MaybeNumber, empty = "--"): string {
  if (!isValidNumber(b) || b < 0) return empty;
  if (b >= GB) return `${(b / GB).toFixed(2)} GB`;
  if (b >= MB) return `${(b / MB).toFixed(0)} MB`;
  if (b >= KB) return `${(b / KB).toFixed(0)} KB`;
  return `${b} B`;
}

export function fmtTemp(t: MaybeNumber, empty = "--"): string {
  return isValidNumber(t) ? `${Math.round(t)}°C` : empty;
}

export function fmtFreq(mhz: MaybeNumber, empty = "--"): string {
  if (!isValidNumber(mhz) || mhz <= 0) return empty;
  return mhz >= 1000 ? `${(mhz / 1000).toFixed(2)} GHz` : `${Math.round(mhz)} MHz`;
}

export function fmtPct(value: MaybeNumber, empty = "--"): string {
  return isValidNumber(value) ? `${Math.round(value)}%` : empty;
}

export function fmtRpm(value: MaybeNumber, empty = "--"): string {
  return isValidNumber(value) && value > 0 ? `${Math.round(value)} RPM` : empty;
}

export function maxValid(values: MaybeNumber[], options: { positiveOnly?: boolean } = {}) {
  let max: number | null = null;
  for (const value of values) {
    if (!isValidNumber(value)) continue;
    if (options.positiveOnly && value <= 0) continue;
    if (max == null || value > max) max = value;
  }
  return max;
}

export function sumValid(values: MaybeNumber[], options: { positiveOnly?: boolean } = {}) {
  let total = 0;
  let hasValue = false;
  for (const value of values) {
    if (!isValidNumber(value)) continue;
    if (options.positiveOnly && value <= 0) continue;
    total += value;
    hasValue = true;
  }
  return hasValue ? total : null;
}

export function fmtMaybe<T>(
  v: T | null | undefined,
  fmt: (x: T) => string,
): string {
  return v === null || v === undefined ? "—" : fmt(v);
}
