import { describe, expect, it } from "vitest";
import { findOrderSlot, moveOrderItem } from "./overlayOrder";
import { toConfigItems } from "./overlayItems";

const row = Array.from({ length: 5 }, (_, index) => ({ left: index * 160, top: 0, width: 150, height: 40 }));

describe("overlay order drop slots", () => {
  it("keeps the same destination after reordering and tolerates boundary jitter", () => {
    expect(findOrderSlot(row, 163, 20, 0)).toBe(1);
    expect(findOrderSlot(row, 163, 20, 1)).toBe(1);
    expect(findOrderSlot(row, 154, 20, 1)).toBe(1);
    expect(findOrderSlot(row, 147, 20, 1)).toBe(0);
  });

  it("supports fast movement past several cards without swapping their identities", () => {
    const index = findOrderSlot(row, 715, 20, 0);
    expect(index).toBe(4);
    expect(moveOrderItem(["net", "cpu", "temp", "mem", "disk"], 0, index)).toEqual(["cpu", "temp", "mem", "disk", "net"]);
  });

  it("selects the closest column on the intended row, including a partially filled last row", () => {
    const wrapped = row.map((slot, i) => ({ ...slot, left: i % 3 * 160, top: Math.floor(i / 3) * 50 }));
    expect(findOrderSlot(wrapped, 395, 70, 2)).toBe(4);
    expect(findOrderSlot(wrapped, 75, 70, 2)).toBe(3);
    expect(findOrderSlot(wrapped, 395, 20, 4)).toBe(2);
    expect(findOrderSlot(wrapped, 75, 46, 0)).toBe(0);
  });

  it("does not mutate source order or duplicate paired network/disk items", () => {
    const items = ["net-down", "cpu", "disk-read", "mem"];
    const moved = moveOrderItem(items, 2, 0);
    expect(items).toEqual(["net-down", "cpu", "disk-read", "mem"]);
    expect(toConfigItems(moved)).toEqual(["disk-read", "disk-write", "net-down", "net-up", "cpu", "mem"]);
    expect(moveOrderItem(items, 0, -1)).toBe(items);
    expect(moveOrderItem(items, 0, items.length)).toBe(items);
  });
});
