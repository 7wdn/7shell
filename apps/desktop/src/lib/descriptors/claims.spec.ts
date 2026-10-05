import { describe, expect, it } from "vitest";

import { claimedCapabilities, isCapabilityClaimed, unclaimedCapabilities } from "@/lib/descriptors/claims";

describe("what the descriptors claim", () => {
  it("counts a capability an authentication method claims as claimed", () => {
    // The three the home page used to report as unclaimed while the build had them: passwords are
    // stored in the vault, keyboard-interactive is answered, and a key file can be imported.
    for (const capability of ["secret", "interactive", "keyMaterial", "platformAgent"] as const) {
      expect(isCapabilityClaimed(capability), `${capability} is claimed`).toBe(true);
    }
  });

  it("counts a dynamic forward as claimed, now that one can be brought up", () => {
    // It was unclaimed here until SOCKS5 landed, which is the guard doing its job: a capability
    // that starts being implemented has to be moved deliberately.
    expect(isCapabilityClaimed("dynamicForwarding")).toBe(true);
  });

  it("counts recording as claimed, now that a session can be written down", () => {
    expect(isCapabilityClaimed("recording")).toBe(true);
  });

  it("leaves the one nothing has been shown to do unclaimed", () => {
    // Agent forwarding is implemented and *still* unclaimed, which is the rule rather than an
    // oversight: what it promises is the far side using this machine's agent, and that has not been
    // demonstrated anywhere this was built. The descriptor says what would change that.
    expect(isCapabilityClaimed("agentForwarding")).toBe(false);
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
