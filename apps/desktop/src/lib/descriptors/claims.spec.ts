import { describe, expect, it } from "vitest";

import { claimedCapabilities, isCapabilityClaimed, unclaimedCapabilities } from "@/lib/descriptors/claims";

describe("what the descriptors claim", () => {
  it("counts a capability an authentication method claims as claimed", () => {
    // The three the home page used to report as unclaimed while the build had them: passwords are
    // stored in the vault, keyboard-interactive is answered, and a key file can be imported.
    for (const capability of ["secret", "interactive", "keyMaterial"] as const) {
      expect(isCapabilityClaimed(capability), `${capability} is claimed by an auth method`).toBe(true);
    }
  });

  it("leaves the ones nothing built unclaimed", () => {
    for (const capability of ["dynamicForwarding", "agentForwarding", "recording"] as const) {
      expect(isCapabilityClaimed(capability), `${capability} is not implemented`).toBe(false);
    }
  });

  it("splits the vocabulary in two, without losing or repeating anything", () => {
    const claimed = claimedCapabilities().map((capability) => capability.key);
    const unclaimed = unclaimedCapabilities().map((capability) => capability.key);

    expect(claimed.length).toBeGreaterThan(0);
    expect(unclaimed.length).toBeGreaterThan(0);
    expect(new Set([...claimed, ...unclaimed]).size).toBe(claimed.length + unclaimed.length);
    for (const capability of claimed) {
      expect(unclaimed.includes(capability)).toBe(false);
    }
  });
});
