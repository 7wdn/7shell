/**
 * A connection test answers through the window's toast: success and failure leave on their own,
 * and only an unseen host key leaves an action behind.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const { toastMock } = vi.hoisted(() => ({ toastMock: vi.fn() }));

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionTest: vi.fn(),
  knownHostTrust: vi.fn(),
}));
vi.mock("@/composables/useToast", () => ({
  useToast: () => ({ toast: toastMock }),
}));

import { knownHostTrust, sessionTest } from "@/lib/backend/tauri";
import { runConnectionTest } from "@/lib/sessions/testConnection";
import type { Session } from "@/types/session";

const sessionTestMock = vi.mocked(sessionTest);
const trustMock = vi.mocked(knownHostTrust);

const session: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: {
    terminalType: "xterm-256color",
    keepaliveSeconds: 30,
    encoding: "utf-8",
    backspace: "del",
    forwardAgent: false,
    record: false,
  },
  updatedAt: 1_700_000_000,
};

const key = {
  host: "example.test",
  port: 22,
  keyType: "ssh-ed25519",
  publicKey: "AAAA",
  fingerprint: "SHA256:presented",
};

describe("running a connection test", () => {
  beforeEach(() => {
    toastMock.mockReset();
    sessionTestMock.mockReset();
    trustMock.mockReset().mockResolvedValue(undefined);
  });

  it("says a successful test in a toast that leaves on its own", async () => {
    sessionTestMock.mockResolvedValue({
      kind: "connected",
      latencyMs: 91,
      authenticated: true,
      fingerprint: "SHA256:example",
    });

    await runConnectionTest(session);

    expect(toastMock).toHaveBeenCalledWith(expect.stringContaining("Signed in, in 91 ms."), "result");
    expect(toastMock.mock.calls[0]?.[0]).toContain("SHA256:example");
  });

  it("offers trusting an unseen key as the toast's one action, then tests again", async () => {
    sessionTestMock
      .mockResolvedValueOnce({ kind: "hostKeyUnknown", key })
      .mockResolvedValueOnce({ kind: "connected", latencyMs: 9, authenticated: true });

    await runConnectionTest(session);

    const [message, tier, action] = toastMock.mock.calls[0] ?? [];
    expect(message).toContain("This host key has never been seen");
    expect(message).toContain("example.test:22");
    expect(message).toContain("SHA256:presented");
    expect(tier).toBe("refusal");
    expect(action?.label).toBe("Trust this key and test again");

    action?.onClick();
    await vi.waitFor(() => expect(trustMock).toHaveBeenCalledWith(key));
    await vi.waitFor(() => expect(sessionTestMock).toHaveBeenCalledTimes(2));
    await vi.waitFor(() =>
      expect(toastMock).toHaveBeenLastCalledWith(expect.stringContaining("Signed in, in 9 ms."), "result"),
    );
  });

  it("a changed key is a failure with both fingerprints and no action", async () => {
    sessionTestMock.mockResolvedValue({
      kind: "hostKeyMismatch",
      key,
      trusted: { ...key, fingerprint: "SHA256:trusted", addedAt: 1 },
    });

    await runConnectionTest(session);

    const [message, tier, action] = toastMock.mock.calls[0] ?? [];
    expect(message).toContain("The host key changed");
    expect(message).toContain("Trusted: SHA256:trusted");
    expect(message).toContain("Presented: SHA256:presented");
    expect(tier).toBe("failure");
    expect(action).toBeUndefined();
  });

  it("a refused test keeps its code on the toast", async () => {
    sessionTestMock.mockRejectedValue({
      version: 1,
      code: "SSH-AUTH-FAILED",
      messageKey: "errors.ssh.authFailed",
      messageParams: { host: "example.test", port: "22", method: "password" },
      origin: { subsystem: "ssh", peer: "example.test:22" },
      outcome: "sent",
    });

    await runConnectionTest(session);

    expect(toastMock).toHaveBeenCalledWith(expect.stringContaining("SSH-AUTH-FAILED"), "failure");
  });
});
