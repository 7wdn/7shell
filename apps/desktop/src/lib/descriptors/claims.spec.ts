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

  it("counts agent forwarding once the far side used this machine's agent", () => {
    // It stayed unclaimed while the live test could not reach an agent. The test now starts the
    // agent side itself, and the capability moved with the proof rather than with the code.
    expect(isCapabilityClaimed("agentForwarding")).toBe(true);
  });

  it("describes remote desktop only through the system client", () => {
    // The embedded engine and its view/input/resize/clipboard capabilities were removed; mstsc is
    // the whole remote-desktop promise, and the container/cluster work is claimed beside it.
    expect(isCapabilityClaimed("systemClient")).toBe(true);
    expect(isCapabilityClaimed("containerFiles")).toBe(true);
    expect(isCapabilityClaimed("kubernetesTunnel")).toBe(true);
  });

  it("leaves nothing in the vocabulary unclaimed", () => {
    const claimed = claimedCapabilities().map((capability) => capability.key);
    const unclaimed = unclaimedCapabilities().map((capability) => capability.key);

    expect(claimed.length).toBeGreaterThan(0);
    expect(unclaimed).toEqual([]);
    expect(new Set([...claimed, ...unclaimed]).size).toBe(claimed.length + unclaimed.length);
  });
});
