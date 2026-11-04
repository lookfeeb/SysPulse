export const SENSOR_FRESH_MS = 5000;
export const SENSOR_DISPLAY_MS = 30000;

export function sensorAge(reported: number | undefined, elapsed = 0): number {
  return reported == null ? Math.max(0, elapsed) : Number.isFinite(reported) && reported >= 0
    ? reported + Math.max(0, elapsed) : Infinity;
}

/** Keep a bounded last reading with a visible delay indication. Never use this
 * display grace period for fan-control decisions. Missing fields stay missing. */
export function visibleHardware<T extends {
  sampleAgeMs?: number; storageAgeMs?: number;
  cpu?: unknown; gpus?: unknown[]; memory?: unknown;
  motherboard?: unknown; fans?: unknown[]; disks?: unknown[];
}>(snapshot: T, elapsed = 0): T {
  return {
    ...snapshot,
    ...(sensorAge(snapshot.sampleAgeMs, elapsed) > SENSOR_DISPLAY_MS
      ? { cpu: null, gpus: [], memory: null, motherboard: null, fans: [] } : {}),
    ...(sensorAge(snapshot.storageAgeMs, elapsed) > SENSOR_DISPLAY_MS ? { disks: [] } : {}),
  };
}
