/**
 * The interface's half of the error contract.
 *
 * Rust sends `sevenshell_error::ShellError` as JSON; whatever arrives is normalized into
 * a {@link ShellError} here, so no caller anywhere in the interface has to guess what a
 * rejection looks like. The envelope shape is pinned on the Rust side by
 * `crates/sevenshell-error/tests/wire_format.rs`.
 */

import { APP_INTERNAL, APP_NO_TRANSPORT, type ErrorCode, messageKeyFor } from "./errorCodes";

const ENVELOPE_VERSION = 1;

export type OperationOutcome = "unknown" | "notSent" | "sent" | "rejected";

export type Subsystem = "vault" | "ssh" | "sftp" | "fs" | "store" | "policy" | "net" | "app" | "plugin";

export interface ErrorOrigin {
  readonly subsystem: Subsystem;
  readonly adapter?: string;
  readonly peer?: string;
}

export interface ErrorEnvelope {
  readonly version: number;
  /** A code from the catalog, or a newer one this build does not know yet. */
  readonly code: string;
  readonly messageKey: string;
  readonly messageParams: Readonly<Record<string, string>>;
  readonly origin: ErrorOrigin;
  readonly outcome: OperationOutcome;
  readonly detail?: string;
  readonly diagnostics?: Readonly<Record<string, string>>;
}

export class ShellError extends Error {
  readonly envelope: ErrorEnvelope;

  constructor(envelope: ErrorEnvelope) {
    super(`${envelope.code} (${envelope.messageKey})`);
    this.name = "ShellError";
    this.envelope = envelope;
    // Keeps `instanceof` reliable when the class is compiled down to a function.
    Object.setPrototypeOf(this, ShellError.prototype);
  }

  get code(): string {
    return this.envelope.code;
  }

  get messageKey(): string {
    return this.envelope.messageKey;
  }

  get messageParams(): Readonly<Record<string, string>> {
    return this.envelope.messageParams;
  }

  get outcome(): OperationOutcome {
    return this.envelope.outcome;
  }

  get detail(): string | undefined {
    return this.envelope.detail;
  }

  /** True when the failure happened before anything sensitive was transmitted. */
  get nothingWasSent(): boolean {
    return this.envelope.outcome === "notSent" || this.envelope.outcome === "rejected";
  }
}

/** Builds the envelope the interface raises when it cannot reach a backend at all. */
export function noTransportError(operation: string): ShellError {
  return new ShellError({
    version: ENVELOPE_VERSION,
    code: APP_NO_TRANSPORT,
    messageKey: messageKeyFor(APP_NO_TRANSPORT),
    messageParams: { operation },
    origin: { subsystem: "app" },
    outcome: "rejected",
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function looksLikeEnvelope(value: unknown): value is Record<string, unknown> {
  return (
    isRecord(value) &&
    typeof value.code === "string" &&
    typeof value.messageKey === "string" &&
    isRecord(value.origin) &&
    typeof value.origin.subsystem === "string"
  );
}

function stringRecord(value: unknown): Record<string, string> {
  if (!isRecord(value)) {
    return {};
  }
  const out: Record<string, string> = {};
  for (const [key, entry] of Object.entries(value)) {
    if (typeof entry === "string") {
      out[key] = entry;
    }
  }
  return out;
}

/** Turns an unexpected failure into an envelope, keeping the original text as detail. */
export function internalError(detail: string, operation: string): ShellError {
  return new ShellError({
    version: ENVELOPE_VERSION,
    code: APP_INTERNAL,
    messageKey: messageKeyFor(APP_INTERNAL),
    messageParams: { operation },
    origin: { subsystem: "app" },
    outcome: "unknown",
    detail,
  });
}

function describeUnknown(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  if (value instanceof Error) {
    return value.message;
  }
  try {
    return JSON.stringify(value) ?? String(value);
  } catch {
    return String(value);
  }
}

/**
 * Normalizes anything a backend call can reject with into a {@link ShellError}.
 *
 * Missing optional fields are filled in rather than treated as errors: a producer that
 * omits `messageParams` or `diagnostics` is still telling us something useful.
 */
export function normalizeShellError(raw: unknown, operation: string): ShellError {
  if (raw instanceof ShellError) {
    return raw;
  }
  if (!looksLikeEnvelope(raw)) {
    return internalError(describeUnknown(raw), operation);
  }

  const detail = typeof raw.detail === "string" ? raw.detail : undefined;
  const diagnostics = stringRecord(raw.diagnostics);
  const origin = isRecord(raw.origin) ? raw.origin : {};

  return new ShellError({
    version: typeof raw.version === "number" ? raw.version : ENVELOPE_VERSION,
    code: raw.code as string,
    messageKey: raw.messageKey as string,
    messageParams: stringRecord(raw.messageParams),
    origin: {
      subsystem: origin.subsystem as Subsystem,
      adapter: typeof origin.adapter === "string" ? origin.adapter : undefined,
      peer: typeof origin.peer === "string" ? origin.peer : undefined,
    },
    outcome: (raw.outcome as OperationOutcome | undefined) ?? "unknown",
    detail,
    diagnostics: Object.keys(diagnostics).length > 0 ? diagnostics : undefined,
  });
}

/**
 * Coerces anything a caller caught into an envelope, keeping an existing one intact.
 *
 * Stores and components use this instead of testing for the class: what matters is that every
 * failure the interface shows carries a code and a message key.
 */
export function asShellError(error: unknown, operation: string): ShellError {
  return error instanceof ShellError ? error : internalError(describeUnknown(error), operation);
}

export type { ErrorCode };
