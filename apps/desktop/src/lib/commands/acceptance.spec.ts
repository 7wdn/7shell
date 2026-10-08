import { beforeEach, describe, expect, it } from "vitest";

import { acceptanceBoost, forgetAccepted, recordAccepted } from "@/lib/commands/acceptance";

beforeEach(() => {
  forgetAccepted();
});

describe("what somebody has taken before", () => {
  it("is worth nothing until a row has been taken", () => {
    expect(acceptanceBoost("uptime")).toBe(0);
  });

  it("is worth more each time, up to a point", () => {
    recordAccepted("uptime");
    const once = acceptanceBoost("uptime");
    expect(once).toBeGreaterThan(0);

    recordAccepted("uptime");
    expect(acceptanceBoost("uptime")).toBeGreaterThan(once);

    for (let index = 0; index < 50; index += 1) {
      recordAccepted("uptime");
    }
    // Capped well below the distance between two match qualities: a hint is not allowed to outrank a
    // better match, or the list would start hiding what somebody typed.
    expect(acceptanceBoost("uptime")).toBeLessThan(1000);
  });

  it("keeps the rows somebody has taken apart from each other", () => {
    recordAccepted("uptime");

    expect(acceptanceBoost("uptime")).toBeGreaterThan(0);
    expect(acceptanceBoost("systemctl restart")).toBe(0);
  });

  it("remembers nothing for a row that inserts nothing", () => {
    recordAccepted("");

    expect(acceptanceBoost("")).toBe(0);
  });

  it("forgets the ones nobody reaches for once it is full", () => {
    for (let index = 0; index < 300; index += 1) {
      recordAccepted(`command-${index}`);
    }

    expect(acceptanceBoost("command-0"), "the oldest fell off").toBe(0);
    expect(acceptanceBoost("command-299")).toBeGreaterThan(0);
  });
});
