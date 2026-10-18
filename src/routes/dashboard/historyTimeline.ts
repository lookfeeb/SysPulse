type Timed = { timestampMs: number };

export function historyTimeline(...sources: readonly Timed[][]): number[] {
  const timestamps = sources.flatMap((source) => source.map((s) => s.timestampMs)).filter(Number.isFinite);
  if (!timestamps.length) return [];
  const end = Math.max(...timestamps);
  const start = Math.max(end - 59_000, Math.min(...timestamps));
  const count = Math.min(60, Math.ceil((end - start) / 1000) + 1);
  return Array.from({ length: count }, (_, i) => start + (end - start) * i / Math.max(1, count - 1));
}

/** Hold only a recent preceding sample; never pull a future value into a gap. */
export function alignHistory<T extends Timed>(samples: readonly T[], timeline: readonly number[], maxAgeMs: number): Array<T | null> {
  const sorted = [...samples].sort((a, b) => a.timestampMs - b.timestampMs);
  let index = -1;
  return timeline.map((at) => {
    while (index + 1 < sorted.length && sorted[index + 1].timestampMs <= at) index++;
    const sample = sorted[index];
    return sample && at - sample.timestampMs <= maxAgeMs ? sample : null;
  });
}
