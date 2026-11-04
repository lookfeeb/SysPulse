import { describe, expect, it } from "vitest";
import { sensorAge, visibleHardware } from "./hardwareFreshness";

describe("hardware display freshness", () => {
  const snapshot = { sampleAgeMs: 6000, storageAgeMs: 1000,
    cpu: { packageTempC: 72 }, fans: [{ rpm: 0 }], disks: [{ tempC: 40 }] };
  it("retains a delayed reading without converting real zero or unknown into fabricated readings", () => {
    expect(sensorAge(snapshot.sampleAgeMs)).toBe(6000);
    expect(visibleHardware(snapshot).cpu?.packageTempC).toBe(72);
    expect(visibleHardware(snapshot).fans[0].rpm).toBe(0);
    expect(visibleHardware({ ...snapshot, cpu: null }).cpu).toBeNull();
  });
  it("expires fast and storage collectors independently", () => {
    expect(visibleHardware(snapshot, 25000)).toMatchObject({ cpu: null, fans: [], disks: [{ tempC: 40 }] });
    expect(visibleHardware({ ...snapshot, sampleAgeMs: 1000, storageAgeMs: 31000 }))
      .toMatchObject({ cpu: { packageTempC: 72 }, disks: [] });
  });
  it("never accepts invalid ages as fresh", () => {
    for (const age of [-1, Infinity, NaN]) expect(sensorAge(age)).toBe(Infinity);
  });
});
