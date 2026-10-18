import { describe, expect, it } from "vitest";
import type { FanHw } from "@/bindings";
import { defaultCpuFan, isCpuCoolingFan } from "./fanSelection";
import { fmtRpm, maxValid } from "@/utils/format";

const fan = (name: string, rpm: number | null = 900, id = "/lpc/fan/0"): FanHw =>
  ({ name, rpm, id, pwmPercent: null, controllable: false });

describe("fan identity and readings", () => {
  it("never substitutes a spinning GPU, case fan, pump, or device name for a CPU fan", () => {
    const fans = [fan("GPU / GPU Fan", 2500, "/nvidiagpu/fan/0"), fan("Board / System Fan"),
      fan("CPU cooler / Pump", 4000), fan("CPU cooler / Fan #1"), fan("Board / CPU Pump")];
    expect(defaultCpuFan(fans)).toBeNull();
    expect(isCpuCoolingFan(fan("GPU / CPU Fan", 2000, "/nvidiagpu/0/fan/0"))).toBe(false);
  });
  it("selects only a unique CPU fan even when stopped or temporarily unreadable", () => {
    const cpu = fan("Board / CPU Fan", 0);
    expect(defaultCpuFan([fan("Board / System Fan", 1800), cpu])).toBe(cpu);
    expect(defaultCpuFan([fan("Board / CPU Fan", null)])?.rpm).toBeNull();
    expect(defaultCpuFan([cpu, fan("Board / CPU Fan #2")])).toBeNull();
  });
  it("distinguishes a measured stop from missing data and preserves high RPM", () => {
    expect(fmtRpm(0)).toBe("0 RPM");
    expect(fmtRpm(18000)).toBe("18000 RPM");
    for (const invalid of [null, undefined, NaN, Infinity, -1]) expect(fmtRpm(invalid)).toBe("--");
    expect(maxValid([null, 0])).toBe(0);
    expect(maxValid([null, undefined])).toBeNull();
  });
});
