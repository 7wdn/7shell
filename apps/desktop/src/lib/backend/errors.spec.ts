import { describe, expect, it, vi } from "vitest";

import { ShellError, internalError, noTransportError, normalizeShellError } from "./errors";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));

import { isDesktopRuntime } from "@/lib/runtime";

const envelope = {
  version: 1,
  code: "SSH-HOSTKEY-MISMATCH",
  messageKey: "errors.ssh.hostKeyMismatch",
  messageParams: { host: "host.example", port: "22" },
  origin: { subsystem: "ssh", adapter: "russh", peer: "host.example:22" },
  outcome: "notSent",
  detail: "presented key differs from the trusted key",
  diagnostics: { keyType: "ssh-ed25519" },
};

describe("normalizeShellError", () => {
  it("keeps an envelope from Rust intact", () => {
    const error = normalizeShellError(envelope, "app_info");

    expect(error).toBeInstanceOf(ShellError);
    expect(error.code).toBe("SSH-HOSTKEY-MISMATCH");
    expect(error.messageKey).toBe("errors.ssh.hostKeyMismatch");
    expect(error.messageParams).toMatchObject({ host: "host.example", port: "22" });
    expect(error.outcome).toBe("notSent");
    expect(error.detail).toContain("presented key");
    expect(error.envelope.origin.peer).toBe("host.example:22");
    expect(error.envelope.diagnostics).toMatchObject({ keyType: "ssh-ed25519" });
  });

  it("fills in what a terse producer leaves out", () => {
    const error = normalizeShellError(
      { version: 1, code: "VAULT-LOCKED", messageKey: "errors.vault.locked", origin: { subsystem: "vault" } },
      "app_info",
    );

    expect(error.code).toBe("VAULT-LOCKED");
    expect(error.messageParams).toEqual({});
    expect(error.outcome).toBe("unknown");
    expect(error.detail).toBeUndefined();
    expect(error.envelope.diagnostics).toBeUndefined();
  });

  it("turns a bare rejection into an envelope instead of losing it", () => {
    const fromString = normalizeShellError("connection reset", "load");
    expect(fromString.code).toBe("APP-INTERNAL");
    expect(fromString.detail).toBe("connection reset");
    expect(fromString.messageParams).toMatchObject({ operation: "load" });

    expect(normalizeShellError(new Error("boom"), "load").detail).toBe("boom");
    expect(normalizeShellError(undefined, "load").code).toBe("APP-INTERNAL");
  });

  it("is idempotent", () => {
    const once = normalizeShellError(envelope, "app_info");
    expect(normalizeShellError(once, "app_info")).toBe(once);
  });
});

describe("ShellError", () => {
  it("answers whether anything was sent", () => {
    expect(normalizeShellError(envelope, "app_info").nothingWasSent).toBe(true);
    expect(noTransportError("app_info").nothingWasSent).toBe(true);
    expect(normalizeShellError({ ...envelope, outcome: "sent" }, "app_info").nothingWasSent).toBe(false);
    expect(internalError("boom", "load").nothingWasSent).toBe(false);
  });

  it("stays an Error and reads sensibly", () => {
    const error = noTransportError("app_info");

    expect(error).toBeInstanceOf(Error);
    expect(error).toBeInstanceOf(ShellError);
    expect(error.message).toContain("APP-NO-TRANSPORT");
    expect(error.detail).toBeUndefined();
  });
});

describe("describeShellError", () => {
  it("renders the localized sentence and keeps the code", async () => {
    const { describeShellError } = await import("@/i18n/errors");
    const described = describeShellError(
      (key, params) => `${key}|${params?.host ?? ""}`,
      normalizeShellError(envelope, "app_info"),
    );

    expect(described.message).toBe("errors.ssh.hostKeyMismatch|host.example");
    expect(described.code).toBe("SSH-HOSTKEY-MISMATCH");
    expect(described.detail).toContain("presented key");
  });

  it("falls back to the backend detail when a key has no translation", async () => {
    const { describeShellError } = await import("@/i18n/errors");
    const described = describeShellError((key) => key, internalError("raw failure text", "load"));

    expect(described.message).toBe("raw failure text");
    expect(described.detail).toBeUndefined();
  });
});

describe("backend facade", () => {
  it("answers a call made outside the desktop shell with a real envelope", async () => {
    vi.mocked(isDesktopRuntime).mockReturnValue(false);

    const { backend } = await import("./api");
    await expect(backend.appInfo()).rejects.toMatchObject({ code: "APP-NO-TRANSPORT" });

    vi.mocked(isDesktopRuntime).mockReturnValue(true);
  });
});
