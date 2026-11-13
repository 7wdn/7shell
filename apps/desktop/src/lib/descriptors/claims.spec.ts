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

  it("still leaves the desktop channels this build does not draw unclaimed", () => {
    // The system client owns the desktop; this process does not carry its frames, input, resize or
    // clipboard channel. Only systemClient is claimed.
    for (const capability of ["desktopView", "desktopInput", "desktopResize", "desktopClipboard"] as const) {
      expect(isCapabilityClaimed(capability), `${capability} stays unclaimed`).toBe(false);
    }
    expect(isCapabilityClaimed("systemClient")).toBe(true);
    expect(isCapabilityClaimed("containerFiles")).toBe(true);
    expect(isCapabilityClaimed("kubernetesTunnel")).toBe(true);
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
