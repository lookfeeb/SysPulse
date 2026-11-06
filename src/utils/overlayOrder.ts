export interface OrderSlot {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Hit-test fixed layout slots, never the animated cards or the floating preview. */
export function findOrderSlot(slots: OrderSlot[], x: number, y: number, current: number): number {
  if (!slots.length) return current;
  const rows = [...new Set(slots.map((slot) => slot.top))];
  const rowDistance = (top: number) => {
    const slot = slots.find((candidate) => candidate.top === top)!;
    return Math.abs(y - top - slot.height / 2);
  };
  let row = rows.reduce((best, top) => rowDistance(top) < rowDistance(best) ? top : best);
  const previous = slots[current];
  // A small dead zone around boundaries prevents jitter from hand tremor.
  if (previous && rowDistance(previous.top) - rowDistance(row) < 12) row = previous.top;
  const candidates = slots.map((slot, index) => ({ slot, index })).filter(({ slot }) => slot.top === row);
  const distance = (slot: OrderSlot) => Math.abs(x - slot.left - slot.width / 2);
  const nearest = candidates.reduce((best, candidate) => distance(candidate.slot) < distance(best.slot) ? candidate : best);
  if (previous?.top === row && distance(previous) - distance(nearest.slot) < 12) return current;
  return nearest.index;
}

export function moveOrderItem<T>(items: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || to < 0 || from >= items.length || to >= items.length) return items;
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
}
