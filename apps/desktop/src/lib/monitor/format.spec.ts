import { describe, expect, it } from "vitest";

import { perSecond, uptime } from "@/lib/monitor/format";

describe("uptime", () => {
  it("splits seconds into the parts a locale arranges", () => {
    expect(uptime(0)).toEqual({ days: 0, hours: 0, minutes: 0 });
    expect(uptime(3661)).toEqual({ days: 0, hours: 1, minutes: 1 });
    expect(uptime(90_061)).toEqual({ days: 1, hours: 1, minutes: 1 });
  });

  it("treats a reading that cannot be a duration as no time at all", () => {
    // A clock that went backwards, or a host that reported garbage: drawn as a zero, never as NaN.
    expect(uptime(-5)).toEqual({ days: 0, hours: 0, minutes: 0 });
    expect(uptime(Number.NaN)).toEqual({ days: 0, hours: 0, minutes: 0 });
  });
});

describe("perSecond", () => {
  it("is the same scale as every other size on screen, with a clock attached", () => {
    expect(perSecond(0)).toBe("0 B/s");
    expect(perSecond(1000)).toBe("1.0 KB/s");
    expect(perSecond(1_500_000)).toBe("1.5 MB/s");
  });
});
