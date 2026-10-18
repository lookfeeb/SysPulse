import type { FanHw } from "@/bindings";

export function isCpuCoolingFan(fan: FanHw): boolean {
  if (/(?:nvidia|ati|intel)?gpu/i.test(fan.id)) return false;
  const label = fan.name.split(" / ").at(-1)?.trim().replace(/_/g, " ") ?? "";
  return /^cpu\s*(?:optional\s*)?fan(?:\s*#?\d+)?$/i.test(label);
}

/** A missing or ambiguous CPU label requires an explicit choice. RPM is not identity. */
export function defaultCpuFan(fans: FanHw[]): FanHw | null {
  const matches = fans.filter(isCpuCoolingFan);
  return matches.length === 1 ? matches[0] : null;
}
